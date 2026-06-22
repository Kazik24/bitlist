use crate::heap::HeapBitList;
use crate::inline::InlineBitList;
use std::hash::{Hash, Hasher};
use std::mem::{ManuallyDrop, align_of, size_of, transmute};
use std::num::NonZeroUsize;
use std::ptr::NonNull;

pub(crate) type NonZeroPtr = NonNull<usize>;
#[cfg_attr(feature = "align_16", repr(C, packed(2)))]
#[cfg_attr(all(feature = "align_32", not(feature = "align_16")), repr(C, packed(4)))]
#[cfg_attr(all(not(feature = "align_32"), not(feature = "align_16")), repr(transparent))]
pub struct BitList {
    //this is a pointer to keep memory provenance
    val: NonZeroPtr,
}
unsafe impl Send for BitList {}
unsafe impl Sync for BitList {}

//assert sizes of struct, and niche optimizations
const _: () = assert!(size_of::<BitList>() == size_of::<HeapBitList>());
const _: () = assert!(size_of::<BitList>() == size_of::<InlineBitList>());
const _: () = assert!(align_of::<BitList>() >= align_of::<HeapBitList>());
const _: () = assert!(align_of::<BitList>() >= align_of::<InlineBitList>());
const _: () = assert!(size_of::<BitList>() == size_of::<Option<BitList>>());
const _: () = assert!(size_of::<InlineBitList>() == size_of::<Option<InlineBitList>>());
const _: () = assert!(size_of::<HeapBitList>() == size_of::<Option<HeapBitList>>());
const _: () = assert!(size_of::<BitList>() >= 4);
const _: () = assert!(align_of::<usize>() >= 2);

impl BitList {
    #[inline]
    pub(crate) const fn from_inline(value: InlineBitList) -> Self {
        // SAFETY: we know that InlineBitList is transparent, and it must have INLINE_FLAG set so is never zero
        unsafe { transmute::<InlineBitList, BitList>(value) }
    }
    #[inline]
    pub(crate) fn from_heap(value: HeapBitList) -> Self {
        // SAFETY: we know that HeapBitList is transparent, and it must have INLINE_FLAG cleared, also pointer is never zero
        unsafe { transmute::<HeapBitList, BitList>(value) }
    }
    #[inline]
    pub(crate) const fn ref_inline(value: &InlineBitList) -> &Self {
        // SAFETY: we know that InlineBitList is transparent, and it must have INLINE_FLAG set so is never zero
        unsafe { transmute::<&InlineBitList, &BitList>(value) }
    }
    #[inline]
    pub(crate) fn ref_heap(value: &HeapBitList) -> &Self {
        // SAFETY: we know that HeapBitList is transparent, and it must have INLINE_FLAG cleared, also pointer is never zero
        unsafe { transmute::<&HeapBitList, &BitList>(value) }
    }
    #[inline]
    pub(crate) fn from_inner(inner: Repr) -> Self {
        match inner {
            Repr::Heap(v) => Self::from_heap(v),
            Repr::Inline(v) => Self::from_inline(v),
        }
    }
    #[inline]
    pub const fn is_inline(&self) -> bool {
        // SAFETY: NonNull<usize> has same alignment and size as usize
        // CONST SAFETY: Const values can only be created from inline repr, which is a integer
        // so since it's never and address, we can transmute it to type from which it came from
        let val = unsafe { transmute::<NonZeroPtr, usize>(self.val) };
        val & InlineBitList::INLINE_FLAG != 0
    }
    #[inline]
    pub const fn by_ref(&self) -> &Self {
        self
    }
    #[inline]
    pub(crate) const fn inner(&self) -> ReprRef<'_> {
        // SAFETY: transmuting references here is safe cause both types have #[repr(transparent)]
        // with same size and alignment as NonZeroUsize, or just usize
        // then we check upfront which representation is it
        // CONST SAFETY: Const values can only be created from inline repr, which is a integer
        // so since it's never and address, we can transmute it to type from which it came from
        if self.is_inline() {
            //this is transmuted as value, cause another pointer indirection would be wasteful
            ReprRef::Inline(unsafe { transmute::<NonZeroPtr, InlineBitList>(self.val) })
        } else {
            ReprRef::Heap(unsafe { transmute::<&BitList, &HeapBitList>(self) })
        }
    }
    #[inline]
    pub(crate) const fn inner_by_ref(&self) -> ReprByRef<'_> {
        // SAFETY: transmuting references here is safe cause both types have #[repr(transparent)]
        // with same size and alignment as NonZeroUsize, or just usize
        // then we check upfront which representation is it
        // CONST SAFETY: Const values can only be created from inline repr, which is a integer
        // so since it's never and address, we can transmute it to type from which it came from
        if self.is_inline() {
            //this is transmuted as value, cause another pointer indirection would be wasteful
            ReprByRef::Inline(unsafe { transmute::<&BitList, &InlineBitList>(self) })
        } else {
            ReprByRef::Heap(unsafe { transmute::<&BitList, &HeapBitList>(self) })
        }
    }
    #[inline]
    pub(crate) const fn inner_mut(&mut self) -> ReprMut<'_> {
        // SAFETY: transmuting references here is safe cause both types have #[repr(transparent)]
        // with same size and alignment as NonZeroUsize, or just usize
        // then we check upfront which representation is it
        if self.is_inline() {
            ReprMut::Inline(unsafe { transmute::<&mut BitList, &mut InlineBitList>(self) })
        } else {
            ReprMut::Heap(unsafe { transmute::<&mut BitList, &mut HeapBitList>(self) })
        }
    }
    ///transform inline to heap, without running any drop check
    #[inline]
    pub(crate) unsafe fn set_heap(&mut self, list: HeapBitList) {
        debug_assert!(self.is_inline());
        self.val = unsafe { transmute::<HeapBitList, NonZeroPtr>(list) };
    }
    #[inline]
    pub(crate) const fn into_inner(self) -> Repr {
        // SAFETY: transmuting values here is safe cause both types have #[repr(transparent)]
        // with same size and alignment as NonZeroUsize, or just usize
        // then we check upfront which representation is it
        // CONST SAFETY: Const values can only be created from inline repr, which is a integer
        // so since it's never and address, we can transmute it to type from which it came from
        if self.is_inline() {
            Repr::Inline(unsafe { transmute::<BitList, InlineBitList>(self) })
        } else {
            Repr::Heap(unsafe { transmute::<BitList, HeapBitList>(self) })
        }
    }

    // pub const fn len(&self) -> usize { (actually latest tests on rust 1.93 shows that hand optimized len is same as this)
    //     match self.inner() {
    //         ReprRef::Heap(v) => v.len(),
    //         ReprRef::Inline(v) => v.len(),
    //     }
    // }
    pub const fn len(&self) -> usize {
        //this has better assembly than normal len (2 instructions less = in total 8 ops)
        let ptr = self.val;
        let val: usize = unsafe { transmute::<NonZeroPtr, usize>(ptr) };
        if (val) & InlineBitList::INLINE_FLAG != 0 {
            (val & InlineBitList::MASK_COUNT_BITS).wrapping_shr(InlineBitList::COUNT_SHIFT)
        } else {
            unsafe { ptr.read() }
        }
    }

    // pub const fn is_empty(&self) -> bool {
    //     match self.inner() {
    //         ReprRef::Heap(v) => v.is_empty(),
    //         ReprRef::Inline(v) => v.is_empty(),
    //     }
    // }
    pub const fn is_empty(&self) -> bool {
        //this has better assembly than normal is_empty (2 instructions less = in total 11 ops)
        let ptr = self.val;
        let val: usize = unsafe { transmute::<NonZeroPtr, usize>(ptr) };
        if (val) & InlineBitList::INLINE_FLAG != 0 {
            (val & InlineBitList::MASK_COUNT_BITS) == 0
        } else {
            unsafe { ptr.read() == 0 }
        }
    }
}
#[derive(Clone, Eq, PartialEq)]
pub enum Repr {
    Heap(HeapBitList),
    Inline(InlineBitList),
}
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum ReprRef<'a> {
    Heap(&'a HeapBitList),
    Inline(InlineBitList),
}
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum ReprByRef<'a> {
    Heap(&'a HeapBitList),
    Inline(&'a InlineBitList),
}
#[derive(Eq, PartialEq)]
pub enum ReprMut<'a> {
    Heap(&'a mut HeapBitList),
    Inline(&'a mut InlineBitList),
}

impl Clone for BitList {
    fn clone(&self) -> Self {
        match self.inner() {
            ReprRef::Inline(v) => Self::from_inline(v),
            ReprRef::Heap(v) => {
                // clone is an opportunity to trim capacity, and also
                // reduce to no allocation at all
                let len = v.len();
                if len <= Self::MAX_INLINE_BITS {
                    //downgrade to inline, in case of small amount of bits
                    Self::from_inline(InlineBitList::new(v.first_word_init().unwrap_or(0), len as _))
                } else {
                    Self::from_heap(v.clone())
                }
            }
        }
    }
}

impl PartialEq for BitList {
    fn eq(&self, other: &Self) -> bool {
        match (self.inner(), other.inner()) {
            (ReprRef::Inline(a), ReprRef::Inline(b)) => a == b,
            (ReprRef::Heap(a), ReprRef::Heap(b)) => a == b,
            (ReprRef::Inline(a), ReprRef::Heap(b)) | (ReprRef::Heap(b), ReprRef::Inline(a)) => {
                let len = a.len();
                if len != b.len() {
                    return false;
                }
                if len != 0 {
                    //unwrap should never be reached
                    return a.data() == b.first_word_init().unwrap_or(0);
                }
                true
            }
        }
    }
}
impl Eq for BitList {}
impl Hash for BitList {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.inner() {
            ReprRef::Heap(v) => {
                state.write_usize(v.len());
                for v in v.init_data() {
                    state.write_usize(*v);
                }
            }
            ReprRef::Inline(v) => {
                state.write_usize(v.len());
                state.write_usize(v.data());
            }
        }
    }
}

impl Drop for BitList {
    fn drop(&mut self) {
        if !self.is_inline() {
            unsafe {
                drop(transmute::<NonZeroPtr, HeapBitList>(self.val));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::prelude::*;

    #[test]
    fn test_list_eq() {
        let rng = &mut StdRng::seed_from_u64(1234566543);
        for _ in 0..1000 {
            let to_push = rng.random_range(0..100);
            let to_pop = rng.random_range(0..100);
            let mut list = BitList::NO_BITS;
            for _ in 0..to_push {
                list.push_bit(rng.random_bool(0.5));
            }
            for _ in 0..to_pop {
                list.pop_bit();
            }
            let bits = list.iter().collect::<Vec<_>>();
            let cmp_list = BitList::from_bits(bits.iter().copied());
            assert_eq!(list, cmp_list);
        }
    }
}
