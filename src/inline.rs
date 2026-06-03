use crate::heap::{bit_in_word_index, is_invalid_index, is_invalid_range, set_bit_value};
use crate::iter::WordBits;
use crate::wrapper::BitList;
use std::num::NonZeroUsize;
use std::ops::Range;

#[derive(Copy, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "align_16", repr(C, packed(2)))]
#[cfg_attr(all(feature = "align_32", not(feature = "align_16")), repr(C, packed(4)))]
#[cfg_attr(all(not(feature = "align_32"), not(feature = "align_16")), repr(transparent))]
pub struct InlineBitList {
    val: NonZeroUsize,
}

const _: () =
    assert!(usize::BITS == (InlineBitList::COUNT_BITS + InlineBitList::DATA_BITS + InlineBitList::COUNT_SHIFT));
impl InlineBitList {
    // inline filed arrangement for 64 bit arch:
    // [dddddddddddddddddddddddddddddddddddddddddddddddddddddddddCCCCCCf]
    // and for 32 bit arch:
    // [ddddddddddddddddddddddddddCCCCCf]
    // Fields:
    // * d - data field
    // * C - bit count field
    // * f - inline flag marker (always 1 for inline variant)
    // inline repr is useful when storing relatively small amount of bits, it's faster and alloc free.
    pub(crate) const COUNT_BITS: u32 = (usize::BITS - 1).count_ones();
    pub(crate) const MASK_COUNT_BITS: usize = ((usize::BITS - 1) as usize) << Self::COUNT_SHIFT;
    pub(crate) const MASK_DATA_BITS: usize = ((1usize << Self::DATA_BITS) - 1) << Self::DATA_SHIFT;
    pub(crate) const DATA_BITS: u32 = usize::BITS - Self::COUNT_SHIFT - Self::COUNT_BITS;
    pub(crate) const COUNT_SHIFT: u32 = 1;
    pub(crate) const DATA_SHIFT: u32 = Self::COUNT_BITS + Self::COUNT_SHIFT;
    pub(crate) const INLINE_FLAG: usize = 1;
    pub const NO_BITS: Self = InlineBitList::new(0, 0);
    pub const TRUE: Self = InlineBitList::new(1, 1);
    pub const FALSE: Self = InlineBitList::new(0, 1);
    pub const MAX_INLINE_BITS: u32 = InlineBitList::DATA_BITS;

    #[inline]
    pub const fn to_list(self) -> BitList {
        BitList::from_inline(self)
    }
    #[inline]
    pub const fn as_list(&self) -> &BitList {
        BitList::ref_inline(self)
    }
    #[inline]
    pub const fn as_word_bits(&self) -> WordBits {
        WordBits::new_unchecked(self.data(), self.len() as _)
    }
    pub(crate) const fn new(data: usize, count: u32) -> Self {
        debug_assert!(count <= Self::DATA_BITS);
        debug_assert!(data <= ((1 << count) - 1));
        let count = count as usize;
        let val = data.wrapping_shl(Self::DATA_SHIFT) | count.wrapping_shl(Self::COUNT_SHIFT) | Self::INLINE_FLAG;
        //SAFETY: We explicitly set INLINE_FLAG so value is never 0
        unsafe { Self { val: NonZeroUsize::new_unchecked(val) } }
    }
    pub const fn concat(self, msb: Self) -> Option<Self> {
        let a_len = self.len();
        let len = a_len + msb.len();
        if len > Self::MAX_INLINE_BITS as _ {
            return None;
        }
        let data = self.data() + (msb.data() << a_len);
        Some(Self::new(data, len as _))
    }
    pub const fn new_masked(data: usize, count: u32) -> Self {
        let mask = (1 << count) - 1;
        Self::new(data & mask, count)
    }
    pub const fn data(&self) -> usize {
        self.val.get().wrapping_shr(Self::DATA_SHIFT)
    }
    pub const fn len(&self) -> usize {
        (self.val.get() & Self::MASK_COUNT_BITS).wrapping_shr(Self::COUNT_SHIFT)
    }
    pub const fn last_bit(&self) -> Option<bool> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let mask = 1 << (len - 1);
        Some(self.data() & mask != 0)
    }
    pub const fn first_bit(&self) -> Option<bool> {
        if self.is_empty() {
            return None;
        }
        Some(self.data() & 1 != 0)
    }
    pub const fn get_bit(&self, index: usize) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        Some(self.data() & (1 << bit_in_word_index(index)) != 0)
    }
    pub fn set_bit(&mut self, index: usize, value: bool) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        //SAFETY: we just checked that index is in bound, so given word index will be valid for read/write
        //and calculated mask won't set any bits outside length
        let mut word = self.data();
        let prev = set_bit_value(&mut word, index, value);
        self.set_data(word);
        Some(prev)
    }
    #[inline]
    pub const fn to_le_bytes<const N: usize>(&self) -> [u8; N] {
        let mut array = [0u8; N];
        let bytes = self.data().to_le_bytes();
        let len = if N < bytes.len() { N } else { bytes.len() };
        //SAFETY: we just picked length that would not overflow either array
        let mut i = 0;
        while i < len {
            array[i] = bytes[i];
            i += 1;
        }
        array
    }
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub const fn get_raw_repr(&self) -> NonZeroUsize {
        self.val
    }
    pub const fn word_mask(&self) -> usize {
        (1 << self.len()) - 1
    }
    pub fn set_data(&mut self, data: usize) -> usize {
        let count_and_flag = self.val.get() & !Self::MASK_DATA_BITS;
        let mask = self.word_mask();
        let val = count_and_flag | (data & mask).wrapping_shl(Self::DATA_SHIFT);
        //SAFETY: count_and_flag contains already at least INLINE_FLAG set
        unsafe { self.val = NonZeroUsize::new_unchecked(val) }
        data & !mask //return overflowing bits
    }

    pub const fn get_range(&self, range: Range<usize>) -> Option<Self> {
        if is_invalid_range(&range, self.len()) {
            return None;
        }
        let new_len = range.end - range.start;
        Some(Self::new_masked(self.data() << range.start, new_len as _))
    }

    pub fn set_at(&mut self, index: usize, value: Self) -> bool {
        if is_invalid_index(self.len(), index, value.len()) {
            return false;
        }
        let mask = (1usize << value.len()) - 1;
        let len = self.len();
        let data = self.data();
        let value = (value.data() & mask).wrapping_shl(index as _);
        let mask = mask.wrapping_shl(index as _);
        *self = InlineBitList::new((data & !mask) | value, len as _);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::inline::InlineBitList;
    use crate::*;
    use std::fmt::{Binary, Display};
    use std::mem::size_of_val;

    macro_rules! pc {
        ($val: expr) => {{
            let val = $val;
            let bits = size_of_val(&val) * 8;
            println!("{:0width$b} = {} // {}", val, val, stringify!($val), width = bits);
        }};
    }
    #[test]
    fn print_consts() {
        pc!(InlineBitList::COUNT_BITS);
        pc!(InlineBitList::MASK_COUNT_BITS);
        pc!(InlineBitList::MASK_DATA_BITS);
        pc!(InlineBitList::DATA_BITS);
        pc!(InlineBitList::COUNT_SHIFT);
        pc!(InlineBitList::COUNT_BITS);
        pc!(InlineBitList::INLINE_FLAG);
    }
}
