use crate::heap::{HeapBitList, bit_in_word_index, is_invalid_range, last_word_mask, word_index};
use crate::wrapper::{ReprByRef, ReprRef};
use crate::{BitList, InlineBitList};
use std::fmt::{Debug, Formatter};
use std::iter::FusedIterator;
use std::marker::PhantomData;
use std::mem::transmute;
use std::num::NonZeroUsize;
use std::ops::{Bound, Range, RangeBounds};
use std::ptr::NonNull;

impl BitList {
    pub const fn iter(&self) -> BitsIter<'_> {
        match self.inner_by_ref() {
            ReprByRef::Inline(inl) => BitsIter::from_inline_bounds(inl, Bound::Unbounded, Bound::Unbounded),
            ReprByRef::Heap(v) => BitsIter::from_heap_bounds(v, Bound::Unbounded, Bound::Unbounded),
        }
    }

    pub fn range_iter<R: RangeBounds<usize>>(&self, range: R) -> BitsIter<'_> {
        match self.inner_by_ref() {
            ReprByRef::Inline(inl) => BitsIter::from_inline(inl, range),
            ReprByRef::Heap(v) => BitsIter::from_heap(v, range),
        }
    }
    pub const fn range_iter_from(&self, start: usize) -> BitsIter<'_> {
        match self.inner_by_ref() {
            ReprByRef::Inline(inl) => BitsIter::from_inline_bounds(inl, Bound::Included(&start), Bound::Unbounded),
            ReprByRef::Heap(v) => BitsIter::from_heap_bounds(v, Bound::Included(&start), Bound::Unbounded),
        }
    }
    pub const fn range_iter_to(&self, end: usize) -> BitsIter<'_> {
        match self.inner_by_ref() {
            ReprByRef::Inline(inl) => BitsIter::from_inline_bounds(inl, Bound::Unbounded, Bound::Excluded(&end)),
            ReprByRef::Heap(v) => BitsIter::from_heap_bounds(v, Bound::Unbounded, Bound::Excluded(&end)),
        }
    }
    pub const fn range_iter_from_to(&self, start: usize, end: usize) -> BitsIter<'_> {
        match self.inner_by_ref() {
            ReprByRef::Inline(inl) => BitsIter::from_inline_bounds(inl, Bound::Included(&start), Bound::Excluded(&end)),
            ReprByRef::Heap(v) => BitsIter::from_heap_bounds(v, Bound::Included(&start), Bound::Excluded(&end)),
        }
    }

    pub fn raw_words(&self) -> RawWordsIter<'_> {
        match self.inner() {
            ReprRef::Inline(inl) => RawWordsIter { repr: Some(inl), words: Default::default() },
            ReprRef::Heap(heap) => RawWordsIter { repr: None, words: heap.init_data().iter() },
        }
    }

    pub fn word_bits_iter(&self) -> WordBitsIter<'_> {
        match self.inner() {
            ReprRef::Inline(i) => WordBitsIter { words: [].iter(), last: i.as_word_bits() },
            ReprRef::Heap(v) => {
                let (words, last) = v.init_data_split_last();
                WordBitsIter { words: words.iter(), last }
            }
        }
    }

    pub fn set_bit_indexes(&self, start: usize) -> SetBitIndexes<'_> {
        let index = start.min(self.len());
        SetBitIndexes { list: self, index }
    }
}

pub(crate) const fn bounds_to_range(start_bound: Bound<&usize>, end_bound: Bound<&usize>, len: usize) -> Range<usize> {
    Range {
        start: match start_bound {
            Bound::Unbounded => 0,
            Bound::Included(v) => *v,
            Bound::Excluded(v) => *v + 1,
        },
        end: match end_bound {
            Bound::Unbounded => len,
            Bound::Included(v) => *v + 1,
            Bound::Excluded(v) => *v,
        },
    }
}

pub struct BitsIter<'a> {
    list_ptr: NonNull<usize>, //pointer to the beginning of bit array
    start: usize,             //bit offset from start of pointer (inclusive)
    stop: usize,              //bit offset from start of pointer (exclusive)
    _phantom: PhantomData<&'a usize>,
}

impl Debug for BitsIter<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitsIter[len: {}]", self.len())
    }
}
impl Clone for BitsIter<'_> {
    #[inline]
    fn clone(&self) -> Self {
        self.copy()
    }
}

// SAFETY: it just keeps references to inline/heap BitList, or slice, so it's Send and Sync as well as them.
unsafe impl Send for BitsIter<'_> {}
unsafe impl Sync for BitsIter<'_> {}

impl<'a> BitsIter<'a> {
    pub const fn empty() -> Self {
        Self { list_ptr: NonNull::dangling(), start: 0, stop: 0, _phantom: PhantomData }
    }
    #[inline]
    pub const fn copy(&self) -> Self {
        /// this keeps just a shared refs, so multiple instances with same pointer can exist
        Self { list_ptr: self.list_ptr, start: self.start, stop: self.stop, _phantom: PhantomData }
    }
    pub const fn len(&self) -> usize {
        self.stop - self.start
    }
    pub const fn is_empty(&self) -> bool {
        self.start == self.stop
    }
    pub fn from_inline<R: RangeBounds<usize>>(inline: &'a InlineBitList, range: R) -> Self {
        Self::from_inline_bounds(inline, range.start_bound(), range.end_bound())
    }
    pub const fn from_inline_bounds(
        inline: &'a InlineBitList,
        start_bound: Bound<&usize>,
        end_bound: Bound<&usize>,
    ) -> Self {
        let len = inline.len();
        let range = bounds_to_range(start_bound, end_bound, len);
        if is_invalid_range(&range, len) {
            panic!("Invalid range");
        }
        Self {
            list_ptr: unsafe { transmute::<&'a InlineBitList, NonNull<usize>>(inline) },
            start: (InlineBitList::DATA_SHIFT as usize) + range.start,
            stop: (InlineBitList::DATA_SHIFT as usize) + range.end,
            _phantom: PhantomData,
        }
    }
    pub(crate) fn from_heap<R: RangeBounds<usize>>(heap: &'a HeapBitList, range: R) -> Self {
        Self::from_heap_bounds(heap, range.start_bound(), range.end_bound())
    }
    pub(crate) const fn from_heap_bounds(
        heap: &'a HeapBitList,
        start_bound: Bound<&usize>,
        end_bound: Bound<&usize>,
    ) -> Self {
        let len = heap.len();
        let range = bounds_to_range(start_bound, end_bound, len);
        if is_invalid_range(&range, len) {
            panic!("Invalid range");
        }
        Self {
            list_ptr: unsafe { NonNull::new_unchecked(heap.data_ptr().cast_mut()) },
            start: range.start,
            stop: range.end,
            _phantom: PhantomData,
        }
    }
    pub fn from_words<R: RangeBounds<usize>>(words: &'a [usize], range: R) -> Self {
        Self::from_words_bounds(words, range.start_bound(), range.end_bound())
    }
    const fn zero_pointer_offset(&mut self) {
        let start_offset = word_index(self.start);
        self.start -= start_offset; //never underflows
        self.stop -= start_offset; //never underflows
        // SAFETY: can never overflow and this iterator always accesses only words from start index onwards
        self.list_ptr = unsafe { self.list_ptr.add(start_offset) }
    }
    pub const fn from_words_bounds(words: &'a [usize], start_bound: Bound<&usize>, end_bound: Bound<&usize>) -> Self {
        let len =
            words.len().checked_mul(HeapBitList::WORD_SIZE as _).expect("Too large array, cannot bit-index with usize");
        let range = bounds_to_range(start_bound, end_bound, len);
        if is_invalid_range(&range, len) {
            panic!("Invalid range");
        }
        Self {
            list_ptr: unsafe { NonNull::new_unchecked(words.as_ptr().cast_mut()) },
            start: range.start,
            stop: range.end,
            _phantom: PhantomData,
        }
    }

    /// works as `.take(limit)` on iterator but doesn't change type and takes mutable reference, trurns true if limit was applied
    pub const fn set_limit(&mut self, limit: usize) -> bool {
        let len = (&*self).len();
        if limit >= len {
            return false; // no limit set, limit is bigger that size of this iterator
        }
        debug_assert!(self.stop >= self.start + limit);
        self.stop = self.start + limit;
        true
    }
    /// Move start to be 'limit' bits from end, returns true if limit was applied, false if it is bigger than length
    pub const fn set_end_limit(&mut self, limit: usize) -> bool {
        let len = (&*self).len();
        if limit >= len {
            return false; // no limit set, limit is bigger that size of this iterator
        }
        debug_assert!(self.start <= self.stop - limit);
        self.start = self.stop - limit;
        true
    }
    /// Similar to `.take(limit)` but doesn't change type
    pub const fn with_limit(mut self, limit: usize) -> Self {
        self.set_limit(limit);
        self
    }
    pub const fn with_end_limit(mut self, limit: usize) -> Self {
        self.set_end_limit(limit);
        self
    }

    // move by n words, stopping at next word alignment boundary
    pub const fn advance_words_by(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let start = match HeapBitList::WORD_SIZE.checked_mul(n) {
            Some(v) => match v.checked_add(self.start) {
                Some(v) => v,
                None => return,
            },
            None => return,
        };
        let floor = start & !(HeapBitList::WORD_SIZE - 1);
        debug_assert!(floor >= self.start);
        if floor > self.stop {
            self.start = self.stop;
        } else {
            self.start = floor
        }
    }
    pub fn advance_by(&mut self, n: usize) -> Result<(), NonZeroUsize> {
        if let Some(new_start) = self.start.checked_add(n).filter(|s| *s <= self.stop) {
            self.start = new_start;
            Ok(())
        } else {
            let len = self.len();
            self.start = self.stop;
            debug_assert_ne!(len, 0);
            Err(unsafe { NonZeroUsize::new_unchecked(len) })
        }
    }

    const fn read_word_at_bit_index(&self, index: usize) -> usize {
        debug_assert!(index >= self.start);
        debug_assert!(index < self.stop);
        unsafe { self.read_word_unchecked(word_index(index)) }
    }
    const unsafe fn read_word_unchecked(&self, index: usize) -> usize {
        unsafe { self.list_ptr.add(index).read() }
    }

    pub const fn peek_word(&self) -> WordBits {
        let len = self.len();
        if len == 0 {
            return WordBits::empty();
        }
        let word = self.read_word_at_bit_index(self.start);
        let idx = bit_in_word_index(self.start);
        let rest = HeapBitList::WORD_SIZE - idx;
        let min = if rest < len { rest } else { len };
        WordBits::new(word >> idx, min as _)
    }
    pub fn rpeek_word(&self) -> WordBits {
        let len = self.len();
        if len == 0 {
            return WordBits::empty();
        }
        let word = self.read_word_at_bit_index(self.stop - 1);
        let idx = bit_in_word_index(self.stop);
        println!("idx: {}", idx); //todo
        let rest = HeapBitList::WORD_SIZE - idx;
        let min = if rest < len { rest } else { len };
        WordBits::new(word, min as _)
    }

    /// Moves this iterator to next set bit or clear bit in sequence, and consumes this bit, returning offset from
    /// start, e.g if next() would return true, then this method consumes next(), and returns 0. and so on (for calling with value = true).
    /// Returns none if there are no more bits of given value in iterator.
    pub const fn bit_position(&mut self, value: bool) -> Option<usize> {
        let word = self.peek_word();
        if let Some(index) = word.first_bit_value(value) {
            self.start += (index + 1) as usize;
            debug_assert!(self.start <= self.stop);
            return Some(index as usize);
        }
        let mut count = word.len();
        self.start += count;
        debug_assert!(self.start <= self.stop);
        // check if word aligned (only applicable if it's not the last word)
        if cfg!(debug_assertions) && self.start != self.stop {
            debug_assert!(self.start % HeapBitList::WORD_SIZE == 0);
        }
        debug_assert!(self.stop <= usize::MAX - HeapBitList::WORD_SIZE, "overflow guard");

        let mut wi = word_index(self.start);
        if value {
            while self.start < self.stop {
                let word = unsafe { self.read_word_unchecked(wi) };
                let index = word.trailing_zeros();
                if index >= HeapBitList::WORD_SIZE as u32 {
                    // advance by word, for tight loop over sparse bits this is more likely branch
                    count += HeapBitList::WORD_SIZE;
                    self.start += HeapBitList::WORD_SIZE; //todo should this be saturating_add? (overflow case)
                    wi += 1;
                } else {
                    count += index as usize;
                    self.start += (index + 1) as usize;
                    if self.start > self.stop {
                        return None;
                    }
                    return Some(count);
                }
            }
        } else {
            while self.start < self.stop {
                let word = unsafe { self.read_word_unchecked(wi) };
                let index = word.trailing_ones();
                if index >= HeapBitList::WORD_SIZE as u32 {
                    // advance by word, for tight loop over sparse bits this is more likely branch
                    count += HeapBitList::WORD_SIZE;
                    self.start += HeapBitList::WORD_SIZE; //todo should this be saturating_add? (overflow case)
                    wi += 1;
                } else {
                    count += index as usize;
                    self.start += (index + 1) as usize;
                    if self.start > self.stop {
                        return None;
                    }
                    return Some(count);
                }
            }
        }

        self.start = self.stop; //end of iteration, fix position if advanced by more than end
        None
    }

    pub fn rbit_position(&mut self, value: bool) -> Option<usize> {
        let mut offset = self.len();
        loop {
            match self.next_back() {
                Some(val) if val == value => return Some(offset - 1),
                Some(_) => offset -= 1,
                None => break,
            }
        }
        None
    }

    /// Find next continuous slot of bits all of same value, return offset of this slot from start
    /// and advances this iterator to the end of that slot.
    pub const fn find_continuous_slot(&mut self, value: bool, length: usize) -> Option<usize> {
        if length == 0 {
            return Some(0); // empty slot is always available without advancing iterator
        }
        let length = length - 1; // bit_position always consumes at least one bit

        let mut index = 0;
        while let Some(offset) = self.bit_position(value) {
            index += offset;
            // println!("index: {index}, offset: {offset}");
            // with_limit to not check too far, e.g if there are few MB of same bits
            if let Some(end_off) = self.copy().with_limit(length).bit_position(!value) {
                // println!("end_off: {end_off}");
                if end_off >= length {
                    self.start += length; //here always valid to self.advance_by(length);
                    debug_assert!(self.start <= self.stop);
                    return Some(index);
                }
                index += end_off + 1;
                self.start += end_off; //here always valid to self.advance_by(end_off);
                debug_assert!(self.start <= self.stop);
            } else if (&*self).len() >= length {
                // no more opposite bits, but length bits availables
                self.start += length; //here always valid to self.advance_by(length);
                debug_assert!(self.start <= self.stop);
                return Some(index);
            }
        }
        None
    }

    /// TODO align data in this iter to word boundary
    pub fn word_aligned(&self) -> (WordBits, &'a [usize]) {
        let word = self.peek_word();
        let new_start = self.start + word.len();
        debug_assert!(new_start <= self.stop);
        // check if word aligned (only applicable if it's not the last word)
        if cfg!(debug_assertions) && new_start != self.stop {
            debug_assert!(new_start % HeapBitList::WORD_SIZE == 0);
        }

        todo!()
    }

    /// if all bits are the same then return that value, if there are multiple bit values or iterator is empty, return None.
    pub fn all_value(mut self) -> Option<bool> {
        if self.is_empty() {
            return None;
        }
        match self.bit_position(true) {
            Some(0) => match self.bit_position(false) {
                Some(_) => None,
                None => Some(true),
            },
            Some(_) => None,
            None => Some(false),
        }
    }
}

impl Iterator for BitsIter<'_> {
    type Item = bool;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.stop {
            None
        } else {
            let word = self.read_word_at_bit_index(self.start);
            let mask = 1 << bit_in_word_index(self.start);
            self.start += 1;
            Some(word & mask != 0)
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
    fn count(self) -> usize
    where
        Self: Sized,
    {
        self.len()
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        if let Some(new_start) = self.start.checked_add(n).filter(|s| *s <= self.stop) {
            self.start = new_start;
        } else {
            self.start = self.stop;
        }
        self.next()
    }
}
impl DoubleEndedIterator for BitsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start == self.stop {
            None
        } else {
            let new_stop = self.stop - 1;
            let word = self.read_word_at_bit_index(new_stop);
            let mask = 1 << bit_in_word_index(new_stop);
            self.stop = new_stop;
            Some(word & mask != 0)
        }
    }
    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        if let Some(new_stop) = self.stop.checked_sub(n).filter(|s| *s > self.start) {
            self.stop = new_stop;
        } else {
            self.stop = self.start;
        }
        self.next_back()
    }
}
impl ExactSizeIterator for BitsIter<'_> {
    fn len(&self) -> usize {
        self.stop - self.start
    }
}
impl FusedIterator for BitsIter<'_> {}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct WordBits {
    value: usize,
    count: u16,
}

impl WordBits {
    pub const BITS: u16 = usize::BITS as _;

    pub const fn empty() -> Self {
        Self::new_unchecked(0, 0)
    }
    pub const fn new(value: usize, count: u32) -> Self {
        assert!(count <= (usize::BITS as _), "Word cannot have that many bits");
        Self { value: value & last_word_mask(count as _), count: count as _ }
    }
    pub(crate) const fn new_unchecked(value: usize, count: u16) -> Self {
        debug_assert!(count <= (usize::BITS as _));
        Self { value, count }
    }
    pub const fn new_full(value: usize) -> Self {
        Self::new_unchecked(value, usize::BITS as _)
    }
    pub const fn raw(&self) -> usize {
        self.value
    }
    pub const fn len(&self) -> usize {
        self.count as _
    }
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub const fn first_set_bit(self) -> Option<u16> {
        let idx = self.value.trailing_zeros() as u16;
        if idx >= self.count { None } else { Some(idx) }
    }
    pub const fn first_clr_bit(self) -> Option<u16> {
        let idx = self.value.trailing_ones() as u16;
        if idx >= self.count { None } else { Some(idx) }
    }
    pub const fn first_bit_value(self, value: bool) -> Option<u16> {
        if value { self.first_set_bit() } else { self.first_clr_bit() }
    }
    // pub const fn last_set_bit(self) -> Option<u16> {
    //     let padding = Self::BITS - self.count;
    //     let idx = self.value.leading_zeros() as u16 - padding;

    //     if idx >= self.count { None } else { Some(idx) }
    // }
    // pub const fn last_clr_bit(self) -> Option<u16> {
    //     let value = self.value | !last_word_mask(self.count as _);
    //     let idx = self.value.leading_ones() as u16;
    //     if idx >= self.count { None } else { Some(idx) }
    // }
    // pub const fn last_bit_value(self, value: bool) -> Option<u16> {
    //     if value { self.last_set_bit() } else { self.last_clr_bit() }
    // }
}

impl Debug for WordBits {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordBits")
            .field("value", &format_args!("0b{:b}", self.value))
            .field("count", &self.count)
            .finish()
    }
}

impl Iterator for WordBits {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count == 0 {
            return None;
        }
        self.count -= 1;
        let ret = self.value & 1 != 0;
        self.value = self.value.wrapping_shr(1);
        Some(ret)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.count as _, Some(self.count as _))
    }
}
impl FusedIterator for WordBits {}
impl ExactSizeIterator for WordBits {
    fn len(&self) -> usize {
        self.count as _
    }
}
impl DoubleEndedIterator for WordBits {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.count == 0 {
            return None;
        }
        self.count -= 1;
        let mask = 1usize.wrapping_shl(self.count as _);
        let ret = mask & self.value != 0;
        self.value &= mask - 1;
        Some(ret)
    }
}

pub struct RawWordsIter<'a> {
    repr: Option<InlineBitList>,
    words: std::slice::Iter<'a, usize>,
}

impl Iterator for RawWordsIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self.repr.take() {
            Some(val) => {
                if val.is_empty() {
                    return None;
                }
                Some(val.data())
            }
            None => self.words.next().copied(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.repr.map(|v| if v.is_empty() { 0 } else { 1 }).unwrap_or_else(|| self.words.len());
        (len, Some(len))
    }
}
impl ExactSizeIterator for RawWordsIter<'_> {}
impl DoubleEndedIterator for RawWordsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self.words.next_back().copied() {
            Some(v) => Some(v),
            None => {
                let repr = self.repr.take()?;
                if repr.is_empty() { None } else { Some(repr.data()) }
            }
        }
    }
}

pub struct WordBitsIter<'a> {
    words: std::slice::Iter<'a, usize>,
    last: WordBits,
}

impl Iterator for WordBitsIter<'_> {
    type Item = WordBits;

    fn next(&mut self) -> Option<Self::Item> {
        match self.words.next() {
            Some(v) => Some(WordBits::new_full(*v)),
            None => {
                if self.last.is_empty() {
                    None
                } else {
                    let ret = self.last;
                    self.last = WordBits::empty();
                    Some(ret)
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl DoubleEndedIterator for WordBitsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.last.is_empty() {
            self.words.next_back().map(|v| WordBits::new_full(*v))
        } else {
            let ret = self.last;
            self.last = WordBits::empty();
            Some(ret)
        }
    }
}

impl ExactSizeIterator for WordBitsIter<'_> {
    fn len(&self) -> usize {
        self.words.len() + if self.last.is_empty() { 0 } else { 1 }
    }
}

#[derive(Clone)]
pub struct SetBitIndexes<'a> {
    list: &'a BitList,
    index: usize,
}

impl Iterator for SetBitIndexes<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        self.list.next_set_bit(self.index).inspect(|index| {
            self.index = *index + 1;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::prelude::*;
    use std::{
        iter::{once, repeat_n},
        panic::catch_unwind,
    };

    #[test]
    fn test_word_bits_forward_backward() {
        let rng = &mut StdRng::seed_from_u64(12312312300);
        for _ in 0..2000 {
            let len = rng.random_range(0..=usize::BITS);
            let word = rng.random::<u64>() as usize;
            let bits_rev = WordBits::new_unchecked(word, len as _).rfold(0, |v, b| (v << 1) | (b as usize));
            let bits = WordBits::new_unchecked(word, len as _).enumerate().fold(0, |v, (i, b)| v | ((b as usize) << i));
            assert_eq!(bits, bits_rev);
            if len == usize::BITS as _ {
                assert_eq!(word, bits_rev);
            } else {
                let mask = (1 << len) - 1;
                assert_eq!(word & mask, bits_rev);
            }
        }
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)]
    fn test_inline_iter() {
        let list = InlineBitList::new(1, 1);
        assert_eq!(BitsIter::from_inline(&list, 0..0).collect::<Vec<_>>(), vec![]);
        assert_eq!(BitsIter::from_inline(&list, 0..0).rev().collect::<Vec<_>>(), vec![]);
        assert_eq!(BitsIter::from_inline(&list, 0..1).collect::<Vec<_>>(), vec![true]);
        assert_eq!(BitsIter::from_inline(&list, 0..1).rev().collect::<Vec<_>>(), vec![true]);
        catch_unwind(|| BitsIter::from_inline(&list, 1..0)).unwrap_err();
        catch_unwind(|| BitsIter::from_inline(&list, 0..2)).unwrap_err();
        let list = InlineBitList::new(2, 2);
        assert_eq!(BitsIter::from_inline(&list, 0..0).collect::<Vec<_>>(), vec![]);
        assert_eq!(BitsIter::from_inline(&list, 0..0).rev().collect::<Vec<_>>(), vec![]);
        assert_eq!(BitsIter::from_inline(&list, 0..1).collect::<Vec<_>>(), vec![false]);
        assert_eq!(BitsIter::from_inline(&list, 0..1).rev().collect::<Vec<_>>(), vec![false]);
        assert_eq!(BitsIter::from_inline(&list, 0..2).collect::<Vec<_>>(), vec![false, true]);
        assert_eq!(BitsIter::from_inline(&list, 0..2).rev().collect::<Vec<_>>(), vec![true, false]);
    }

    #[test]
    fn test_find_next_set_bit() {
        let rng = &mut StdRng::seed_from_u64(676767);
        for _ in 0..10000 {
            let mut list = BitList::zeros(rng.random_range(1..260));
            let count_set = rng.random_range(0..(list.len() as f32 / 3.0).ceil() as usize);
            let mut set_bits_idx = (0..count_set).map(|_| rng.random_range(0..list.len())).collect::<Vec<_>>();
            set_bits_idx.sort_unstable();
            set_bits_idx.dedup();
            for idx in &set_bits_idx {
                list.set(*idx, true);
            }
            let mut iter = list.iter();

            let mut list_index = 0;
            let mut place_index = 0;
            while let Some(index) = iter.bit_position(true) {
                let exp_idx = set_bits_idx[place_index] - list_index;
                assert_eq!(exp_idx, index, "place: {}", set_bits_idx[place_index]);
                list_index += index + 1;
                place_index += 1;
            }
        }
    }

    #[test]
    fn test_find_next_clear_bit() {
        // perf test: this approaches the scan rate of around 17.69 bits per cpu cycle on my machine @2.2ghz, for sparse data, which is pretty good
        let rng = &mut StdRng::seed_from_u64(676767);
        for _ in 0..10000 {
            let mut list = BitList::ones(rng.random_range(1..260));
            let count_set = rng.random_range(0..(list.len() as f32 / 3.0).ceil() as usize);
            let mut set_bits_idx = (0..count_set).map(|_| rng.random_range(0..list.len())).collect::<Vec<_>>();
            set_bits_idx.sort_unstable();
            set_bits_idx.dedup();
            for idx in &set_bits_idx {
                list.set(*idx, false);
            }
            let mut iter = list.iter();

            let mut list_index = 0;
            let mut place_index = 0;
            while let Some(index) = iter.bit_position(false) {
                let exp_idx = set_bits_idx[place_index] - list_index;
                assert_eq!(exp_idx, index, "place: {}", set_bits_idx[place_index]);
                list_index += index + 1;
                place_index += 1;
            }
        }
    }

    #[test]
    fn test_bit_position_equivalence() {
        let rng = &mut StdRng::seed_from_u64(969696);
        for i in 0..10 {
            let vals = BitList::from_trunc_u128(1 << i, 9);
            println!(
                "position: {:?}, bit_position: {:?}, vals: {vals:?}",
                vals.iter().position(|v| v),
                vals.iter().bit_position(true)
            );
        }
        for _ in 0..10000 {
            let vals = BitList::from_trunc_u128(rng.random(), rng.random_range(0..128));
            assert_eq!(vals.iter().position(|v| !v), vals.iter().bit_position(false), "{vals:?}");
        }
        for _ in 0..10000 {
            let vals = BitList::from_trunc_u128(rng.random(), rng.random_range(0..128));
            assert_eq!(vals.iter().position(|v| v), vals.iter().bit_position(true), "{vals:?}");
        }
    }

    macro_rules! bool_vec {
        ($($val:literal : $num:expr),* $(,)?) => {
            [false; 0].into_iter()
                $( .chain([$val; $num]) )*
                .collect::<Vec<bool>>()
        };
    }

    const PW: usize = HeapBitList::WORD_SIZE;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_bits_iter() {
        let iter = BitsIter::from_words(&[0b00000010_00010001], 0..PW).collect::<Vec<_>>();
        assert_eq!(iter, bool_vec!(true: 1, false: 3, true: 1, false: 4, true: 1, false: 54));
        let iter = BitsIter::from_words(&[0b00000010_00010001], 0..64).rev().collect::<Vec<_>>();
        assert_eq!(iter, bool_vec!(false: 54, true: 1, false: 4, true: 1, false: 3, true: 1));
        let iter = BitsIter::from_words(&[0b00000010_00010001], 1..63).collect::<Vec<_>>();
        assert_eq!(iter, bool_vec!(false: 3, true: 1, false: 4, true: 1, false: 53));
        let iter = BitsIter::from_words(&[0b00000010_00010001, 0b11], 1..66).collect::<Vec<_>>();
        assert_eq!(iter, bool_vec!(false: 3, true: 1, false: 4, true: 1, false: 54, true: 2));
        let iter = BitsIter::from_words(&[0b00000010_00010001, 0b11], 1..66).rev().collect::<Vec<_>>();
        assert_eq!(iter, bool_vec!(true: 2, false: 54, true: 1, false: 4, true: 1, false: 3));
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_peek_word() {
        let word = BitsIter::from_words(&[0b00000010_00010001], 0..64).peek_word();
        assert!(word.len() == 64 && word.raw() == 0b00000010_00010001);
        let word = BitsIter::from_words(&[0b00000010_00010001 | 0x8000_0000_0000_0000], 0..64).peek_word();
        assert!(word.len() == 64 && word.raw() == 0b00000010_00010001 | 0x8000_0000_0000_0000);
        let word = BitsIter::from_words(&[0b00000010_00010001 | 0x8000_0000_0000_0000], 0..63).peek_word();
        assert!(word.len() == 63 && word.raw() == 0b00000010_00010001);
        let word = BitsIter::from_words(&[0b00000010_00010001 | 0x8000_0000_0000_0000], 1..63).peek_word();
        assert!(word.len() == 62 && word.raw() == 0b00000010_0001000);
    }

    #[test]
    #[ignore]
    #[cfg(target_pointer_width = "64")]
    fn test_rpeek_word() {
        let word = BitsIter::from_words(&[0b00000010_00010001], 0..64).rpeek_word();
        assert!(word.len() == 64 && word.raw() == 0b00000010_00010001);
        let word = BitsIter::from_words(&[0b00000010_00010001 | 0x8000_0000_0000_0000], 0..64).rpeek_word();
        assert!(word.len() == 64 && word.raw() == 0b00000010_00010001 | 0x8000_0000_0000_0000);
        let word = BitsIter::from_words(&[0b00000010_00010001 | 0x8000_0000_0000_0000], 0..63).rpeek_word();
        println!("word: {word:?}");
        assert!(word.len() == 63 && word.raw() == 0b00000010_00010001);
        // let word = BitsIter::from_words(&[0b00000010_00010001 | 0x8000_0000_0000_0000], 1..63).rpeek_word();
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_find_continuous_slot() {
        // check 0 sized slot doesn't advance iter
        let mut iter = BitsIter::from_words(&[0b00000010_00010001], 0..64);
        assert_eq!(iter.find_continuous_slot(true, 0), Some(0));
        assert_eq!(iter.find_continuous_slot(false, 0), Some(0));
        assert_eq!(iter.len(), 64);
        // check normal advance
        assert_eq!(iter.find_continuous_slot(false, 4), Some(5));
        assert_eq!(iter.len(), 55);
        // check no slots left
        let mut iter = BitsIter::from_words(&[0b00001011_00010001], 0..12);
        assert_eq!(iter.find_continuous_slot(false, 4), None);
        assert_eq!(iter.len(), 0);
        // check first slot
        let mut iter = BitsIter::from_words(&[0b00001011_00010000], 0..12);
        assert_eq!(iter.find_continuous_slot(false, 4), Some(0));
        assert_eq!(iter.len(), 8);
        // check 1 size slot
        let mut iter = BitsIter::from_words(&[0b00001011_01110111], 0..12);
        assert_eq!(iter.find_continuous_slot(false, 1), Some(3));
        assert_eq!(iter.len(), 8);
        // check bigger slot
        println!("check bigger slot at end");
        let mut iter = BitsIter::from_words(&[0b00001011_01110111], 0..60);
        assert_eq!(iter.find_continuous_slot(false, 3), Some(12));
        assert_eq!(iter.len(), 45);
        let mut iter = BitsIter::from_words(&[0b00000000_00000000], 0..4);
        assert_eq!(iter.find_continuous_slot(false, 4), Some(0));
        assert_eq!(iter.len(), 0);
    }

    #[test]
    fn test_rbit_position() {
        let rng = &mut StdRng::seed_from_u64(12312312314);
        for i in 0..10 {
            let vals = BitList::from_trunc_u128(1 << i, 9);
            println!(
                "rposition: {:?}, rbit_position: {:?}, vals: {vals:?}",
                vals.iter().rposition(|v| v),
                vals.iter().rbit_position(true)
            );
        }
        for _ in 0..10000 {
            let vals = BitList::from_trunc_u128(rng.random(), rng.random_range(0..128));
            assert_eq!(vals.iter().rposition(|v| !v), vals.iter().rbit_position(false), "{vals:?}");
        }
        for _ in 0..10000 {
            let vals = BitList::from_trunc_u128(rng.random(), rng.random_range(0..128));
            assert_eq!(vals.iter().rposition(|v| v), vals.iter().rbit_position(true), "{vals:?}");
        }
    }
}
