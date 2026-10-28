#![allow(unsafe_code)]
#![allow(unused)]
#![doc = include_str!("../README.md")]

mod basic_math;
mod convert;
mod heap;
mod inline;
mod iter;
mod ops;
mod util;
mod wrapper;

use std::convert::Infallible;

pub use basic_math::MathError;
pub use inline::InlineBitList;
pub use iter::*;
pub use ops::AllocateError;
pub use wrapper::BitList;

pub trait BitWrite {
    type Error;

    fn write(&mut self, bits: BitsIter<'_>) -> Result<usize, Self::Error>;

    fn flush(&mut self) -> Result<(), Self::Error>;
}

impl BitWrite for BitList {
    type Error = Infallible;

    fn write(&mut self, bits: BitsIter<'_>) -> Result<usize, Self::Error> {
        let len = bits.len();
        self.push_bits(bits);
        Ok(len)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
