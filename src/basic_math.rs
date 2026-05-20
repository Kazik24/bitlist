use crate::wrapper::BitList;
use std::ops::*;

impl BitList {
    pub fn assign_or(&mut self, other: &Self) {
        self.assign_for_each_carry(other, (), |_, a, b| *a = *a | b);
    }
    pub fn assign_and(&mut self, other: &Self) {
        self.assign_for_each_carry(other, (), |_, a, b| *a = *a & b);
    }
    pub fn assign_xor(&mut self, other: &Self) {
        self.assign_for_each_carry(other, (), |_, a, b| *a = *a ^ b);
    }
    pub fn assign_not(&mut self) {
        self.for_each_carry((), |_, v| *v = !*v)
    }

    pub fn assign_neg(&mut self) {
        self.for_each_carry((), |_, v| *v = !*v)
    }

    pub fn assign_add_overflow(&mut self, other: &Self) -> bool {
        let (o, l) = self.assign_for_each_carry(other, false, |carry, a, b| {
            let (res, c1) = a.overflowing_add(b);
            (*a, *carry) = res.overflowing_add(*carry as _);
            *carry |= c1;
        });
        o || l != 0
    }

    pub fn wrapping_add(&self, other: &Self) -> Option<Self> {
        if self.len() != other.len() {
            return None; // bit size difference
        }
        let mut res = self.clone();
        res.assign_add_overflow(other);
        Some(res)
    }

    pub fn checked_add(&self, other: &Self) -> Result<Self, MathError> {
        if self.len() != other.len() {
            return Err(MathError::BitSizeDifference);
        }
        let mut res = self.clone();
        if res.assign_add_overflow(other) {
            return Err(MathError::Overflow);
        }
        Ok(res)
    }

    pub fn wrapping_sub(&self, other: &Self) -> Option<Self> {
        if self.len() != other.len() {
            return None; // bit size difference
        }
        let mut res = self.clone();
        res.assign_sub_overflow(other);
        Some(res)
    }

    pub fn checked_sub(&self, other: &Self) -> Result<Self, MathError> {
        if self.len() != other.len() {
            return Err(MathError::BitSizeDifference);
        }
        let mut res = self.clone();
        if res.assign_sub_overflow(other) {
            return Err(MathError::Overflow);
        }
        Ok(res)
    }

    pub fn assign_sub_overflow(&mut self, other: &Self) -> bool {
        let (o, l) = self.assign_for_each_carry(other, false, |carry, a, b| {
            let (res, c1) = a.overflowing_sub(b);
            (*a, *carry) = res.overflowing_sub(*carry as _);
            *carry |= c1;
        });
        o || l != 0
    }

    pub fn assign_umul_scalar(&mut self, scalar: u32) {
        let scalar = scalar as usize;
        self.for_each_carry(0usize, |carry, a| {
            (*a, *carry) = carrying_mul(*a, scalar, *carry);
        });
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MathError {
    Overflow,
    BitSizeDifference,
}

fn carrying_mul(a: usize, b: usize, carry: usize) -> (usize, usize) {
    let result = (a as u128) * (b as u128) + (carry as u128);
    (result as usize, (result >> usize::BITS) as usize)
}

macro_rules! impl_op {
    ($op_trait:ident | $assign:ident < $for_ty:ident > fn $func:ident | $func_ass:ident($lhs:ident, $rhs:ident) $code:block) => {
        impl $op_trait<&$for_ty> for &$for_ty {
            type Output = $for_ty;
            fn $func(self, rhs: &$for_ty) -> Self::Output {
                let mut $lhs = self.clone();
                let $rhs = rhs;
                $code
                $lhs
            }
        }
        impl $op_trait<&$for_ty> for $for_ty {
            type Output = $for_ty;
            fn $func(self, rhs: &$for_ty) -> Self::Output {
                let mut $lhs = self;
                let $rhs = rhs;
                $code
                $lhs
            }
        }
        impl $op_trait<$for_ty> for &$for_ty {
            type Output = $for_ty;
            fn $func(self, rhs: $for_ty) -> Self::Output {
                let mut $lhs = self.clone();
                let $rhs = &rhs;
                $code
                $lhs
            }
        }
        impl $op_trait<$for_ty> for $for_ty {
            type Output = $for_ty;
            fn $func(self, rhs: $for_ty) -> Self::Output {
                let mut $lhs = self;
                let $rhs = &rhs;
                $code
                $lhs
            }
        }
        impl $assign<$for_ty> for $for_ty {
            fn $func_ass(&mut self, rhs: Self) {
                let $lhs = self;
                let $rhs = &rhs;
                $code
            }
        }
        impl $assign<&$for_ty> for $for_ty {
            fn $func_ass(&mut self, rhs: &Self) {
                let $lhs = self;
                let $rhs = rhs;
                $code
            }
        }
    }
}

impl_op! {
    Add|AddAssign<BitList> fn add|add_assign(a,b) {
        let overflow = a.assign_add_overflow(b);
        debug_assert!(!overflow, "Overflow occurred");
    }
}
impl_op! {
    Sub|SubAssign<BitList> fn sub|sub_assign(a,b) {
        let overflow = a.assign_sub_overflow(b);
        debug_assert!(!overflow, "Overflow occurred");
    }
}
impl_op! {
    BitAnd|BitAndAssign<BitList> fn bitand|bitand_assign(a,b) {
        a.assign_and(b);
    }
}
impl_op! {
    BitOr|BitOrAssign<BitList> fn bitor|bitor_assign(a,b) {
        a.assign_or(b);
    }
}
impl_op! {
    BitXor|BitXorAssign<BitList> fn bitxor|bitxor_assign(a,b) {
        a.assign_xor(b);
    }
}

impl Not for BitList {
    type Output = BitList;
    fn not(self) -> Self::Output {
        let mut val = self.clone();
        val.assign_not();
        val
    }
}
impl Not for &BitList {
    type Output = BitList;
    fn not(self) -> Self::Output {
        let mut val = self.clone();
        val.assign_not();
        val
    }
}

#[cfg(test)]
mod tests {
    use super::BitList;
    use crate::basic_math::MathError;

    #[test]
    fn test_checked_add() {
        let a = BitList::lit("1110");
        let b = BitList::lit("0011");
        assert_eq!(a.wrapping_add(&b).unwrap().first_word(), 0b0001);
        let e = a.checked_add(&b).unwrap_err();
        assert_eq!(e, MathError::Overflow);
    }

    #[test]
    fn test_checked_sub() {
        let a = BitList::lit("0001");
        let b = BitList::lit("0011");
        assert_eq!(a.wrapping_sub(&b).unwrap().first_word(), 0b1110);
        let e = a.checked_sub(&b).unwrap_err();
        assert_eq!(e, MathError::Overflow);
    }
}
