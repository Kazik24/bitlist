use super::BitList;
use crate::inline::InlineBitList;
use crate::iter::{BitsIter, WordBits};
use crate::ops::AllocateError;
use crate::wrapper::NonZeroPtr;
use std::alloc::{Layout, LayoutError, alloc, alloc_zeroed, dealloc, handle_alloc_error, realloc};
use std::collections::TryReserveError;
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, MaybeUninit, size_of, transmute_copy};
use std::ops::{Deref, DerefMut, Range};
use std::ptr::NonNull;
use std::slice::{from_raw_parts, from_raw_parts_mut};

#[cfg_attr(feature = "align_16", repr(C, packed(2)))]
#[cfg_attr(all(feature = "align_32", not(feature = "align_16")), repr(C, packed(4)))]
#[cfg_attr(all(not(feature = "align_32"), not(feature = "align_16")), repr(transparent))]
pub struct HeapBitList {
    ptr: NonZeroPtr,
}
unsafe impl Send for HeapBitList {}
unsafe impl Sync for HeapBitList {}

//allocation format
// ptr ------+
//           |   idx: 0             1              2                  (2 + cap_in_words)
//           +------> [ len_in_bits | cap_in_words | data ...     ... ]
//
// to get capacity in bits: cap_in_words * usize::BITS

/// Memory allocation and deallocation
impl HeapBitList {
    /// Word size in bits
    pub const WORD_SIZE: usize = usize::BITS as _;
    /// Header size in words, it consists of length and capacity field.
    pub const HEADER: usize = 2;
    /// Word size in bytes
    pub const WORD_BYTES: usize = size_of::<usize>();

    fn layout(count: usize) -> (Layout, usize) {
        //guard against bad usage in BitList struct
        debug_assert!(count > InlineBitList::DATA_BITS as _);
        let cap_words = words_for(count);
        //never overflows
        let words = cap_words + Self::HEADER;
        //SAFETY: this can never fail, cause words is at at most `count / 16` (but most likely `count / 32`)
        // so resulting array will never exceed isize::MAX
        let lay = Layout::array::<usize>(words);
        unsafe {
            debug_assert!(lay.is_ok());
            let lay = lay.unwrap_unchecked();
            (lay, cap_words)
        }
    }

    unsafe fn handle_alloc(ptr: Option<NonNull<u8>>, layout: Layout, capacity: usize) -> Result<Self, Layout> {
        let Some(ptr) = ptr else {
            return Err(layout);
        };
        let mut value = Self { ptr: ptr.cast::<usize>() };
        unsafe { value.set_cap_len(capacity, 0) };
        Ok(value)
    }

    fn panic_on_alloc_err(result: Result<Self, Layout>) -> Self {
        match result {
            Ok(v) => v,
            Err(lay) => handle_alloc_error(lay),
        }
    }

    unsafe fn alloc_zeroed(capacity: usize) -> Result<Self, Layout> {
        let (layout, cap_words) = Self::layout(capacity);
        unsafe {
            let ptr = NonNull::new(alloc_zeroed(layout));
            Self::handle_alloc(ptr, layout, cap_words)
        }
    }
    unsafe fn alloc_uninit(capacity: usize) -> Result<Self, Layout> {
        let (layout, cap_words) = Self::layout(capacity);
        unsafe {
            let ptr = NonNull::new(alloc(layout));
            Self::handle_alloc(ptr, layout, cap_words)
        }
    }

    #[must_use]
    pub unsafe fn memory_mut_vec(&mut self) -> BorrowMutVec<'_> {
        let cap = self.allocation_words();
        let len = words_for(self.len()) + Self::HEADER;
        debug_assert!(len <= cap);
        let vec = ManuallyDrop::new(unsafe { Vec::from_raw_parts(self.ptr.as_ptr(), len, cap) });
        BorrowMutVec { parent: self, vec }
    }

    #[must_use]
    pub fn memory_const_vec(&self) -> BorrowVec<'_> {
        let cap = self.allocation_words();
        let len = words_for(self.len()) + Self::HEADER;
        debug_assert!(len <= cap);
        unsafe {
            let vec = ManuallyDrop::new(Vec::from_raw_parts(self.ptr.as_ptr(), cap, len));
            BorrowVec { vec, _phantom: PhantomData }
        }
    }

    /// Return memory view of this list as vector, length of vector is number of words currently initialized
    /// (always >= 3).
    pub fn into_vec_memory(self) -> Vec<usize> {
        let cap = self.allocation_words();
        let len = words_for(self.len()) + Self::HEADER;
        debug_assert!(len <= cap);
        unsafe { Vec::from_raw_parts(self.ptr.as_ptr(), len, cap) }
    }

    /// #Safety
    /// The vec must have element at index 1 set to valid capacity in words (minus header size), and element at index 0
    /// is valid length in bits, also trailing bits in last world must be set to zero (if length is not multiple of word size).
    pub unsafe fn from_vec_memory(mut vec: Vec<usize>, update_capacity: bool) -> Self {
        Self::assert_update_memory_vec(&mut vec, update_capacity, true);
        //construct a HeapBitList
        let ptr = ManuallyDrop::new(vec).as_mut_ptr();
        Self { ptr: unsafe { NonNull::new_unchecked(ptr) } }
    }

    fn assert_update_memory_vec(vec: &mut Vec<usize>, update_capacity: bool, check_last_word: bool) {
        //the smallest heap allocated list has only one word + header
        debug_assert!(vec.capacity() >= 1 + Self::HEADER);

        if update_capacity {
            //at least header is initialized
            assert!(vec.len() >= Self::HEADER);

            let cap = vec.capacity();
            unsafe {
                vec.as_mut_ptr().add(1).write(cap - Self::HEADER);
            }
        }
        //check that we uphold a capacity invariant
        debug_assert!(vec[1] == vec.capacity() - Self::HEADER);
        //length is lower than capacity converted to bits
        debug_assert!(vec[1] * Self::WORD_SIZE >= vec[0]);
        // check that last word bits are zero if lenght is not aligned to word boundary
        if cfg!(debug_assertions) && check_last_word {
            let idx = words_for(vec[0]).saturating_sub(1) + Self::HEADER;
            assert!(vec[idx] & !last_word_mask(vec[0]) == 0, "last word: {:064b}, len: {}", vec[idx], vec[0]);
        }
    }

    unsafe fn dealloc(&mut self) {
        let lay = self.current_layout();
        unsafe { dealloc(self.ptr.as_ptr() as _, lay) };
    }
}

pub struct BorrowMutVec<'a> {
    parent: &'a mut HeapBitList,
    vec: ManuallyDrop<Vec<usize>>,
}
pub struct BorrowVec<'a> {
    vec: ManuallyDrop<Vec<usize>>,
    _phantom: PhantomData<&'a HeapBitList>,
}
impl Deref for BorrowVec<'_> {
    type Target = Vec<usize>;
    #[must_use]
    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}
impl Deref for BorrowMutVec<'_> {
    type Target = Vec<usize>;
    #[must_use]
    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}
impl DerefMut for BorrowMutVec<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.vec
    }
}
impl BorrowMutVec<'_> {
    pub fn set(mut self, update_capacity: bool) {
        HeapBitList::assert_update_memory_vec(&mut self.vec, update_capacity, true);
        drop(self);
    }
}
impl Drop for BorrowMutVec<'_> {
    fn drop(&mut self) {
        HeapBitList::assert_update_memory_vec(&mut self.vec, false, true);
        unsafe {
            self.parent.ptr = NonNull::new_unchecked(self.vec.as_mut_ptr());
        }
    }
}

impl HeapBitList {
    pub fn zeros(count: usize) -> Self {
        debug_assert_ne!(count, 0);
        unsafe {
            let mut list = Self::panic_on_alloc_err(Self::alloc_zeroed(count));
            list.set_len(count);
            list
        }
    }
    pub fn with_capacity(capacity: usize) -> Self {
        unsafe { Self::panic_on_alloc_err(Self::alloc_uninit(capacity)) }
    }
    pub fn try_with_capacity(capacity: usize) -> Result<Self, Layout> {
        unsafe { Self::alloc_uninit(capacity) }
    }
    pub fn ones(count: usize) -> Self {
        debug_assert_ne!(count, 0);
        let mut list = Self::with_capacity(count);
        unsafe {
            list.data_ptr_mut().write_bytes(0xff, words_for(count));
            list.set_len(count);
            let last = list.init_data_mut().last_mut().unwrap();
            *last &= last_word_mask(count);
        }
        list
    }

    /// Set length in bits, performs checks in debug
    pub unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity(), "len:{len}, capacity:{}", self.capacity()); //guard against undefined state
        unsafe { self.ptr.as_ptr().write(len) };
    }
    /// Set capacity in words (which is in bits: cap * usize::BITS), performs checks in debug
    unsafe fn set_cap(&mut self, cap: usize) {
        debug_assert!(cap * (usize::BITS as usize) >= self.len()); //guard against undefined state
        unsafe { self.ptr.as_ptr().add(1).write(cap) };
    }
    /// Set length in bits and capacity in words, perform checks in debug
    unsafe fn set_cap_len(&mut self, cap: usize, len: usize) {
        debug_assert!(cap * (usize::BITS as usize) >= len); //guard against undefined state
        unsafe {
            self.ptr.as_ptr().write(len);
            self.ptr.as_ptr().add(1).write(cap);
        }
    }
    pub const fn len(&self) -> usize {
        unsafe { self.ptr.as_ptr().read() }
    }
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub const fn capacity(&self) -> usize {
        self.capacity_words() * Self::WORD_SIZE
    }
    pub const fn capacity_words(&self) -> usize {
        unsafe { self.ptr.as_ptr().add(1).read() }
    }
    pub const fn allocation_words(&self) -> usize {
        self.capacity_words() + Self::HEADER
    }
    fn current_layout(&self) -> Layout {
        let words = self.allocation_words();
        //SAFETY: this capacity was allocated with valid layout, so creating layout from
        //capacity is also valid
        unsafe { Layout::array::<usize>(words).unwrap_unchecked() }
    }
    #[inline]
    pub const fn init_data(&self) -> &[usize] {
        let words = words_for(self.len());
        unsafe {
            let ptr = self.data_ptr();
            from_raw_parts(ptr, words)
        }
    }
    /// to check if data is empty, just check if last WordBits is empty
    pub const fn init_data_split_last(&self) -> (&[usize], WordBits) {
        let Some((&last, words)) = self.init_data().split_last() else {
            return (&[], WordBits::empty());
        };
        //length here is never 0, that is why we can assume that 0 means max bits
        let mut rest = self.len() % HeapBitList::WORD_SIZE;
        if rest == 0 {
            rest = HeapBitList::WORD_SIZE;
        }
        (words, WordBits::new_unchecked(last, rest as _))
    }
    #[inline]
    pub fn init_data_mut(&mut self) -> &mut [usize] {
        let words = words_for(self.len());
        unsafe {
            let ptr = self.data_ptr_mut();
            from_raw_parts_mut(ptr, words)
        }
    }
    pub const fn last_bit(&self) -> Option<bool> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let idx = len - 1;
        //SAFETY: we just checked that index is in bound, so given word index will be valid for read
        let word = unsafe { self.data_ptr().add(word_index(idx)).read() };
        Some(word & (1 << bit_in_word_index(idx)) != 0)
    }
    pub const fn first_bit(&self) -> Option<bool> {
        if let Some(first) = self.first_word_init() {
            return Some(first & 1 != 0);
        }
        None
    }
    pub const fn get_bit(&self, index: usize) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        //SAFETY: we just checked that index is in bound, so given word index will be valid for read
        let word = unsafe { self.data_ptr().add(word_index(index)).read() };
        Some(word & (1 << bit_in_word_index(index)) != 0)
    }
    pub fn set_bit(&mut self, index: usize, value: bool) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        //SAFETY: we just checked that index is in bound, so given word index will be valid for read/write
        //and calculated mask won't set any bits outside length
        unsafe {
            let word = &mut *self.data_ptr_mut().add(word_index(index));
            Some(set_bit_value(word, index, value))
        }
    }
    pub const fn data_ptr(&self) -> *const usize {
        unsafe { self.ptr.as_ptr().add(2) }
    }
    pub fn data_ptr_mut(&mut self) -> *mut usize {
        unsafe { self.ptr.as_ptr().add(2) }
    }

    pub fn uninit_data_mut(&mut self) -> &mut [MaybeUninit<usize>] {
        let words = self.capacity_words();
        unsafe {
            let ptr = self.data_ptr_mut();
            from_raw_parts_mut(ptr.cast::<MaybeUninit<usize>>(), words)
        }
    }
    /// get words that all should be initialized when given length is set
    /// (panics if length is greater than capacity)
    pub fn words_for_init(&mut self, new_len: usize) -> &mut [MaybeUninit<usize>] {
        if new_len > self.capacity() {
            panic!("Length is greater than capacity, new_len = {new_len}, capacity = {}", self.capacity());
        }
        let len = words_for(new_len);
        unsafe {
            let ptr = self.data_ptr_mut();
            from_raw_parts_mut(ptr.cast::<MaybeUninit<usize>>(), len)
        }
    }
    pub fn init_from(&mut self, iter: impl Iterator<Item = usize>) {
        for (dst, src) in self.uninit_data_mut().iter_mut().zip(iter) {
            *dst = MaybeUninit::new(src);
        }
    }
    #[inline]
    pub const fn first_word(&self) -> MaybeUninit<usize> {
        // SAFETY: list always contains at least one usize word (but it might be uninitialized)
        unsafe { self.data_ptr().cast::<MaybeUninit<usize>>().read() }
    }
    #[inline]
    pub const fn first_word_init(&self) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        // SAFETY: we just asserted that list is not empty, so that first word must be initialized
        unsafe { Some(self.first_word().assume_init()) }
    }
    #[inline]
    pub const fn to_le_bytes<const N: usize>(&self) -> [u8; N] {
        //todo this code results in poor assembly, try to optimize it
        let mut array = [0u8; N];
        let words = self.init_data();
        let init_count = words.len() * Self::WORD_BYTES;
        let len = if N < init_count { N } else { init_count };
        //SAFETY: we just picked length that would not overflow either array
        let mut i = 0;
        while i < len {
            let word = words[i.wrapping_shr((Self::WORD_BYTES - 1).count_ones())].to_le_bytes();
            array[i] = word[i & (Self::WORD_BYTES - 1)];
            i += 1;
        }
        array
    }
    #[inline]
    pub unsafe fn set_first_word(&mut self, value: usize) {
        //SAFETY: first word is always valid for writes, caller must ensure that no set bits
        //are outside current length
        unsafe { self.data_ptr_mut().write(value) };
    }
    #[inline]
    pub fn set_from_inline(&mut self, inline: InlineBitList) {
        //SAFETY: inline length is always lower that the lowest capacity of heap list
        //data written also has mask applied already so no set bits are outside of length
        unsafe {
            self.set_first_word(inline.data());
            self.set_len(inline.len())
        }
    }

    pub fn try_ensure_capacity(&mut self, new_cap: usize) -> Result<(), AllocateError> {
        if new_cap <= self.capacity() {
            return Ok(());
        }
        unsafe {
            let additional = words_for(new_cap) - words_for(self.len());
            let mut vec = self.memory_mut_vec();
            vec.try_reserve(additional).map_err(AllocateError::Internal)?;
            vec.set(true);
            Ok(())
        }
    }
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), AllocateError> {
        let Some(new_cap) = self.len().checked_add(additional) else {
            return Err(AllocateError::LengthOverflow { current: self.len(), additional });
        };
        self.try_ensure_capacity(new_cap)
    }

    pub fn get_range(&self, range: Range<usize>) -> Option<BitList> {
        if is_invalid_range(&range, self.len()) {
            return None;
        }
        //todo more performant impl
        Some(BitList::from_bits(BitsIter::from_heap(self, range)))
    }

    pub fn set_at(&mut self, index: usize, value: &BitList) -> bool {
        if is_invalid_index(self.len(), index, value.len()) {
            return false;
        }
        //todo more performant impl
        for (i, bit) in value.iter().enumerate() {
            self.set_bit(i + index, bit).unwrap();
        }
        true
    }
}

impl Clone for HeapBitList {
    fn clone(&self) -> Self {
        //handle cloning memory by std vector's code
        let vec = self.memory_const_vec();
        let vec = (*vec).clone();
        // SAFETY: vec from memory_const_vec is always valid to construct list
        unsafe { Self::from_vec_memory(vec, true) }
    }
}

impl PartialEq for HeapBitList {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.init_data() == other.init_data()
    }
}
impl Eq for HeapBitList {}

impl Drop for HeapBitList {
    fn drop(&mut self) {
        unsafe {
            self.dealloc();
        }
    }
}

/// word_index_inclusive
#[inline]
pub(crate) const fn words_for(bits: usize) -> usize {
    word_index(bits) + if bit_in_word_index(bits) != 0 { 1 } else { 0 }
}
#[inline]
pub(crate) const fn last_word_mask(len: usize) -> usize {
    let shift = bit_in_word_index(len);
    if shift == 0 && len != 0 {
        return usize::MAX;
    }
    (1usize.wrapping_shl(shift as _)) - 1
}
#[inline]
pub(crate) const fn word_index(bit_index: usize) -> usize {
    const SHIFT: u32 = (HeapBitList::WORD_SIZE - 1).count_ones();
    bit_index.wrapping_shr(SHIFT)
}
#[inline]
pub(crate) const fn bit_in_word_index(bit_index: usize) -> usize {
    bit_index & (HeapBitList::WORD_SIZE - 1)
}
#[inline]
pub(crate) const fn is_invalid_range(range: &Range<usize>, len: usize) -> bool {
    if range.start > range.end || range.end > len {
        return true;
    }
    false
}
#[inline]
pub(crate) const fn bcd_size_for_bits(bit_len: usize) -> usize {
    if bit_len <= 3 {
        return 4;
    }
    bit_len + ((bit_len - 4) / 3) + 1
}
/// Set bit in word and return previous value, index is wrapping so bit index is `index % WORD_SIZE`
#[inline]
pub(crate) const fn set_bit_value(word: &mut usize, index: usize, value: bool) -> bool {
    let mask = 1 << bit_in_word_index(index);
    let prev = *word & mask != 0;
    if value {
        *word |= mask;
    } else {
        *word &= !mask;
    }
    prev
}

#[inline]
pub(crate) const fn is_invalid_index(self_len: usize, index: usize, other_len: usize) -> bool {
    match index.checked_add(other_len) {
        Some(tot) => tot > self_len,
        None => true,
    }
}

mod tests {
    use super::*;

    #[test]
    fn test_layout_for_bits() {
        let v = crate::wrapper::BitList::MAX_INLINE_BITS + 1;
        for i in (v..(v + 512)).chain([usize::MAX - 3, usize::MAX - 2, usize::MAX - 1, usize::MAX]) {
            HeapBitList::layout(i); //test if this panics in debug mode
        }
    }
    #[test]
    fn test_last_word_mask() {
        for i in 0..(usize::BITS * 10) {
            let mask = last_word_mask(i as _);
            let mut expect = i % usize::BITS;
            if i != 0 && expect == 0 {
                expect = usize::BITS;
            }
            //println!("{i:>3} {mask:064b}");
            assert_eq!(mask.count_ones(), expect);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_bit_word_index() {
        assert_eq!(bit_in_word_index(0), 0);
        assert_eq!(bit_in_word_index(63), 63);
        assert_eq!(bit_in_word_index(64), 0);
        assert_eq!(word_index(0), 0);
        assert_eq!(word_index(63), 0);
        assert_eq!(word_index(64), 1);
        assert_eq!(words_for(0), 0);
        assert_eq!(words_for(1), 1);
        assert_eq!(words_for(63), 1);
        assert_eq!(words_for(64), 1);
        assert_eq!(words_for(65), 2);
        assert_eq!(last_word_mask(0), 0);
        assert_eq!(last_word_mask(1), 1);
        assert_eq!(last_word_mask(63), 0x7fff_ffff_ffff_ffff);
        assert_eq!(last_word_mask(64), usize::MAX);
        assert_eq!(last_word_mask(65), 1);
    }
}
