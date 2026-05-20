#![allow(unsafe_code)]
#![allow(unused)]
#![doc = include_str!("../README.md")]

mod basic_math;
mod bitlist;
mod convert;
mod heap;
mod inline;
mod ops;
//mod ptr_bitlist;
mod iter;
mod util;
mod wrapper;

//pub use bitlist::BitList;
pub use basic_math::MathError;
pub use inline::InlineBitList;
pub use iter::*;
pub use ops::AllocateError;
pub use wrapper::BitList;
