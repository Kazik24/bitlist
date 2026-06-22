use super::wrapper::BitList;
use crate::BitsIter;
use crate::heap::{
    HeapBitList, bcd_size_for_bits, bit_in_word_index, is_invalid_range, last_word_mask, word_index, words_for,
};
use crate::inline::InlineBitList;
use crate::iter::bounds_to_range;
use crate::util::{copy_bits_nonoverlapping, fill_bits, for_each_carry, unary_for_each_carry};
use crate::wrapper::{ReprByRef, ReprMut, ReprRef};
use std::alloc::Layout;
use std::cmp::Ordering;
use std::collections::TryReserveError;
use std::convert::Infallible;
use std::fmt::{Binary, Debug, Display, Formatter, LowerHex, UpperHex, Write};
use std::io::stdout;
use std::mem::{MaybeUninit, size_of};
use std::num::NonZeroUsize;
use std::ops::{Add, Bound, Range, RangeBounds};
use std::str::FromStr;

impl BitList {
    pub const NO_BITS: Self = InlineBitList::NO_BITS.to_list();
    pub const TRUE: Self = InlineBitList::TRUE.to_list();
    pub const FALSE: Self = InlineBitList::FALSE.to_list();
    pub const MAX_INLINE_BITS: usize = InlineBitList::MAX_INLINE_BITS as usize;

    pub const fn try_inline(data: usize, count: usize) -> Option<Self> {
        if count <= Self::MAX_INLINE_BITS {
            let v = InlineBitList::new_masked(data, count as u32);
            Some(Self::from_inline(v))
        } else {
            None
        }
    }
    pub const fn inline(data: usize, count: usize) -> Self {
        if count <= Self::MAX_INLINE_BITS {
            let v = InlineBitList::new_masked(data, count as u32);
            Self::from_inline(v)
        } else {
            panic!("bit count is too large for BitList")
        }
    }
    #[inline]
    pub const fn as_inline(&self) -> Option<&InlineBitList> {
        if let ReprByRef::Inline(inl) = self.inner_by_ref() {
            return Some(inl);
        }
        None
    }
    #[inline]
    pub const fn single(value: bool) -> Self {
        if value { Self::TRUE } else { Self::FALSE }
    }

    pub const fn from_u8(value: u8) -> Self {
        Self::from_inline(InlineBitList::new(value as _, 8))
    }
    pub const fn from_u16(value: u16) -> Self {
        Self::from_inline(InlineBitList::new(value as _, 16))
    }

    pub fn from_trunc_u64(value: u64, length: usize) -> Self {
        if length <= Self::MAX_INLINE_BITS {
            return Self::from_inline(InlineBitList::new_masked(value as usize, length as _));
        }
        let mut list = HeapBitList::zeros(length);
        let value = if length > 64 { value } else { value & ((1 << length) - 1) };
        let mut bytes = value.to_le_bytes();
        let chunks = bytes.chunks(HeapBitList::WORD_BYTES).map(|v| usize::from_le_bytes(v.try_into().unwrap()));
        list.init_from(chunks);
        Self::from_heap(list)
    }
    pub fn from_trunc_u128(value: u128, length: usize) -> Self {
        if length <= Self::MAX_INLINE_BITS {
            return Self::from_inline(InlineBitList::new_masked(value as usize, length as _));
        }
        let mut list = HeapBitList::zeros(length);
        let value = if length >= 128 { value } else { value & ((1 << length) - 1) };
        let mut bytes = value.to_le_bytes();
        const WB: usize = HeapBitList::WORD_BYTES;
        let chunks = bytes.as_chunks::<WB>().0.iter().map(|v| usize::from_le_bytes(*v));
        list.init_from(chunks);
        Self::from_heap(list)
    }

    pub const fn to_le_bytes<const N: usize>(&self) -> [u8; N] {
        match self.inner() {
            ReprRef::Heap(v) => v.to_le_bytes(),
            ReprRef::Inline(v) => v.to_le_bytes(),
        }
    }
    #[inline]
    fn collect_bytes<const N: usize>(iter: &mut impl Iterator<Item = u8>) -> [u8; N] {
        let mut arr = [0u8; N];
        let mut i = 0;
        while i < arr.len() {
            if let Some(v) = iter.next() {
                arr[i] = v;
            } else {
                break;
            }
            i += 1;
        }
        arr
    }

    pub fn zeros(count: usize) -> Self {
        Self::try_inline(0, count).unwrap_or_else(|| Self::from_heap(HeapBitList::zeros(count)))
    }
    pub fn zeros_like(&self) -> Self {
        Self::zeros(self.len())
    }
    pub fn ones(count: usize) -> Self {
        Self::try_inline(usize::MAX, count).unwrap_or_else(|| Self::from_heap(HeapBitList::ones(count)))
    }
    pub fn ones_like(&self) -> Self {
        Self::ones(self.len())
    }
    pub fn values(count: usize, value: bool) -> Self {
        if value { Self::ones(count) } else { Self::zeros(count) }
    }
    pub fn values_like(&self, value: bool) -> Self {
        Self::values(self.len(), value)
    }
    /// Create mask with only one bit with specific index set to `1`.
    pub fn mask_or_bit(length: usize, index: usize) -> Self {
        let mut list = Self::zeros(length);
        list.set(index, true);
        list
    }
    /// Create mask with only one bit with specific index set to `0`.
    pub fn mask_and_bit(length: usize, index: usize) -> Self {
        let mut list = Self::ones(length);
        list.set(index, false);
        list
    }
    fn with_capacity(count: usize) -> Self {
        Self::try_inline(0, count).unwrap_or_else(|| Self::from_heap(HeapBitList::with_capacity(count)))
    }
    pub fn from_le_bytes(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::NO_BITS;
        }
        let bits = bytes.len().checked_mul(8).expect("Capacity overflow");
        if bits <= Self::MAX_INLINE_BITS {
            let data = word_from_le_bytes(bytes); //no panic cause max bits is always lower thant usize bits
            return Self::from_inline(InlineBitList::new(data, bits as _));
        }
        let words = bytes.chunks(size_of::<usize>()).map(word_from_le_bytes);
        let mut list = HeapBitList::with_capacity(bits);
        list.init_from(words);
        unsafe {
            list.set_len(bits);
        }
        Self::from_heap(list)
    }

    pub fn from_bits<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = bool>,
        I::IntoIter: ExactSizeIterator,
    {
        Self::try_from_nibbles(iter.into_iter().map(Ok::<_, Infallible>)).unwrap()
    }

    pub fn try_from_bits<I, E>(iter: I) -> Result<Self, E>
    where
        I: IntoIterator<Item = Result<bool, E>>,
        I::IntoIter: ExactSizeIterator,
    {
        Self::try_from_nibbles(iter.into_iter())
    }

    fn try_from_nibbles<I, E, N>(mut iter: I) -> Result<Self, E>
    where
        N: Nibble,
        I: ExactSizeIterator<Item = Result<N, E>>,
    {
        let len = iter.len().checked_mul(N::SIZE as _).expect("Size overflow");
        if len <= Self::MAX_INLINE_BITS {
            let (data, len) = Self::try_collect_bits(&mut iter, len as _)?;
            return Ok(Self::from_inline(InlineBitList::new(data, len)));
        }
        let mut list = HeapBitList::with_capacity(len);
        unsafe {
            let last_mask = last_word_mask(len);
            let mut cnt = len;
            for word in list.uninit_data_mut() {
                let (data, count) = Self::try_collect_bits(&mut iter, usize::BITS)?;
                if cnt <= (count as usize) || count != usize::BITS {
                    word.write(data & last_mask);
                    break;
                } else {
                    word.write(data);
                    cnt -= count as usize;
                }
            }
            list.set_len(len);
        }
        Ok(Self::from_heap(list))
    }

    pub fn lit(text: &str) -> Self {
        match Self::parse_bits(text) {
            Ok(v) => v,
            Err(_) => panic!("Cannot parse literal: '{text}'"),
        }
    }
    pub fn parse_bits(text: &str) -> Result<Self, usize> {
        Self::parse_bits_ascii(text.as_bytes())
    }
    pub fn parse_bits_ascii(text: &[u8]) -> Result<Self, usize> {
        let iter = text.iter().copied().enumerate().rev().map(|(i, c)| match c {
            b'0' => Ok(false),
            b'1' => Ok(true),
            _c => Err(i),
        });
        Self::try_from_nibbles(iter)
    }
    pub fn parse_hex(text: &str) -> Result<Self, usize> {
        Self::parse_hex_ascii(text.as_bytes())
    }
    pub fn parse_hex_ascii(text: &[u8]) -> Result<Self, usize> {
        let iter = text.iter().copied().enumerate().rev().map(|(i, c)| match c {
            c @ b'0'..=b'9' => Ok(Hex((c - b'0') as _)),
            c @ b'a'..=b'f' => Ok(Hex((c - b'a' + 10) as _)),
            c @ b'A'..=b'F' => Ok(Hex((c - b'A' + 10) as _)),
            _c => Err(i),
        });
        Self::try_from_nibbles(iter)
    }
    pub fn parse_bcd(text: &str) -> Result<Self, usize> {
        Self::parse_bcd_ascii(text.as_bytes())
    }
    pub fn parse_bcd_ascii(text: &[u8]) -> Result<Self, usize> {
        let iter = text.iter().copied().enumerate().rev().map(|(i, c)| match c {
            c @ b'0'..=b'9' => Ok(Hex((c - b'0') as _)),
            _c => Err(i),
        });
        Self::try_from_nibbles(iter)
    }

    pub fn unsigned_binary_to_bcd(&self) -> Self {
        //     for(i = 0; i <= W+(W-4)/3; i = i+1) bcd[i] = 0;     // initialize with zeros
        //     bcd[W-1:0] = bin;                                   // initialize with input vector
        //     for(i = 0; i <= W-4; i = i+1)                       // iterate on structure depth
        //       for(j = 0; j <= i/3; j = j+1)                     // iterate on structure width
        //         if (bcd[W-i+4*j -: 4] > 4)                      // if > 4
        //           bcd[W-i+4*j -: 4] = bcd[W-i+4*j -: 4] + 4'd3; // add 3
        let len = self.len();
        let mut bcd = self.clone();
        bcd.resize(bcd_size_for_bits(len), Some(false));
        if len <= 3 {
            // 3 bits or less it always one digit
            return bcd;
        }
        for i in 0..=(len - 4) {
            // iterate on structure depth
            for j in 0..=(i / 3) {
                // iterate on structure width
                //if (bcd[W-i+4*j -: 4] > 4)                      // if > 4
                //    bcd[W-i+4*j -: 4] = bcd[W-i+4*j -: 4] + 4'd3; // add 3

                let idx = len - i + (4 * j) - 3;
                let value = bcd.get_value_at(idx, 4).unwrap();
                if value > 4 {
                    let update = (value + 3) & 0xf;
                    bcd.set_byte_at(idx, update as _, 4);
                }
            }
        }
        bcd
    }

    pub fn get_range<R: RangeBounds<usize>>(&self, range: R) -> Option<Self> {
        let range = bounds_to_range(range.start_bound(), range.end_bound(), self.len());
        match self.inner() {
            ReprRef::Inline(v) => v.get_range(range).map(Self::from_inline),
            ReprRef::Heap(v) => v.get_range(range),
        }
    }

    pub const fn set_range(&mut self, index: usize, value: &Self) -> Option<()> {
        let mut it = match self.try_range_iter_mut_from(index) {
            Some(it) => it,
            None => return None,
        };
        let value = value.iter();
        if !it.set_limit(value.len()) {
            return None;
        }
        it.copy_from(value);
        Some(())
    }

    pub fn set_range_fill(&mut self, range: impl RangeBounds<usize>, value: bool) {
        self.range_iter_mut(range).fill(value);
    }

    pub fn try_reserve(&mut self, additional: usize) -> Result<(), AllocateError> {
        match self.inner_mut() {
            ReprMut::Heap(v) => v.try_reserve(additional),
            ReprMut::Inline(v) => {
                let Some(new_cap) = v.len().checked_add(additional) else {
                    return Err(AllocateError::LengthOverflow { current: v.len(), additional });
                };
                if new_cap <= Self::MAX_INLINE_BITS {
                    return Ok(());
                }
                let mut heap = HeapBitList::try_with_capacity(new_cap).map_err(AllocateError::Allocation)?;
                heap.set_from_inline(*v);
                *self = Self::from_heap(heap);
                Ok(())
            }
        }
    }
    pub fn reserve(&mut self, additional: usize) {
        if let Err(err) = self.try_reserve(additional) {
            panic!("Capacity overflow, cannot reserve additional {additional} bits, {err}");
        }
    }

    /// Set length to 0 without truncating capacity, if instead you want to truncate capacity to
    /// minimal amount, consider assigning `= BitList::NO_BITS`
    pub fn clear(&mut self) {
        match self.inner_mut() {
            ReprMut::Inline(r) => *r = InlineBitList::NO_BITS,
            ReprMut::Heap(l) => {
                //SAFETY: it's always safe to set length to 0, remaining data will be treated as
                //uninitialized anyways
                unsafe {
                    l.set_len(0);
                }
            }
        }
    }

    pub fn format_decimal(&self) -> impl Display {
        let list = self.unsigned_binary_to_bcd();
        let bcd = format!("{list:x}");

        let mut s = bcd.chars().skip_while(|v| *v == '0').collect::<String>();
        if s.is_empty() {
            if self.is_empty() {
                s.push('_');
            } else {
                s.push('0');
            }
        }
        s
    }

    pub fn resize(&mut self, new_len: usize, fill: Option<bool>) {
        if new_len == 0 {
            self.clear();
            return;
        }
        match self.inner_mut() {
            ReprMut::Heap(l) => {
                let len = l.len();
                if len >= new_len {
                    //SAFETY: new length is less than current, this means that last word of
                    //this list will always be initialized, we meed to apply mask to it
                    //to clear any remaining bits
                    unsafe {
                        l.set_len(new_len);
                        let last = l.init_data_mut().last_mut().unwrap();
                        *last = last_word_mask(new_len);
                    }
                } else {
                    let fill = fill.unwrap_or_else(|| l.last_bit().unwrap_or(false));
                    l.try_ensure_capacity(new_len).unwrap();
                    if fill && let Some(v) = l.init_data_mut().last_mut() {
                        *v |= !last_word_mask(len); //fill remaining bits
                    }
                    let uninit = &mut l.uninit_data_mut()[words_for(len)..words_for(new_len)];
                    let to_fill = if fill { usize::MAX } else { 0 };
                    for v in uninit {
                        v.write(to_fill);
                    }
                    unsafe {
                        l.set_len(new_len);
                        //new len is > 0 so at least one word is initialized
                        //fix last bits
                        *l.init_data_mut().last_mut().unwrap() &= last_word_mask(new_len);
                    }
                }
            }
            ReprMut::Inline(l) => {
                let fill = fill.unwrap_or_else(|| l.last_bit().unwrap_or(false));
                let mut data = l.data();
                if fill {
                    data |= !l.word_mask(); //set other bits to 1
                }
                if new_len <= Self::MAX_INLINE_BITS {
                    //will apply bit mask
                    *l = InlineBitList::new_masked(data, new_len as _);
                } else {
                    let mut list = if fill { HeapBitList::ones(new_len) } else { HeapBitList::zeros(new_len) };
                    if new_len < HeapBitList::WORD_SIZE {
                        data &= last_word_mask(new_len)
                    }
                    //SAFETY: data has last word mask applied
                    unsafe {
                        list.set_first_word(data);
                        self.set_heap(list);
                    }
                }
            }
        }
    }

    pub fn truncate(&mut self, len: usize) {
        if len >= self.len() {
            return;
        }
        self.resize(len, Some(false));
    }

    pub fn shrink_to_fit(&mut self) {
        let ReprMut::Heap(list) = self.inner_mut() else {
            return;
        };
        let len = list.len();
        if len <= Self::MAX_INLINE_BITS {
            //list has only one word
            let data = list.first_word_init().unwrap_or(0);
            *self = Self::from_inline(InlineBitList::new(data, len as _));
            return;
        }

        unsafe {
            let mut vec = list.memory_mut_vec();
            vec.shrink_to_fit();
            vec.set(true);
        }
    }

    pub const fn to_inline(&self) -> Option<Self> {
        match self.to_inline_type() {
            Ok(v) => Some(Self::from_inline(v)),
            Err(_) => None,
        }
    }
    pub(crate) const fn to_inline_type(&self) -> Result<InlineBitList, &HeapBitList> {
        match self.inner() {
            ReprRef::Inline(v) => Ok(v),
            ReprRef::Heap(v) => {
                let len = v.len();
                if len <= Self::MAX_INLINE_BITS {
                    //list has only one word
                    match v.first_word_init() {
                        Some(d) => Ok(InlineBitList::new(d, len as _)),
                        None => Ok(InlineBitList::NO_BITS),
                    }
                } else {
                    Err(v)
                }
            }
        }
    }

    pub fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = bool>,
        I::IntoIter: ExactSizeIterator,
    {
        let mut iter = iter.into_iter();
        self.reserve(iter.len());
        for bit in iter {
            self.push_bit(bit);
        }
    }

    pub fn push_bit(&mut self, bit: bool) {
        self.push_bits(BitsIter::single(bit));
    }
    pub fn push_many_bits(&mut self, bit: bool, count: usize) {
        self.reserve(count);
        match self.inner_mut() {
            ReprMut::Inline(v) => {
                let len = v.len() + count;
                let data = v.data() | usize::MAX.wrapping_shl(v.len() as _);
                *v = InlineBitList::new_masked(data, len as _);
            }
            ReprMut::Heap(v) => {
                let len = v.len() + count; // reserve() checked that this will never overflow
                if words_for(v.len()) != words_for(len) {
                    if let Some(last) = v.words_for_init(len).last_mut() {
                        last.write(0); // init last uninit word - requirement for copy
                    }
                }
                // SAFETY: we resized the list to be big enough and last word is initialized
                unsafe {
                    fill_bits(v.data_ptr_mut(), v.len(), count, bit);
                    v.set_len(len);
                }
            }
        }
    }
    pub fn concat(&self, msb: &BitList) -> Self {
        let mut list = self.clone();
        list.push_list(msb);
        list
    }

    pub fn push_bits(&mut self, mut bits: BitsIter<'_>) {
        self.reserve(bits.len());
        match self.inner_mut() {
            ReprMut::Inline(v) => {
                let word = bits.next_unaligned_word();
                let len = v.len() + word.len();
                let data = v.data() | word.raw() << v.len();
                *v = InlineBitList::new(data, len as _);
            }
            ReprMut::Heap(v) => {
                let new_len = v.len() + bits.len(); // reserve() checked that this will never overflow
                if words_for(v.len()) != words_for(new_len) {
                    if let Some(last) = v.words_for_init(new_len).last_mut() {
                        last.write(0); // init last uninit word - requirement for copy
                    }
                }

                // SAFETY: we resized the list to be big enough and last word is initialized
                unsafe {
                    bits.copy_bits_nonoverlapping(v.data_ptr_mut(), v.len());
                    v.set_len(new_len);
                }
            }
        }
    }

    pub fn push_list(&mut self, value: &BitList) {
        self.push_bits(value.iter());
    }

    pub fn pop_bit(&mut self) -> Option<bool> {
        let last = self.last_bit()?;
        self.truncate(self.len() - 1);
        Some(last)
    }

    pub fn repeat(&mut self, count: usize) {
        if count == 0 {
            self.clear();
        } else {
            let me = self.clone();
            let repeats = count - 1;
            let to_add = self.len().checked_mul(repeats).expect("Length overflow");
            self.reserve(to_add);
            for _ in 0..repeats {
                self.push_list(&me);
            }
        }
    }

    fn collect_bits<T: Nibble>(iter: &mut impl Iterator<Item = T>, max_bits: u32) -> (usize, u32) {
        debug_assert!(usize::BITS % T::SIZE == 0);
        debug_assert!(max_bits <= usize::BITS);
        //debug_assert!(max_bits > 0);
        let mut acc = 0;
        let mut count = 0;
        for v in iter.take((max_bits / T::SIZE) as usize) {
            acc |= v.to_usize() << count;
            count += T::SIZE;
        }
        (acc, count)
    }
    fn try_collect_bits<T: Nibble, E>(
        iter: &mut impl Iterator<Item = Result<T, E>>,
        max_bits: u32,
    ) -> Result<(usize, u32), E> {
        debug_assert!(usize::BITS % T::SIZE == 0);
        debug_assert!(max_bits <= usize::BITS);
        let mut acc = 0;
        let mut count = 0;
        for v in iter.take((max_bits / T::SIZE) as usize) {
            acc |= v?.to_usize() << count;
            count += T::SIZE;
        }
        Ok((acc, count))
    }

    pub fn try_usize(&self) -> Option<usize> {
        let (len, first, _) = self.decompose();
        if len > usize::BITS as _ {
            return None;
        }
        Some(first as _)
    }
    pub fn try_u32(&self) -> Option<u32> {
        let (len, first, _) = self.decompose();
        if len > u32::BITS as _ {
            return None;
        }
        Some(first as _)
    }

    pub fn try_u128(&self) -> Option<u128> {
        if self.len() > u128::BITS as _ {
            return None;
        }
        let res = self.iter().enumerate().fold(0u128, |val, (i, bit)| val | ((bit as u128) << i));
        Some(res)
    }

    pub fn words(words: &[usize]) -> Self {
        if words.is_empty() {
            return Self::NO_BITS;
        }
        let bits = words.len() * (usize::BITS as usize);
        let mut list = HeapBitList::with_capacity(bits);
        list.init_from(words.iter().copied());
        unsafe {
            list.set_len(bits);
        }
        Self::from_heap(list)
    }
    pub const fn capacity(&self) -> usize {
        match self.inner() {
            ReprRef::Heap(v) => v.capacity(),
            ReprRef::Inline(_) => Self::MAX_INLINE_BITS,
        }
    }
    pub const fn heap_words_count(&self) -> usize {
        match self.inner() {
            ReprRef::Heap(v) => v.capacity_words(),
            ReprRef::Inline(_) => 0,
        }
    }
    pub const fn heap_bytes_count(&self) -> usize {
        self.heap_words_count() * HeapBitList::WORD_BYTES
    }

    ///decompose value into tuple of (length, first word, rest of words)
    fn decompose(&self) -> (usize, usize, &[usize]) {
        match self.inner() {
            ReprRef::Inline(v) => (v.len(), v.data(), &[]),
            ReprRef::Heap(v) => {
                let len = v.len();
                let arr = v.init_data();
                (len, arr[0], &arr[1..])
            }
        }
    }

    pub const fn count_ones(&self) -> usize {
        match self.inner() {
            ReprRef::Inline(v) => v.data().count_ones() as usize,
            ReprRef::Heap(v) => {
                let words = v.init_data();
                let mut i = 0;
                let mut sum = 0;
                while i < words.len() {
                    sum += words[i].count_ones() as usize;
                    i += 1;
                }
                sum
            }
        }
    }
    pub const fn count_zeros(&self) -> usize {
        self.len() - self.count_ones()
    }

    pub const fn next_bit(&self, bit_value: bool, from_idx: usize) -> Option<usize> {
        match self.range_iter_from(from_idx).bit_position(bit_value) {
            Some(idx) => Some(idx + from_idx),
            None => None,
        }
    }

    pub const fn next_set_bit(&self, from_idx: usize) -> Option<usize> {
        self.next_bit(true, from_idx)
        // if from_idx >= self.len() {
        //     return None;
        // }
        // match self.inner() {
        //     ReprRef::Inline(v) => {
        //         let word = v.data() & usize::MAX.wrapping_shl(bit_in_word_index(from_idx) as _);
        //         if word == 0 {
        //             return None;
        //         }
        //         Some(word.trailing_zeros() as _)
        //     }
        //     ReprRef::Heap(v) => {
        //         let array = v.init_data();
        //         let mut arr_idx = word_index(from_idx);
        //         let mut word = array[arr_idx] & usize::MAX.wrapping_shl(bit_in_word_index(from_idx) as _);
        //         while word == 0 {
        //             arr_idx += 1;
        //             if arr_idx >= array.len() {
        //                 return None;
        //             }
        //             word = array[arr_idx];
        //         }
        //         Some(((usize::BITS as usize) * arr_idx) + (word.trailing_zeros() as usize))
        //     }
        // }
    }

    pub const fn next_clr_bit(&self, from_idx: usize) -> Option<usize> {
        self.next_bit(false, from_idx)
    }

    const fn word(&self, word_index: usize) -> usize {
        match self.inner() {
            ReprRef::Inline(v) => {
                if word_index != 0 {
                    panic!("Index out of bounds");
                }
                v.data()
            }
            ReprRef::Heap(v) => v.init_data()[word_index],
        }
    }

    pub(crate) fn assign_for_each_carry<T>(
        &mut self,
        other: &Self,
        init: T,
        mut func: impl FnMut(&mut T, &mut usize, usize),
    ) -> (T, usize) {
        match (self.inner_mut(), other.inner()) {
            (ReprMut::Inline(a), ReprRef::Inline(b)) if !a.is_empty() => {
                assert_eq!(a.len(), b.len(), "Lists should have the same length");
                let mut value = a.data();
                let mut carry = init;
                func(&mut carry, &mut value, b.data());
                let ovf = a.set_data(value);
                (carry, ovf)
            }
            (ReprMut::Heap(a), ReprRef::Inline(b)) if !a.is_empty() => {
                assert_eq!(a.len(), b.len(), "Lists should have the same length");
                // SAFETY: we checked that length != 0 so at least one word need to be initialized
                let value = unsafe { &mut *a.data_ptr_mut() };
                let mut carry = init;
                func(&mut carry, value, b.data());
                let p = *value;
                *value &= last_word_mask(b.len());
                (carry, p & !last_word_mask(b.len()))
            }
            (ReprMut::Inline(a), ReprRef::Heap(b)) if !a.is_empty() => {
                assert_eq!(a.len(), b.len(), "Lists should have the same length");
                let mut value = a.data();
                let mut carry = init;
                // SAFETY: we checked that length != 0 so at least one word need to be initialized
                let b_val = unsafe { *b.data_ptr() };
                func(&mut carry, &mut value, b_val);
                let ovf = a.set_data(value);
                (carry, ovf)
            }
            (ReprMut::Heap(a), ReprRef::Heap(b)) => {
                // no need to check for 0 case, cause it is handled by for_each_carry
                assert_eq!(a.len(), b.len(), "Lists should have the same length");
                let value = a.init_data_mut();
                let c = for_each_carry(value, b.init_data(), init, func);
                if let Some(v) = value.last_mut() {
                    let p = *v;
                    *v &= last_word_mask(b.len());
                    (c, p & !last_word_mask(b.len()))
                } else {
                    (c, 0)
                }
            }
            //lengths are 0, so it is no-op (just return initial carry value)
            _ if other.is_empty() => (init, 0),
            _ => panic!("Lists should have the same length"),
        }
    }

    pub(crate) fn for_each_carry<T>(&mut self, init: T, mut func: impl FnMut(&mut T, &mut usize)) -> T {
        match self.inner_mut() {
            ReprMut::Inline(v) if !v.is_empty() => {
                let mut data = v.data();
                let mut carry = init;
                func(&mut carry, &mut data);
                v.set_data(data);
                carry
            }
            ReprMut::Heap(v) if !v.is_empty() => {
                let len = v.len();
                let value = v.init_data_mut();
                let c = unary_for_each_carry(value, init, func);
                if let Some(v) = value.last_mut() {
                    *v &= last_word_mask(len);
                }
                c
            }
            //lengths is 0, so it is no-op (just return initial carry value)
            _ => init,
        }
    }

    pub const fn get(&self, index: usize) -> Option<bool> {
        match self.inner() {
            ReprRef::Heap(v) => v.get_bit(index),
            ReprRef::Inline(v) => v.get_bit(index),
        }
    }
    /// Sets value of bit at given index, if index is out of bounds, returns None, else it
    /// returns previous bit value
    pub fn set(&mut self, index: usize, value: bool) -> Option<bool> {
        match self.inner_mut() {
            ReprMut::Heap(v) => v.set_bit(index, value),
            ReprMut::Inline(v) => v.set_bit(index, value),
        }
    }

    /// Reverse the order of bits
    pub fn reverse(&mut self) {
        match self.inner_mut() {
            ReprMut::Inline(v) => {
                let shift = HeapBitList::WORD_SIZE - v.len();
                let data = v.data();
                v.set_data(data.reverse_bits() >> shift);
            }
            ReprMut::Heap(v) => {
                //todo dont reallocate memory
                *self = Self::from_bits(BitsIter::from_heap(v, ..).rev());
            }
        }
    }

    pub const fn get_value_at(&self, index: usize, bits: usize) -> Option<u32> {
        match self.try_range_iter_from(index) {
            Some(mut iter) if !iter.is_empty() => Some(iter.next_bits(bits).raw() as u32),
            Some(_) | None => None,
        }
    }
    pub const fn get_byte_at(&self, index: usize) -> Option<u8> {
        match self.get_value_at(index, 8) {
            Some(v) => Some(v as u8),
            None => None,
        }
        // const SIZE: usize = u8::BITS as _;
        // self.try_range_iter_from(index)?.next_bits(SIZE);

        // match self.inner() {
        //     ReprRef::Inline(v) => {
        //         if index >= v.len() {
        //             None
        //         } else {
        //             Some(v.data().wrapping_shr(index as u32) as u8)
        //         }
        //     }
        //     ReprRef::Heap(v) => {
        //         if index >= v.len() {
        //             None
        //         } else {
        //             let words = v.init_data();
        //             let first_index = word_index(index);
        //             let bit_index = bit_in_word_index(index);
        //             let first = words[first_index].wrapping_shr(bit_index as _);
        //             //check if byte is in this word only (also handle edge case when overflowing usize)
        //             let next_idx = index.checked_add(SIZE - 1).map(word_index);
        //             if next_idx == Some(first_index) || next_idx.is_none() {
        //                 return Some(first as u8);
        //             }
        //             let byte_split = bit_index - (HeapBitList::WORD_SIZE - SIZE);
        //             let second =
        //                 words.get(first_index + 1).copied().unwrap_or(0).wrapping_shl((SIZE - byte_split) as _); //never fails

        //             //println!("{index}, first_index: {first_index}, next_idx: {next_idx:?}, byte_split: {byte_split}");

        //             Some((first | second) as u8)
        //         }
        //     }
        // }
    }
    pub fn set_byte_at(&mut self, index: usize, value: u8, value_len: usize) {
        assert!(value_len <= 8);
        // match self.inner_mut() {
        //     ReprMut::Inline(inl) => {
        //         if !inl.set_at(index, InlineBitList::new_masked(value as _, value_len as _)) {
        //             panic!("Value placed outside bit range.");
        //         }
        //     }
        //     ReprMut::Heap(alc) => {
        //         if !alc.set_at(index, &BitList::from_inline(InlineBitList::new_masked(value as _, value_len as _))) {
        //             panic!("Value placed outside bit range.");
        //         }
        //     }
        // }
        let inl = InlineBitList::new_masked(value as _, value_len as _);
        self.set_bits_at(index, inl.as_list().iter());
    }
    pub fn set_bits_at(&mut self, index: usize, value: BitsIter<'_>) {
        self.range_iter_mut_from(index).with_limit(value.len()).copy_from(value);
    }
    pub const fn last_bit(&self) -> Option<bool> {
        // assembly difference between this and `self.get_bit(self.len().checked_sub(1)?)`
        // is minimal but this one seems to have one less jump
        match self.inner() {
            ReprRef::Heap(v) => v.last_bit(),
            ReprRef::Inline(v) => v.last_bit(),
        }
    }
    pub const fn first_bit(&self) -> Option<bool> {
        self.get(0) //results in better assembly than using first_bit of both lists
    }

    pub const fn first_word(&self) -> usize {
        match self.inner() {
            ReprRef::Heap(v) => match v.first_word_init() {
                Some(v) => v,
                None => 0,
            },
            ReprRef::Inline(v) => v.data(),
        }
    }

    pub fn signed_partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let len = self.len();
        if len != other.len() {
            return None;
        }
        if len == 0 {
            return Some(Ordering::Equal);
        }
        let mut words = self.raw_words().zip(other.raw_words()).rev();
        let (a, b) = words.next().unwrap();
        let mask = last_word_mask(len) as isize;
        let last_bit_mask = (mask ^ (mask >> 1)) as usize;
        let mut cmp = if last_bit_mask != 0 {
            //check if not exactly aligned
            let a = if last_bit_mask & a != 0 { a as isize | !mask } else { a as isize };
            let b = if last_bit_mask & b != 0 { b as isize | !mask } else { b as isize };
            a.cmp(&b)
        } else {
            a.cmp(&b)
        };

        if cmp == Ordering::Equal {
            for (a, b) in words {
                cmp = a.cmp(&b);
                if cmp != Ordering::Equal {
                    break;
                }
            }
        }
        Some(cmp)
    }

    /// Check if list has all bits set to `0`. Return true also if list is empty.
    pub const fn is_zeros(&self) -> bool {
        match self.inner() {
            ReprRef::Heap(v) => {
                let words = v.init_data();
                let mut i = 0;
                while i < words.len() {
                    if words[i] != 0 {
                        return false;
                    }
                    i += 1;
                }
                true
            }
            ReprRef::Inline(v) => v.data() == 0,
        }
    }
    /// Check if list has all bits set to `1`. Return true also if list is empty.
    pub const fn is_ones(&self) -> bool {
        match self.inner() {
            ReprRef::Heap(v) => {
                let Some((&last, words)) = v.init_data().split_last() else {
                    return true; // empty
                };
                let mut i = 0;
                while i < words.len() {
                    if words[i] != usize::MAX {
                        return false;
                    }
                    i += 1;
                }
                let rest = v.len() % HeapBitList::WORD_SIZE;
                last.count_ones() == (rest as _)
            }
            ReprRef::Inline(v) => v.data().count_ones() == (v.len() as _),
        }
    }
    /// Check if list has minimum unsigned value, a.k.a 0. Returns false for empty list.
    pub const fn is_unsigned_min(&self) -> bool {
        self.is_zeros() && !self.is_empty()
    }
    /// Check if list has maximum unsigned value, a.k.a all bits set to `1`. Returns false for empty list.
    pub const fn is_unsigned_max(&self) -> bool {
        self.is_ones() && !self.is_empty()
    }
    /// Check if list has minimum signed value, a.k.a only last bit is set to `1`. Returns false for empty list.
    pub const fn is_signed_min(&self) -> bool {
        self.count_ones() == 1 && matches!(self.last_bit(), Some(true))
    }
    /// Check if list has maximum signed value, a.k.a all bits except last are set to `1`. Returns false for empty list.
    pub const fn is_signed_max(&self) -> bool {
        self.count_zeros() == 1 && matches!(self.last_bit(), Some(false))
    }

    pub const fn reduction_xor(&self) -> bool {
        self.count_ones() & 1 == 0
    }
    pub const fn reduction_xnor(&self) -> bool {
        self.count_ones() & 1 != 0
    }
}

impl PartialOrd for BitList {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.len() != other.len() {
            return None;
        }
        if self.is_empty() {
            return Some(Ordering::Equal);
        }
        Some(self.raw_words().rev().zip(other.raw_words().rev()).fold(Ordering::Equal, |o, (a, b)| o.then(a.cmp(&b))))
    }
}
// TODO implement
pub fn bit_range_copy(src: &[usize], dst: &mut [MaybeUninit<usize>], src_range: Range<usize>, dst_index: usize) {
    if let Some((src_range, dst_index)) = as_word_aligned(src_range, dst_index) {
        copy_ranges(src, dst, src_range, dst_index);
        return;
    }
    todo!()
}

fn copy_ranges<T: Copy>(src: &[T], dst: &mut [MaybeUninit<T>], src_range: Range<usize>, dst_index: usize) {
    if src_range.start > src_range.end {
        panic!("Start cannot be greater than end");
    }
    if src_range.end > src.len() {
        panic!("Source index out of bounds");
    }
    let len = src_range.end - src_range.start; //end is always greater or equal to start, no overflow
    if dst_index.checked_add(len).is_some_and(|i| i > dst.len()) {
        panic!("Destination index out of bound");
    }
    unsafe {
        // SAFETY: preconditions for length and ranges were checked in code above,
        // and since one slice is mutable, we know for sure that they don't overlap
        let src = src.as_ptr().add(src_range.start);
        let dst = dst.as_mut_ptr().cast::<T>().add(dst_index);
        std::ptr::copy_nonoverlapping(src, dst, len);
    }
}

fn as_word_aligned(src: Range<usize>, dst: usize) -> Option<(Range<usize>, usize)> {
    if bit_in_word_index(src.start) != 0 || bit_in_word_index(src.end) != 0 || bit_in_word_index(dst) != 0 {
        return None;
    }
    let range = Range { start: word_index(src.start), end: word_index(src.end) };
    Some((range, word_index(dst)))
}

fn word_from_le_bytes(bytes: &[u8]) -> usize {
    let mut arr = [0u8; size_of::<usize>()];
    arr[..bytes.len()].copy_from_slice(bytes);
    usize::from_le_bytes(arr)
}

impl FromStr for BitList {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.bytes().all(|b| b == b'0' || b == b'1') {
            return Err(());
        }
        let bits = s.bytes().map(|b| b == b'1').rev();
        Ok(BitList::from_bits(bits))
    }
}

impl Binary for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut buffer = [0u8; 128];
        let mut i = 0;

        for b in self.iter().rev() {
            buffer[i] = if b { b'1' } else { b'0' };
            i += 1;
            if i == buffer.len() {
                unsafe {
                    f.write_str(std::str::from_utf8_unchecked(&buffer))?;
                }
                i = 0;
            }
        }
        if i != 0 {
            unsafe {
                f.write_str(std::str::from_utf8_unchecked(&buffer[..i]))?;
            }
        }
        Ok(())
    }
}

impl UpperHex for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut index = 0;
        let mut buf = String::new();
        while index < self.len() {
            let byte = self.get_byte_at(index).unwrap();
            let ch = match byte & 0xf {
                v @ 0..=9 => (b'0' + v) as char,
                v => (b'A' + v - 10) as char,
            };
            buf.push(ch);
            index += 4;
        }
        for c in buf.chars().rev() {
            f.write_char(c)?;
        }
        Ok(())
    }
}
impl LowerHex for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut index = 0;
        let mut buf = String::new();
        while index < self.len() {
            let byte = self.get_byte_at(index).unwrap();
            let ch = match byte & 0xf {
                v @ 0..=9 => (b'0' + v) as char,
                v => (b'a' + v - 10) as char,
            };
            buf.push(ch);
            index += 4;
        }
        for c in buf.chars().rev() {
            f.write_char(c)?;
        }
        Ok(())
    }
}

impl Debug for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitList[{:b}]", self)
    }
}

// impl Display for BitList {
//     fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
//         if self.is_empty() {
//             write!(f, "_")
//         } else {
//             write!(f, "{:b}", self)
//         }
//     }
// }

trait Nibble {
    const SIZE: u32;
    fn to_usize(self) -> usize;
}

impl Nibble for bool {
    const SIZE: u32 = 1;
    fn to_usize(self) -> usize {
        self as _
    }
}
struct Hex(u8);
impl Nibble for Hex {
    const SIZE: u32 = 4;
    fn to_usize(self) -> usize {
        self.0 as _
    }
}
#[derive(Debug)]
pub enum AllocateError {
    CapacityOverflow,
    Allocation(Layout),
    Internal(TryReserveError),
    LengthOverflow { current: usize, additional: usize },
}

impl Display for AllocateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal(v) => write!(f, "Internal - {v}"),
            Self::CapacityOverflow => write!(f, "Capacity overflowed usize type"),
            Self::Allocation(lay) => write!(f, "Tried to allocate {} bytes", lay.size()),
            Self::LengthOverflow { current, additional } => {
                write!(f, "Length overflow - tried to extend length of {current} with additional {additional} bits.")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrapper::BitList;
    use rand::prelude::*;
    use std::collections::HashSet;
    use std::fmt::format;
    use std::ops::Range;
    use std::time::{Duration, Instant};

    fn generate_lists(rng: &mut impl Rng, ranges: impl IntoIterator<Item = usize>) -> Vec<BitList> {
        ranges.into_iter().map(|len| BitList::from_bits((0..len).map(|_| rng.random_bool(0.5)))).collect()
    }

    #[test]
    fn test_heap_alloc() {
        for i in 0..600 {
            let v = BitList::zeros(i);
        }
    }

    #[test]
    fn test_logic() {
        let mut list = "0010".parse::<BitList>().unwrap();
        let other = "0011".parse::<BitList>().unwrap();
        println!("ai: {},bi: {}", list.is_inline(), other.is_inline());
        list.assign_add_overflow(&other);
        println!("{:?}", list);
    }

    #[test]
    fn test_count_ones_zeros() {
        let mut rng = &mut StdRng::seed_from_u64(420);
        for i in 0..2000 {
            let len = rng.random_range(0..500);

            let mut zero_count = 0;
            let mut one_count = 0;
            let bits = (0..len).map(|_| {
                if rng.random_bool(0.5) {
                    one_count += 1;
                    true
                } else {
                    zero_count += 1;
                    false
                }
            });

            let list = BitList::from_bits(bits);

            //println!("{i}: {len}, list: {}", list.len());
            assert_eq!(len, list.len(), "List size doesn't match");
            assert_eq!(list.count_ones(), one_count, "Count ones failed\n{list:b}");
            assert_eq!(list.count_zeros(), zero_count, "Count zeros failed\n{list:b}");
        }
    }
    #[test]
    fn test_from_iter() {
        let mut rng = &mut StdRng::seed_from_u64(69);
        let mut bits = String::with_capacity(512);

        for len in [1, 2, 3, 4, 5, 6, 7, 8].map(|v| v * 32) {
            let mut zero_count = 0;
            let mut one_count = 0;
            let bits = (0..len)
                .map(|_| {
                    if rng.random_bool(0.5) {
                        one_count += 1;
                        true
                    } else {
                        zero_count += 1;
                        false
                    }
                })
                .collect::<Vec<_>>();

            //println!("{:?}", bits.iter().map(|v| if *v { "1" } else { "0" }).rev().collect::<String>());
            let list = BitList::from_bits(bits);
            assert_eq!(len, list.len(), "List size doesn't match");
            assert_eq!(list.count_ones(), one_count, "Count ones failed\n{list:b}");
            assert_eq!(list.count_zeros(), zero_count, "Count zeros failed\n{list:b}");
        }
    }

    #[test]
    fn test_print() {
        let list = BitList::from_le_bytes(&[123, 129, 0, 0, 0, 0, 0, 1, 3, 7]);
        println!("list: {list:?}");
        println!("First set bit: {:?}", list.next_set_bit(0));
        let set = list.set_bit_indexes(0).collect::<Vec<_>>();
        println!("Set bits: {:?}", set);
        assert_eq!(set.len(), list.count_ones());
    }

    #[test]
    fn test_set_bit_indexes() {
        let rand = &mut StdRng::seed_from_u64(96024);
        let mut bits = HashSet::new();
        for _ in 0..1000 {
            bits.clear();
            let length = rand.random_range(1..1000);
            let bit_count = rand.random_range(0..length);
            bits.extend((0..bit_count).map(|_| rand.random_range(0..length)));
            let mut list = BitList::zeros(length);
            for b in bits.iter().copied() {
                list.set(b, true);
            }
            let mut count = 0;
            for b in list.set_bit_indexes(0) {
                assert!(bits.contains(&b));
                count += 1;
            }
            assert_eq!(count, bits.len());
        }
    }

    #[test]
    fn test_add() {
        //simple
        let mut a = BitList::from_le_bytes(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]);
        let b = BitList::from_le_bytes(&[1, 0, 0, 0, 0, 0, 0, 0, 0]);
        a.assign_add_overflow(&b);
        assert_eq!(a.set_bit_indexes(0).collect::<Vec<_>>(), [71]);

        let rng = &mut StdRng::seed_from_u64(12345);
        for _ in 0..2000 {
            let cap = rng.random_range(30..110);
            let bits = rng.random_range(30..110);
            let value = rng.random_range(0..(1u128 << bits));

            //let v = BitList::zeros();
        }
    }

    #[test]
    fn test_push_list() {
        println!("Size of bit list: {}, inline bits: {}", size_of::<BitList>(), BitList::MAX_INLINE_BITS);
        let rng = &mut StdRng::seed_from_u64(1234);
        for (prob, cnt) in [(0.01, 2000), (0.001, 2000), (0.0001, 2000)] {
            let mut list = BitList::from_u8(0b10000101);
            println!("Set: {prob}, count:{cnt}");
            for _i in 0..cnt {
                if rng.random_bool(prob) {
                    list = BitList::NO_BITS;
                }
                let before = format!("{list:b}");
                let size = 0..(rng.random_range(0..BitList::MAX_INLINE_BITS));
                let added = BitList::from_bits(size.map(|_| rng.random_bool(0.5)));

                list.push_list(&added);

                //check if result is a concatenation
                let list = format!("{list:b}");
                let added = format!("{added:b}");

                //bits are printed in big endian order
                let expect = added.clone() + &before;
                let fill1 = " ".repeat(added.len());
                assert_eq!(list, expect, "\noriginal:{fill1} {before}\nadded:    {added}\n");
            }
        }
    }

    #[test]
    fn test_signed_cmp() {
        fn comp(a: i32, b: i32) -> Ordering {
            let a = a.to_be_bytes();
            let b = b.to_be_bytes();
            a[1..].iter().zip(b[1..].iter()).fold((a[0] as i8).cmp(&(b[0] as i8)), |o, (a, b)| o.then(a.cmp(b)))
        }

        for a in -1024i128..1024 {
            let a = a * 143213231232312338333;
            let al = BitList::from_trunc_u128(a as u128, 100);
            for b in -1024i128..1024 {
                let b = b * 123124823123435235747;
                let bl = BitList::from_trunc_u128(b as u128, 100);
                let exp = a.cmp(&b);
                let res = al.signed_partial_cmp(&bl).unwrap();
                assert_eq!(exp, res, "a: {}={:04X}, b: {}={:04X}\nal: {al:b}\nbl: {bl:b}", a, a, b, b);
            }
        }
    }

    #[test]
    fn test_write_binary() {
        let rng = &mut StdRng::seed_from_u64(12341234);
        let mut buff = String::with_capacity(1024);
        for _ in 0..100000 {
            buff.clear();
            for _ in 0..rng.random_range(0..512) {
                buff.push(if rng.random_bool(0.5) { '1' } else { '0' });
            }
            let fmt = format!("{:b}", BitList::lit(&buff));
            assert_eq!(buff, fmt);
        }
    }

    #[test]
    fn test_unsigned_cmp() {
        fn comp(a: i32, b: i32) -> Ordering {
            let a = a.to_be_bytes();
            let b = b.to_be_bytes();
            a[1..].iter().zip(b[1..].iter()).fold((a[0] as i8).cmp(&(b[0] as i8)), |o, (a, b)| o.then(a.cmp(b)))
        }

        for a in 0i128..2048 {
            let a = a * 143213231232312338333;
            let al = BitList::from_trunc_u128(a as u128, 100);
            for b in 0i128..2048 {
                let b = b * 123124823123435235747;
                let bl = BitList::from_trunc_u128(b as u128, 100);
                let exp = a.cmp(&b);
                let res = al.partial_cmp(&bl).unwrap();
                assert_eq!(exp, res, "a: {}={:04X}, b: {}={:04X}", a, a, b, b);
            }
        }
    }

    #[test]
    fn test_resize() {
        let mut list = BitList::lit("00101010");
        list.resize(10, None);
        assert_eq!(list, BitList::lit("0000101010"));
        list.resize(12, Some(true));
        assert_eq!(list, BitList::lit("110000101010"));
        list.resize(14, None);
        assert_eq!(list, BitList::lit("11110000101010"));
        list.resize(16, Some(false));
        assert_eq!(list, BitList::lit("0011110000101010"));

        let big = BitList::lit("010100101001001001010100101001001001010");
        let mut list = big.clone();
        list.resize(63, None);
        assert_eq!(list, BitList::lit("000000000000000000000000010100101001001001010100101001001001010"))
    }

    #[test]
    fn test_bcd() {
        let rng = &mut StdRng::seed_from_u64(98765);
        for num in [11, 123] {
            let data = BitList::from_trunc_u64(num, 8);
            let bcd = data.unsigned_binary_to_bcd();
            let chars = (bcd.len() / 4) + if bcd.len() % 4 == 0 { 0 } else { 1 };
            let expected = format!("{num:0width$}", width = chars);
            let value = format!("{:x}", bcd);
            assert_eq!(expected, value);
            assert_eq!(format!("{num}"), data.format_decimal().to_string())
        }
    }

    #[test]
    fn test_bcd_random() {
        let rng = &mut StdRng::seed_from_u64(98765132);
        let mut data = [0usize; 64];

        let mut dur = Duration::ZERO;
        let mut count = 0u32;
        for _ in 0..10 {
            unsafe { rng.fill_bytes(&mut data.align_to_mut::<u8>().1) };
            let iter = BitsIter::from_full_words(&data);
            let len = rng.random_range(0..iter.len());
            let list = BitList::from_bits(iter.with_limit(len));
            let start = Instant::now();
            let val = list.unsigned_binary_to_bcd();
            dur += start.elapsed();
            count += 1;
            std::hint::black_box(val);
        }
        println!("Avg bin to bcd: {:?}", dur / count);
    }

    #[test]
    fn test_get_byte() {
        let bits = "10101010_11100111_11000011_10000001_00000000_11110000_11100011_11001100_01010101".replace("_", "");
        let v = BitList::parse_bits(&bits).unwrap();
        for i in 0..v.len() {
            let mut vals = bits
                [bits.len().checked_sub(i).and_then(|v| v.checked_sub(8)).unwrap_or(0)..(bits.len() - i)]
                .to_string();
            for _ in 0..(8 - vals.len()) {
                vals.insert(0, '0');
            }
            let byte = format!("{:08b}", v.get_byte_at(i).unwrap());
            assert_eq!(vals, byte, "index {i}");
        }
        assert!(v.get_byte_at(v.len()).is_none());
    }

    #[test]
    fn test_format_decimal() {
        let rng = &mut StdRng::seed_from_u64(9876543211);
        for _ in 0..10000 {
            let number: u128 = rng.random();
            let length = rng.random_range(1..127);
            let number = number & ((1u128 << length) - 1);
            let bl = BitList::from_trunc_u128(number, length);

            let expected = format!("{number}");
            let got = bl.format_decimal().to_string();
            assert_eq!(expected, got);
        }
    }

    #[test]
    fn test_set_range() {
        let rng = &mut StdRng::seed_from_u64(54342);
        let mut bits = Vec::with_capacity(1024);
        let mut to_set = Vec::with_capacity(1024);
        for _ in 0..1000 {
            bits.clear();
            for _ in 0..rng.random_range(1..1024) {
                bits.push(rng.random_bool(0.5));
            }

            let start = rng.random_range(0..bits.len());
            let end = rng.random_range(start..bits.len());
            to_set.clear();
            for i in start..end {
                to_set.push(rng.random_bool(0.5));
            }

            let mut main = BitList::from_bits(bits.iter().copied());
            let mut set = BitList::from_bits(to_set.iter().copied());
            main.set_range(start, &set);
            bits[start..end].copy_from_slice(&to_set);
            assert!(main.iter().eq(bits.iter().copied()));
        }
    }
}
