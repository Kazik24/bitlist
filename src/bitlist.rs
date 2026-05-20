#![allow(unused)]

use std::cmp::{Ordering, min};
use std::convert::TryInto;
use std::fmt::{Debug, Display, Formatter};
use std::io::Result;
use std::io::{Read, Write};
use std::mem::size_of;
use std::num::Wrapping;
use std::ops::Not;

#[derive(Clone, Eq)]
pub struct BitList {
    list: Vec<Chunk>,
    count: u32,
}

type Chunk = u64;

fn bits_to_bytes(value: u32) -> u32 {
    value / 8 + if value % 8 != 0 { 1 } else { 0 }
}

impl BitList {
    pub const NO_BITS: BitList = BitList { list: Vec::new(), count: 0 };
    const CHUNK_SIZE: u32 = Chunk::BITS as _;
    pub fn single(value: bool) -> Self {
        if value { Self::ones(1) } else { Self::zeros(1) }
    }
    pub fn zeros(size: u32) -> Self {
        Self { list: vec![0; bits_to_bytes(size) as usize], count: size }
    }
    pub fn ones(size: u32) -> Self {
        let mut v = Self { list: vec![Chunk::MAX; bits_to_bytes(size) as usize], count: size };
        if let Some((l, m)) = v.last_with_mask_mut() {
            *l &= m;
        }
        v
    }
    pub fn set_single(&mut self, value: bool) {
        self.list.clear();
        self.count = 0;
        self.push(value as _); //0 or 1
        self.count = 1;
    }
    pub const fn new() -> Self {
        Self { list: Vec::new(), count: 0 }
    }
    pub fn bits(text: &str) -> Self {
        let mut list = Self::new();
        for c in text.chars().rev() {
            match c {
                '0' => list.push(false),
                '1' => list.push(true),
                c => panic!("Unknown character '{}'", c),
            }
        }
        list
    }
    pub fn from_trunc_128(value: u128, size: u32) -> Self {
        let value = if size > 128 { value } else { value & ((1 << size) - 1) };
        let bytes = value.to_le_bytes();
        let list = bytes
            .chunks((Self::CHUNK_SIZE / 8) as _)
            .map(|a| Chunk::from_le_bytes(a.try_into().unwrap()))
            .collect::<Vec<_>>();
        Self { list, count: size }
    }
    pub fn from_trunc_u64(value: u64, size: u32) -> Self {
        let value = if size > 64 { value } else { value & ((1 << size) - 1) };
        let bytes = value.to_le_bytes();
        let list = bytes
            .chunks((Self::CHUNK_SIZE / 8) as _)
            .map(|a| Chunk::from_le_bytes(a.try_into().unwrap()))
            .collect::<Vec<_>>();
        Self { list, count: size }
    }
    pub fn to_u64(&self) -> Option<u64> {
        if self.count > 64 {
            return None;
        }
        let mut array = [0u8; size_of::<u64>()];
        let list = self.list.iter().flat_map(|v| v.to_le_bytes());
        array.iter_mut().zip(list).for_each(|(a, b)| {
            *a = b;
        });
        Some(u64::from_le_bytes(array))
    }
    pub fn push(&mut self, bit: bool) {
        let pos = self.count;
        self.count += 1;
        if pos % Self::CHUNK_SIZE == 0 {
            self.list.push(bit as _);
        } else {
            self.set_bit(pos as _, bit);
        }
    }
    pub fn set_bit(&mut self, index: usize, bit: bool) {
        if index >= self.count as _ {
            panic!("Bit index out of bounds {}, size: {}.", index, self.count);
        }
        let w = &mut self.list[index / Self::CHUNK_SIZE as usize];
        let flag = (1 << (index % Self::CHUNK_SIZE as usize));
        if bit {
            *w |= flag;
        } else {
            *w &= !flag;
        }
    }
    pub fn get_bit(&self, index: usize) -> bool {
        if index >= self.count as _ {
            panic!("Bit index out of bounds {}, size: {}.", index, self.count);
        }
        let word = self.list[index / Self::CHUNK_SIZE as usize];
        word & (1 << (index % Self::CHUNK_SIZE as usize)) != 0
    }
    pub fn last_bit(&self) -> Option<bool> {
        self.list.last().map(|&last| last & (1 << ((self.count - 1) % Self::CHUNK_SIZE)) != 0)
    }
    pub fn to_i64(&self) -> Option<i64> {
        if self.count > 64 {
            return None;
        }
        let mut array = [0u8; size_of::<u64>()];
        let list = self.list.iter().flat_map(|v| v.to_le_bytes());
        array.iter_mut().zip(list).for_each(|(a, b)| {
            *a = b;
        });
        let u = u64::from_le_bytes(array);
        Some(if self.last_bit().unwrap_or(false) { (u | !((1 << self.count) - 1)) as _ } else { u as _ })
    }
    fn last_with_mask(&self) -> Option<(Chunk, Chunk)> {
        self.list.last().map(|last| {
            let extra = self.count % Self::CHUNK_SIZE;
            (*last, (1 << extra) - 1)
        })
    }
    fn last_with_mask_mut(&mut self) -> Option<(&mut Chunk, Chunk)> {
        let cnt = self.count;
        self.list.last_mut().map(|last| {
            let extra = cnt % Self::CHUNK_SIZE;
            (last, (1 << extra) - 1)
        })
    }
    fn last_word_mask(&self) -> Chunk {
        self.last_with_mask().map(|(_, v)| v).unwrap_or(0)
    }

    pub fn is_signed(&self) -> bool {
        self.count & (1 << 31) != 0
    }
    pub fn len(&self) -> usize {
        self.bit_count() as usize
    }
    fn bit_count(&self) -> u32 {
        self.count & ((1 << 31) - 1)
    }

    pub fn add_assign(&mut self, other: &Self) {
        let mut prev_ov = false;
        for (a, b) in self.list.iter_mut().zip(other.list.iter().copied()) {
            let (res, ov1) = a.overflowing_add(prev_ov as _);
            let (res, ov2) = res.overflowing_add(b);
            *a = res;
            prev_ov = ov1 | ov2;
        }
        if let Some((l, m)) = self.last_with_mask_mut() {
            *l &= m;
        }
    }
    pub fn sub_assign(&mut self, other: &Self) {
        let mut prev_ov = false;
        for (a, b) in self.list.iter_mut().zip(other.list.iter().copied()) {
            let (res, ov1) = a.overflowing_sub(prev_ov as _);
            let (res, ov2) = res.overflowing_sub(b);
            *a = res;
            prev_ov = ov1 | ov2;
        }
        if let Some((l, m)) = self.last_with_mask_mut() {
            *l &= m;
        }
    }
    pub fn truncate(&mut self, len: usize) {
        if self.count < len as _ {
            self.count = len as _;
        }
        self.list.truncate(bits_to_bytes(len as _) as _);
        if let Some((l, m)) = self.last_with_mask_mut() {
            *l &= m;
        }
    }

    pub fn push_list(&mut self, other: &Self) {
        let b = self.count % Self::CHUNK_SIZE;
        self.count += other.count;

        if b == 0 {
            self.list.extend_from_slice(&other.list);
        } else {
            self.list.reserve(other.list.len());

            for block in other.list.iter().copied() {
                let last = self.list.last_mut().unwrap();
                *last |= block << b;
                self.list.push(block >> (Self::CHUNK_SIZE - b));
            }
            let o = other.count % Self::CHUNK_SIZE;
            let overflow = (b + o > Self::CHUNK_SIZE) || (o == 0 && b != 0);
            if !overflow {
                // Remove additional block
                self.list.pop();
            }
        }
    }

    pub fn repeat(&mut self, times: u32) {
        let val = self.clone();
        self.list.reserve((val.list.len() as u32 * times) as usize);
        for _ in 0..times {
            self.push_list(&val);
        }
    }

    pub fn write<W: Write>(&self, write: &mut W) -> Result<()> {
        write.write_all(&self.count.to_be_bytes())?;
        let mut bytes = bits_to_bytes(self.bit_count()) as usize;
        for chunk in self.list.as_slice().iter().copied() {
            let size = min(bytes, size_of::<Chunk>());
            write.write_all(&chunk.to_le_bytes()[..size])?;
            bytes -= size;
            if bytes == 0 {
                break;
            }
        }
        Ok(())
    }
    pub fn read<R: Read>(read: &mut R) -> Result<Self> {
        let count = {
            let mut data = [0u8; 4];
            read.read_exact(&mut data)?;
            u32::from_be_bytes(data)
        };
        let mut bytes = bits_to_bytes(count & ((1 << 31) - 1)) as usize;
        let chunks = bytes / size_of::<Chunk>() + if bytes % size_of::<Chunk>() != 0 { 1 } else { 0 };
        let mut list = Vec::with_capacity(chunks);
        while bytes != 0 {
            let mut data = [0u8; size_of::<Chunk>()];
            let size = min(bytes, size_of::<Chunk>());
            read.read_exact(&mut data[..size])?;
            list.push(Chunk::from_le_bytes(data));
            bytes -= size;
        }
        Ok(Self { list, count })
    }
    pub fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.count != other.count {
            return None;
        }
        if self.count == 0 {
            return Some(Ordering::Equal);
        }
        debug_assert_eq!(self.list.len(), other.list.len());
        Some(self.list.iter().rev().zip(other.list.iter().rev()).fold(Ordering::Equal, |o, (a, b)| o.then(a.cmp(b))))
    }
    pub fn signed_partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.count != other.count {
            return None;
        }
        if self.count == 0 {
            return Some(Ordering::Equal);
        }
        let len = self.list.len();
        debug_assert!(len > 0 && len == other.list.len());
        let last = len - 1;
        let init = (self.list[last] as i64).cmp(&(other.list[last] as i64));
        Some(self.list[..last].iter().zip(other.list[..last].iter()).fold(init, |o, (a, b)| o.then(a.cmp(b))))
    }
}
impl PartialEq for BitList {
    fn eq(&self, other: &Self) -> bool {
        if self.count != other.count {
            return false;
        }
        if self.count == 0 {
            return true;
        }
        debug_assert_eq!(self.list.len(), other.list.len());
        self.list == other.list
    }
}
impl Debug for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitList[{}]", self)
    }
}
impl Display for BitList {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.count == 0 {
            write!(f, "_")
        } else {
            if self.count > 64 {
                write!(f, "..")?;
            }
            let mut val = *self.list.first().unwrap();
            if self.count < 64 {
                val &= (1 << self.count) - 1;
            }
            write!(f, "{:0width$b}", val, width = self.count as usize)
        }
    }
}

impl Not for BitList {
    type Output = Self;

    fn not(mut self) -> Self::Output {
        self.list.iter_mut().for_each(|v| *v = !*v);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_add_assign() {
        for a in (0u64..1024).map(|v| v * 128) {
            let al = BitList::from_trunc_u64(a, 24);
            for b in (0u64..1024).map(|v| v * 128) {
                let exp = b.wrapping_add(a);
                let mut data = BitList::from_trunc_u64(b, 24);
                data.add_assign(&al);
                let res = data.to_u64().unwrap();
                assert_eq!(res, exp as _);
            }
        }
    }
    #[test]
    fn test_sub_assign() {
        for a in (0u64..1024).map(|v| v * 128) {
            let al = BitList::from_trunc_u64(a, 24);
            for b in (0u64..1024).map(|v| v * 128) {
                let exp = b.wrapping_sub(a);
                let mut data = BitList::from_trunc_u64(b, 24);
                data.sub_assign(&al);
                let res = data.to_i64().unwrap() as u64;
                assert_eq!(res, exp);
            }
        }
    }
    #[test]
    fn test_concat() {
        let mut v1 = BitList::bits("110101");
        v1.push_list(&BitList::bits("100101000"));
        assert_eq!(v1, BitList::bits("100101000110101"));
        let mut v1 = BitList::bits("0000000011111111000000001111111100000000111111110000000011111111");
        v1.push_list(&BitList::bits("100"));
        assert_eq!(v1, BitList::bits("1000000000011111111000000001111111100000000111111110000000011111111"));
        let mut v1 = BitList::bits("000000011111111000000001111111100000000111111110000000011111111");
        v1.push_list(&BitList::bits("101"));
        assert_eq!(v1, BitList::bits("101000000011111111000000001111111100000000111111110000000011111111"));
    }

    #[test]
    #[ignore = "this is old bitlist functionality, soon to be removed"]
    fn test_cmp() {
        fn comp(a: i32, b: i32) -> Ordering {
            let a = a.to_be_bytes();
            let b = b.to_be_bytes();
            a[1..].iter().zip(b[1..].iter()).fold((a[0] as i8).cmp(&(b[0] as i8)), |o, (a, b)| o.then(a.cmp(b)))
        }

        for a in -1024i64..1024 {
            let a = a * 12456;
            let al = BitList::from_trunc_128(a as i128 as u128, 100);
            for b in -1024i64..1024 {
                let b = b * 54113;
                let bl = BitList::from_trunc_128(b as i128 as u128, 100);
                let exp = a.cmp(&b);
                let res = al.signed_partial_cmp(&bl).unwrap();
                assert_eq!(exp, res, "a: {}={:04X}, b: {}={:04X}", a, a, b, b);
            }
        }
    }
}
