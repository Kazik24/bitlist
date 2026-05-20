#![allow(unused)]

use std::alloc::Layout;
use std::fmt::{Binary, Debug, Formatter};
use std::marker::PhantomData;
use std::mem::{forget, replace, size_of, transmute, ManuallyDrop};
use std::ops::{Deref, DerefMut, Range, RangeBounds};
use std::ptr::NonNull;

pub struct BitList {
    pointer: NonNull<u8>,
}

const _: () = if BitList::LAYOUT.size() < 4 {
    panic!("Requires usize to be at least 4 bytes.");
} else {
    ()
};
const _: () = if BitList::LAYOUT.align() < 4 {
    panic!("Requires usize to be aligned to at least 4 bytes.");
} else {
    ()
};

#[inline]
const fn words_for(bits: usize) -> usize {
    (bits / BitList::BITS) + if bits % BitList::BITS == 0 { 0 } else { 1 }
}
#[inline]
const fn mask(bits: u32) -> usize {
    if bits >= BitList::BITS as _ {
        return usize::MAX;
    }
    (1usize << bits) - 1
}
#[inline]
const fn last_mask(bits: usize) -> usize {
    let b = bits % BitList::BITS;
    if bits != 0 && b == 0 {
        return usize::MAX;
    }
    mask(b as _)
}

impl BitList {
    const LAYOUT: Layout = Layout::new::<usize>();
    const BITS: usize = Self::LAYOUT.size() * 8;
    const RESERVED_MASK: usize = (Self::LAYOUT.align() - 1);
    const RESERVED_BITS: u32 = Self::RESERVED_MASK.count_ones();
    const INLINE_FLAG: usize = 1 << (Self::RESERVED_BITS - 1);
    const ALL_BITS: usize = 1; //todo flag to implement vectors of all zeros or all ones without allocation
    const INLINE_CNTBITS: u32 = (Self::BITS as u32 - Self::RESERVED_BITS).next_power_of_two().trailing_zeros();
    const INLINE_DATABITS: u32 = (Self::BITS as u32 - Self::RESERVED_BITS) - Self::INLINE_CNTBITS;
    const INL_COUNT_SHIFT: u32 = Self::RESERVED_BITS;
    const INL_COUNT_MASK: usize = mask(Self::INLINE_CNTBITS) << Self::INL_COUNT_SHIFT;
    const INL_DATA_SHIFT: u32 = Self::INL_COUNT_SHIFT + Self::INLINE_CNTBITS;
    const INL_DATA_MASK: usize = (2usize.pow(Self::INLINE_DATABITS) - 1) << Self::INL_DATA_SHIFT;

    // inline filed arrangement for 64 bit chunk:
    // [dddddddddddddddddddddddddddddddddddddddddddddddddddddddCCCCCCfrr]
    // d - data field, C - count field, r - reserved field where f is inline flag
    // inline repr is useful when storing relatively small amount of bits, it's faster and alloc free.

    pub const NONE: Self = Self::raw_inline(0, 0);

    #[inline]
    pub const fn single(value: bool) -> Self {
        Self::raw_inline(value as usize, 1)
    }
    #[inline]
    const fn raw_inline(data: usize, len: usize) -> Self {
        Self::from_inline(InlineRepr::new(data, len))
    }

    #[inline]
    pub fn ones(len: usize) -> Self {
        Self::new(len, true)
    }
    #[inline]
    pub fn zeros(len: usize) -> Self {
        Self::new(len, false)
    }
    pub fn new(len: usize, fill: bool) -> Self {
        if len <= Self::INLINE_DATABITS as _ {
            //store bits inline
            let mask = if fill { 2usize.pow(len as _) - 1 } else { 0 };
            Self::raw_inline(mask, len)
        } else {
            //create allocation
            let last = if fill { 2usize.pow((len % Self::BITS) as _) - 1 } else { 0 };
            let all = AllocRepr::allocate_bits(words_for(len), len, if fill { usize::MAX } else { 0 }, last);
            Self::from_alloc(all)
        }
    }
    pub fn truncate_u64(value: u64, len: usize) -> Self {
        if len <= Self::INLINE_DATABITS as _ {
            Self::raw_inline(value as usize & mask(len as _), len)
        } else {
            let mut value = if len >= size_of::<u64>() * 8 { value } else { value & ((1 << len) - 1) };
            let mut alc = AllocRepr::allocate_bits(words_for(len), len, 0, 0);
            for w in alc.words_mut() {
                *w = value as _;
                value = value.wrapping_shr((size_of::<usize>() * 8) as _);
            }
            Self::from_alloc(alc)
        }
    }

    pub fn to_u64(&self) -> Option<u64> {
        if self.len() > size_of::<u64>() * 8 {
            return None;
        }
        match self.inner() {
            Left(inl) => Some(inl.data() as _),
            Right(alc) => {
                let mut array = [0u8; size_of::<u64>()];
                let list = alc.words().iter().flat_map(|v| v.to_be_bytes());
                array.iter_mut().zip(list).for_each(|(a, b)| {
                    *a = b;
                });
                Some(u64::from_be_bytes(array))
            }
        }
    }
    pub fn to_i64(&self) -> Option<u64> {
        self.to_u64().map(
            |v| {
                if self.last_bit().unwrap_or(false) {
                    (v | !((1 << self.len()) - 1)) as _
                } else {
                    v as _
                }
            },
        )
    }

    pub fn set_single(&mut self, value: bool) {
        *self = Self::single(value);
    }

    pub fn from_hex(value: &str) -> Self {
        if !value.is_ascii() {
            panic!("unexpected chars detected");
        }
        Self::from_element_bits(value.as_bytes(), value.len(), 4, |&b| match b {
            v @ b'0'..=b'9' => (v - b'0') as _,
            v @ b'a'..=b'f' => (v - b'a' + 10) as _,
            v @ b'A'..=b'F' => (v - b'A' + 10) as _,
            c => panic!("unexpected character {:?}, only numbers from '0' to 'f' are allowed", c as char),
        })
    }
    pub fn from_bcd(value: &str) -> Self {
        if !value.is_ascii() {
            panic!("unexpected chars detected");
        }
        Self::from_element_bits(value.as_bytes(), value.len(), 4, |&b| match b {
            v @ b'0'..=b'9' => (v - b'0') as _,
            c => panic!("unexpected character {:?}, only numbers from '0' to '9' are allowed", c as char),
        })
    }
    pub fn from_bits(value: &str) -> Self {
        Self::truncate_from_bits(value, value.len())
    }
    fn from_element_bits<T>(
        value: &[T],
        chunk_count: usize,
        chunk_size: u32,
        mut conv: impl FnMut(&T) -> usize,
    ) -> Self {
        assert!(chunk_size > 0);
        assert_eq!(Self::BITS % chunk_size as usize, 0);
        let len = chunk_count * chunk_size as usize;
        let validation_mask = mask(chunk_size);
        let value = value.rchunks(len).next().unwrap_or(&[]);
        let mut words = value.rchunks(Self::BITS / chunk_size as usize).map(|chunk| {
            let mut data = 0usize;
            let mut shift = 0;
            for elem in chunk.iter().rev() {
                data |= (conv(elem) & validation_mask) << shift;
                shift += chunk_size;
            }
            data
        });

        if len <= Self::INLINE_DATABITS as _ {
            let data = words.next().unwrap_or(0);
            assert!(words.next().is_none());
            Self::raw_inline(data, len)
        } else {
            let mut alc = AllocRepr::allocate_bits(words_for(len), len, 0, 0);
            alc.words_mut().iter_mut().zip(words.by_ref()).for_each(|(v, w)| *v = w);
            assert!(words.next().is_none());
            Self::from_alloc(alc)
        }
    }
    pub fn truncate_from_bits(value: &str, len: usize) -> Self {
        if !value.is_ascii() {
            panic!("unexpected chars detected");
        }
        Self::from_element_bits(value.as_bytes(), len, 1, |&b| match b {
            b'1' => 1,
            b'0' => 0,
            c => panic!("unexpected character {:?}, only '1' or '0' are allowed", c as char),
        })
    }

    pub fn unsigned_binary_to_bcd(&self) -> Self {
        let len = self.len();
        let mut bcd = self.clone();
        bcd.resize(len + ((len - 4) / 3) + 1, Some(false));
        for i in 0..=(len - 4) {
            // iterate on structure depth
            for j in 0..=(i / 3) {
                // iterate on structure width
                //if (bcd[W-i+4*j -: 4] > 4)                      // if > 4
                //    bcd[W-i+4*j -: 4] = bcd[W-i+4*j -: 4] + 4'd3; // add 3

                let idx = len - i + 4 * j - 4;
                //todo not working
                println!("idx {}", idx);
                let mut value = bcd.get_byte_at(idx);
                if value > 4 {
                    value += 3;
                    bcd.set_byte_at(idx, value, 4);
                }
            }
        }
        bcd
    }

    pub fn len(&self) -> usize {
        match self.inner() {
            Left(inl) => inl.len(),
            Right(inner) => inner.read_len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn is_inline(&self) -> bool {
        self.inner().is_left()
    }
    pub fn capacity(&self) -> usize {
        match self.inner() {
            Left(_) => Self::INLINE_DATABITS as _,
            Right(inner) => inner.read_cap(),
        }
    }

    pub fn get_bit(&self, index: usize) -> Option<bool> {
        match self.inner() {
            Left(inl) => {
                if index >= inl.len() as _ {
                    return None;
                }
                Some(inl.data() & (1 << index) != 0)
            }
            Right(inner) => {
                let arr = inner.get_array();
                let count = arr[0];
                if index >= count {
                    return None;
                }
                let word = &arr[1..];
                let word = word[index / Self::BITS];
                Some(word & (1 << (index % Self::BITS)) != 0)
            }
        }
    }
    pub fn get_range(&self, range: Range<usize>) -> Self {
        todo!()
    }
    ///Get byte at bit offset
    pub fn get_byte_at(&self, index: usize) -> u8 {
        match self.inner() {
            Left(inl) => {
                if index >= inl.len() {
                    0
                } else {
                    inl.data().wrapping_shr(index as u32) as u8
                }
            }
            Right(alc) => {
                if index >= alc.read_len() {
                    0
                } else {
                    let w = alc.words()[index / Self::BITS];
                    w.wrapping_shr((index % Self::BITS) as _) as u8
                }
            }
        }
    }
    /// Set value at bit offset with specified bit length. Panics if it would overflow length.
    pub fn set_byte_at(&mut self, index: usize, value: u8, value_len: usize) {
        assert!(value_len <= 8);
        let mask = 2usize.pow(value_len as _) - 1;
        match self.inner_mut() {
            Left(inl) => {
                let len = inl.len();
                if index + value_len >= len {
                    panic!("Value placed outside bit range.");
                }
                let data = inl.data();
                let value = ((value as usize) & mask).wrapping_shl(index as _);
                let mask = mask.wrapping_shl(index as _);
                inl.set((data & mask) | value, len);
            }
            Right(alc) => {}
        }
    }
    pub fn last_bit(&self) -> Option<bool> {
        //todo optimize this
        let len = self.len();
        if len != 0 {
            return Some(self.get_bit(len - 1).unwrap());
        }
        None
    }
    pub fn set_bit(&mut self, index: usize, value: bool) -> bool {
        match self.inner_mut() {
            Left(inl) => {
                let len = inl.len();
                if index >= len {
                    return false;
                }
                let mask = (1 << index);
                let data = if value { inl.data() | mask } else { inl.data() & !mask };
                inl.set(data, len);
            }
            Right(alc) => {
                let len = alc.read_len();
                if index >= len {
                    return false;
                }
                let w = &mut alc.words_mut()[index / Self::BITS];
                let mask = 1 << (index % Self::BITS);
                if value {
                    *w |= mask;
                } else {
                    *w &= !mask;
                }
            }
        }
        true
    }

    pub fn shrink_to_fit(&mut self) {
        match self.inner_mut() {
            Left(_) => return, //nothing to shrink in inline form
            Right(inner) => {
                let len = inner.read_len();
                if len <= Self::INLINE_DATABITS as _ {
                    //dealloc memory and make it inline
                    let word = inner.words()[0]; //always only first word is used in this case
                                                 //word should have no more bits than len
                    debug_assert_ne!(word & !mask(len as _), 0);

                    *self = Self::raw_inline(word, len);
                } else {
                    //try reallocate memory
                    inner.shrink_mem_to(len);
                }
            }
        }
    }
    fn modify_assign<T: Copy>(
        &mut self,
        other: &Self,
        mut over: T,
        simple: impl FnOnce(usize, usize) -> usize,
        mut carry: impl FnMut(usize, usize, T) -> (usize, T),
    ) {
        let len = self.len();
        match (self.inner_mut(), other.inner()) {
            (Left(a), Left(b)) => {
                let data = simple(a.data(), b.data());
                a.set(data & mask(len as _), len);
            }
            (Left(a), Right(b)) => {
                let data = simple(a.data(), b.words()[0]);
                a.set(data & mask(len as _), len);
            }
            (Right(a), Left(b)) => {
                let res = &mut a.words_mut()[0];
                *res = simple(*res, b.data()) & mask(len as _);
            }
            (Right(a), Right(b)) => {
                let res = a.words_mut();
                for (a, b) in res.iter_mut().zip(b.words().iter().copied()) {
                    let (res, o) = carry(*a, b, over);
                    over = o;
                    *a = res;
                }
                if let Some(last) = res.last_mut() {
                    *last &= last_mask(len);
                }
            }
        }
    }
    pub fn wrapping_add(&self, other: &Self) -> Self {
        let mut ret = self.clone();
        ret.wrapping_add_assign(other);
        ret
    }
    pub fn wrapping_sub(&self, other: &Self) -> Self {
        let mut ret = self.clone();
        ret.wrapping_sub_assign(other);
        ret
    }
    pub fn wrapping_add_assign(&mut self, other: &Self) {
        assert_eq!(self.len(), other.len());
        self.modify_assign(
            other,
            0,
            |a, b| a.wrapping_add(b),
            |a, b, prev_ov| {
                let (res, ov1) = a.overflowing_add(prev_ov);
                let (res, ov2) = res.overflowing_add(b);
                (res, (ov1 | ov2) as _)
            },
        );
    }
    pub fn wrapping_sub_assign(&mut self, other: &Self) {
        assert_eq!(self.len(), other.len());
        self.modify_assign(
            other,
            0,
            |a, b| a.wrapping_sub(b),
            |a, b, prev_ov| {
                let (res, ov1) = a.overflowing_sub(prev_ov);
                let (res, ov2) = res.overflowing_sub(b);
                (res, (ov1 | ov2) as _)
            },
        );
    }

    pub fn reserve(&mut self, additional: usize) {
        match self.inner_mut() {
            Left(inl) if inl.len() + additional > Self::INLINE_DATABITS as _ => {
                //make allocation
                let mut alc = AllocRepr::allocate_bits(words_for(inl.len() + additional), inl.len(), 0, 0);
                alc.words_mut()[0] = inl.data();
                *self = Self::from_alloc(alc);
            }
            Right(alc) => {
                let cap = words_for(alc.read_len() + additional);
                alc.handle_realloc(|vec| {
                    let add = cap - vec.len();
                    vec.reserve(add);
                })
            }
            _ => {}
        }
    }

    pub fn append(&mut self, other: &Self) {
        let len = self.len();
        self.insert(len..len, other);
    }

    fn assert_valid_bits(&self) {
        match self.inner() {
            Left(inl) => assert_eq!(inl.data() & !mask(inl.len() as _), 0),
            Right(alc) => {
                let len = alc.read_len();
                let w = alc.words();
                assert_eq!(w.len(), words_for(len));
                if let Some(&l) = w.last() {
                    assert_eq!(l & !mask((len % Self::BITS) as _), 0);
                }
            }
        }
    }

    pub fn resize(&mut self, to_len: usize, extend_bit: Option<bool>) {
        let len = self.len();
        if to_len <= len {
            match self.inner_mut() {
                Left(inl) => inl.set(inl.data() & mask(to_len as _), to_len),
                Right(alc) => {
                    alc.get_array_mut()[0] = to_len;
                    *alc.words_mut().last_mut().unwrap() &= mask((to_len % Self::BITS) as _);
                }
            }
        } else {
            let add = to_len - len;
            self.reserve(add);
            match self.inner_mut() {
                Left(inl) => {
                    let bit = extend_bit.unwrap_or_else(|| inl.data() & (1usize << len.saturating_sub(1)) != 0);
                    if bit {
                        inl.set((mask(add as _) << len), to_len);
                    } else {
                        inl.set(inl.data(), to_len)
                    }
                }
                Right(alc) => {
                    todo!()
                }
            }
        }
    }

    pub fn insert(&mut self, range: Range<usize>, bits: &Self) {
        let len = self.len();
        //range checking, todo use std::slice::range for this
        if range.start > len || range.end > len || range.start > range.end {
            panic!("range out of bounds");
        }
        let removed_bits = range.len();
        debug_assert!(removed_bits <= len);
        let new_length = len - removed_bits + bits.len();
        self.resize(new_length, Some(false));
        //guaranteed to hold specified amount of bits
        match self.inner_mut() {
            Left(inl) => {
                let data = inl.data();
            }
            Right(alc) => {}
        }
    }

    fn inner(&self) -> Either<&InlineRepr, &AllocRepr> {
        let value = self.pointer.as_ptr() as usize;
        if value & Self::INLINE_FLAG != 0 {
            Either::Left(unsafe { transmute::<&NonNull<u8>, &InlineRepr>(&self.pointer) })
        } else {
            Either::Right(unsafe { transmute::<&NonNull<u8>, &AllocRepr>(&self.pointer) })
        }
    }
    fn inner_mut(&mut self) -> Either<&mut InlineRepr, &mut AllocRepr> {
        let value = self.pointer.as_ptr() as usize;
        if value & Self::INLINE_FLAG != 0 {
            Either::Left(unsafe { transmute::<&mut NonNull<u8>, &mut InlineRepr>(&mut self.pointer) })
        } else {
            Either::Right(unsafe { transmute::<&mut NonNull<u8>, &mut AllocRepr>(&mut self.pointer) })
        }
    }
    fn into_raw(self) -> Either<InlineRepr, AllocRepr> {
        let pointer = self.pointer;
        forget(self);
        let value = pointer.as_ptr() as usize;
        if value & Self::INLINE_FLAG != 0 {
            Either::Left(unsafe { transmute::<NonNull<u8>, InlineRepr>(pointer) })
        } else {
            Either::Right(unsafe { transmute::<NonNull<u8>, AllocRepr>(pointer) })
        }
    }

    #[inline]
    fn from_raw(inner: Either<InlineRepr, AllocRepr>) -> Self {
        match inner {
            Either::Left(val) => Self::from_inline(val),
            Either::Right(val) => Self::from_alloc(val),
        }
    }
    #[inline]
    const fn from_inline(val: InlineRepr) -> Self {
        unsafe { Self { pointer: transmute::<InlineRepr, NonNull<u8>>(val) } }
    }
    #[inline]
    const fn from_alloc(val: AllocRepr) -> Self {
        unsafe { Self { pointer: transmute::<AllocRepr, NonNull<u8>>(val) } }
    }
}

impl PartialEq for BitList {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        if self.is_empty() {
            return true;
        }
        match (self.inner(), other.inner()) {
            (Left(a), Left(b)) => a.value == b.value,
            (Left(a), Right(b)) => a.data() == b.words()[0],
            (Right(a), Left(b)) => a.words()[0] == b.data(),
            (Right(a), Right(b)) => a.words() == b.words(),
        }
    }
}

impl Clone for BitList {
    fn clone(&self) -> Self {
        Self::from_raw(self.inner().map_left(Clone::clone).map_right(Clone::clone))
    }
}
impl From<u8> for BitList {
    fn from(value: u8) -> Self {
        Self::raw_inline(value as _, 8)
    }
}
impl From<u16> for BitList {
    fn from(value: u16) -> Self {
        Self::raw_inline(value as _, 16)
    }
}

impl Debug for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitList[{:b}]", self)
    }
}
impl Binary for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for i in (0..self.len()).rev() {
            if self.get_bit(i).unwrap() {
                write!(f, "1")?;
            } else {
                write!(f, "0")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
#[repr(transparent)]
struct InlineRepr {
    value: usize,
}
#[repr(transparent)]
struct AllocRepr {
    pointer: NonNull<usize>,
}

impl InlineRepr {
    #[inline]
    pub const fn new(data: usize, len: usize) -> Self {
        debug_assert!(data & !mask(BitList::INLINE_DATABITS) == 0);
        debug_assert!(len & !mask(BitList::INLINE_CNTBITS) == 0);
        //safe cause we set inline flag manually, that means we cant have allocation
        let value = (data << BitList::INL_DATA_SHIFT) | (len << BitList::INL_COUNT_SHIFT) | BitList::INLINE_FLAG;
        Self { value }
    }
    #[inline]
    pub const fn data(&self) -> usize {
        (self.value & BitList::INL_DATA_MASK) >> BitList::INL_DATA_SHIFT
    }
    #[inline]
    pub const fn len(&self) -> usize {
        (self.value & BitList::INL_COUNT_MASK) >> BitList::INL_COUNT_SHIFT
    }
    #[inline]
    pub fn set(&mut self, data: usize, len: usize) {
        *self = Self::new(data, len);
    }
}

impl AllocRepr {
    fn allocate_bits(capacity_words: usize, bits: usize, fill: usize, last: usize) -> Self {
        let mut vec = Vec::with_capacity(capacity_words + 2);
        vec.push(capacity_words);
        vec.push(bits);
        vec.extend(std::iter::repeat(fill).take(capacity_words));
        debug_assert_eq!(vec.capacity(), capacity_words + 2);
        debug_assert_eq!(vec.capacity(), vec.len());
        *vec.last_mut().unwrap() = last;
        let ptr = ManuallyDrop::new(vec).as_mut_ptr();
        Self { pointer: unsafe { NonNull::new_unchecked(ptr as _) } }
    }
    fn shrink_mem_to(&mut self, len: usize) {
        if words_for(len) >= self.read_cap() {
            //check if shrink is needed
            return;
        }
        self.handle_realloc(|vec| vec.shrink_to_fit());
    }

    fn reserve(&mut self, additional: usize) {
        let len = self.read_len();
        let words = words_for(len + additional) - words_for(len);
        self.handle_realloc(|vec| vec.reserve_exact(words));
    }

    fn handle_realloc(&mut self, func: impl FnOnce(&mut Vec<usize>)) {
        //change to sentinel value that is know to not be allocated, this temporary invalidates
        //struct contract, that states that value has no inline flag set, and is valid pointer.
        let ptr = replace(&mut self.pointer, BitList::NONE.pointer.cast());
        unsafe {
            let ptr = ptr.as_ptr();
            let size = ptr.read() + 2;
            let bits = ptr.offset(1).read(); //second value is len
            let len = 2 + words_for(bits);
            debug_assert!(len <= size);
            debug_assert!(len >= 3);
            let mut vec = Vec::from_raw_parts(ptr, len, size);
            func(&mut vec); //if this panics, the vec is dropped and parent BitList inline 0 bit vector

            let cap = vec.capacity() - 2;
            vec.as_mut_ptr().write(cap); //fix capacity
            let ptr = ManuallyDrop::new(vec).as_mut_ptr();
            self.pointer = NonNull::new_unchecked(ptr)
        }
    }

    fn get_array_mut(&mut self) -> &mut [usize] {
        unsafe {
            let ptr = self.pointer.as_ptr();
            let size = ptr.read() + 2;
            &mut std::slice::from_raw_parts_mut(ptr, size)[1..]
        }
    }
    fn words_mut(&mut self) -> &mut [usize] {
        let arr = self.get_array_mut();
        let len = arr[0];
        &mut arr[1..(words_for(len) + 1)]
    }
    fn get_array(&self) -> &[usize] {
        unsafe {
            let ptr = self.pointer.as_ptr();
            let size = ptr.read() + 2;
            &std::slice::from_raw_parts(ptr, size)[1..]
        }
    }
    fn words(&self) -> &[usize] {
        let arr = self.get_array();
        let len = arr[0];
        &arr[1..(words_for(len) + 1)]
    }
    fn read_len(&self) -> usize {
        let ptr = self.pointer.as_ptr();
        unsafe { ptr.offset(1).read() }
    }
    fn read_cap(&self) -> usize {
        let ptr = self.pointer.as_ptr();
        unsafe { ptr.read() }
    }
}
impl Clone for AllocRepr {
    fn clone(&self) -> Self {
        unsafe {
            let ptr = self.pointer.as_ptr();
            let size = ptr.read() + 2;
            let b = std::slice::from_raw_parts(ptr, size).to_vec().into_boxed_slice();
            Self { pointer: NonNull::new_unchecked(Box::into_raw(b) as *mut usize as *mut _) }
        }
    }
}

impl Drop for AllocRepr {
    fn drop(&mut self) {
        unsafe {
            let ptr = self.pointer.as_ptr();
            debug_assert!((ptr as usize) & BitList::INLINE_FLAG == 0);
            let size = ptr.read() + 2;
            let _ = Box::from_raw(std::slice::from_raw_parts_mut(ptr, size));
        }
    }
}

impl Drop for BitList {
    fn drop(&mut self) {
        drop(replace(self, Self::NONE).into_raw());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy() {
        println!("{:064b}", BitList::INLINE_FLAG);
        println!("{:064b}", BitList::RESERVED_MASK);
        println!("{:064b}", BitList::INL_COUNT_MASK);
        println!("{:064b}", BitList::INL_DATA_MASK);
        println!("cb: {}", BitList::INLINE_DATABITS);
        let a = BitList::truncate_from_bits("110000000000000000000000000100101010110000", 130);
        let b = BitList::truncate_from_bits("100000000000000000000000000010101010110000", 130);
        let l = a.wrapping_add(&b);
        println!("{:?}, {}", l, l.is_inline());
        println!("{:?}", BitList::from_hex("FfAa09"));
        println!("{:?}", BitList::from_bcd("1209"));
    }
    #[test]
    fn test_add_assign() {
        for a in (0u64..1024).map(|v| v * 128) {
            let al = BitList::truncate_u64(a, 24);
            for b in (0u64..1024).map(|v| v * 128) {
                let exp = b.wrapping_add(a);
                let mut data = BitList::truncate_u64(b, 24);
                data.wrapping_add_assign(&al);
                let res = data.to_u64().unwrap();
                assert_eq!(res, exp as _);
            }
        }
    }
    #[test]
    fn test_sub_assign() {
        for a in (0u64..1024).map(|v| v * 128) {
            let al = BitList::truncate_u64(a, 24);
            for b in (0u64..1024).map(|v| v * 128) {
                let exp = b.wrapping_sub(a);
                let mut data = BitList::truncate_u64(b, 24);
                data.wrapping_sub_assign(&al);
                let res = data.to_i64().unwrap() as u64;
                assert_eq!(res, exp);
            }
        }
    }

    #[test]
    fn test_bcd() {
        let data = BitList::truncate_u64(123, 8);
        let bcd = data.unsigned_binary_to_bcd();
        println!("{:?}", bcd);
    }
}
