use std::ptr::NonNull;

use crate::{
    BitsIter, WordBits,
    heap::{bit_in_word_index, last_word_mask, word_index, words_for},
};
macro_rules! arr {
    ($cnt:literal, mut $exp:expr) => {
        <&mut [_; $cnt] as TryFrom<&mut [_]>>::try_from($exp).unwrap()
    };
}

pub fn unary_for_each_carry<A, T>(value: &mut [A], init: T, mut func: impl FnMut(&mut T, &mut A)) -> T {
    // SAFETY: this is a slice of ZST so any pointer in it is sufficient
    let dummy = unsafe { std::slice::from_raw_parts(NonNull::<()>::dangling().as_ptr(), value.len()) };
    for_each_carry(value, dummy, init, move |c, v, _| func(c, v))
}

pub fn for_each_carry<A, B: Copy, T>(
    value: &mut [A],
    other: &[B],
    init: T,
    mut func: impl FnMut(&mut T, &mut A, B),
) -> T {
    assert_eq!(value.len(), other.len());
    match value.len() {
        0 => init,
        1 => for_each_carry_const(arr!(1, mut value), other.try_into().unwrap(), init, func),
        2 => for_each_carry_const(arr!(2, mut value), other.try_into().unwrap(), init, func),
        3 => for_each_carry_const(arr!(3, mut value), other.try_into().unwrap(), init, func),
        4 => for_each_carry_const(arr!(4, mut value), other.try_into().unwrap(), init, func),
        5 => for_each_carry_const(arr!(5, mut value), other.try_into().unwrap(), init, func),
        6 => for_each_carry_const(arr!(6, mut value), other.try_into().unwrap(), init, func),
        7 => for_each_carry_const(arr!(7, mut value), other.try_into().unwrap(), init, func),
        8 => for_each_carry_const(arr!(8, mut value), other.try_into().unwrap(), init, func),
        9 => for_each_carry_const(arr!(9, mut value), other.try_into().unwrap(), init, func),
        10 => for_each_carry_const(arr!(10, mut value), other.try_into().unwrap(), init, func),
        _ => {
            let mut carry = init;
            for (a, b) in value.iter_mut().zip(other) {
                func(&mut carry, a, *b);
            }
            carry
        }
    }
}

fn for_each_carry_const<A, B: Copy, T, const N: usize>(
    value: &mut [A; N],
    other: &[B; N],
    init: T,
    mut func: impl FnMut(&mut T, &mut A, B),
) -> T {
    let mut carry = init;
    for (a, b) in value.iter_mut().zip(other) {
        func(&mut carry, a, *b);
    }
    carry
}

/// Copy non-overlapping bit ranges from `src` to `dst`
/// # Safety
/// Caller must ensure that:
/// - src pointer is initialized and valid for reads of at least `self.len()` bits where start (`src_bit_offset`) is rounded to previous
///   word boundary and end is rounded to next word boundary
/// - dst pointer is valid for writes of at least `self.len()` bits, where start (`dst_bit_offset`) is rounded to previous word boundary
///   and end is rounded to next word boundary.
/// - src and dst are not overlapping regarding their bit ranges (they can overlap in words)
/// - if `dst_bit_offset` is not aligned to word boundary, caller must ensure that first word in `dst` with bit
///   offset of `dst_bit_offset` is initialized
/// - if `dst_bit_offset + self.len()` is not aligned to word boundary, caller must ensure that last word in `dst`
///   with bit offset of `dst_bit_offset + self.len()` is initialized
pub const unsafe fn copy_bits_nonoverlapping(
    src: *const usize,
    src_bit_offset: usize,
    dst: *mut usize,
    dst_bit_offset: usize,
    bit_count: usize,
) {
    if bit_count == 0 {
        return;
    }
    unsafe {
        let src = src.add(word_index(src_bit_offset));
        let src_bit_offset = bit_in_word_index(src_bit_offset);
        let dst = dst.add(word_index(dst_bit_offset));
        let dst_bit_offset = bit_in_word_index(dst_bit_offset);
        if src_bit_offset == 0 && dst_bit_offset == 0 {
            // start of both regions is aligned to word
            let whole_words_len = word_index(bit_count);
            std::ptr::copy_nonoverlapping(src, dst, whole_words_len);
            if bit_in_word_index(bit_count) != 0 {
                // last word is not full
                let mask = last_word_mask(bit_count);
                let last_word_masked = src.add(whole_words_len).read() & mask;
                let dst_word_ptr = dst.add(whole_words_len);
                let new_dst = (dst_word_ptr.read() & !mask) | last_word_masked;
                dst_word_ptr.write(new_dst);
            }
            return;
        } else if bit_in_byte_index(src_bit_offset) == 0 && bit_in_byte_index(dst_bit_offset) == 0 {
            // convert offsets to bytrs
            let src = src.cast::<u8>().add(byte_index(src_bit_offset));
            let dst = dst.cast::<u8>().add(byte_index(dst_bit_offset));
            // start of both regions is aligned to byte, use normal memcopy
            let whole_bytes_len = byte_index(bit_count);
            std::ptr::copy_nonoverlapping(src, dst, whole_bytes_len);
            if bit_in_byte_index(bit_count) != 0 {
                // last byte is not full
                let mask = last_byte_mask(bit_count);
                let last_byte_masked = src.add(whole_bytes_len).read() & mask;
                let dst_byte_ptr = dst.add(whole_bytes_len);
                let new_dst = (dst_byte_ptr.read() & !mask) | last_byte_masked;
                dst_byte_ptr.write(new_dst);
            }
            return;
        }

        debug_assert!(src_bit_offset < WordBits::BITS as _);
        debug_assert!(dst_bit_offset < WordBits::BITS as _);

        let mut iter = BitsIter::new_unchecked(src, src_bit_offset, bit_count);

        // fill start word

        // read_acc = read_current.push_overflowing(read_acc);

        let mut src = src;
        let mut dst = dst;
        let mut bit_count = bit_count;

        // write first unaligned word, and keep the rest in acc
        if dst_bit_offset != 0 {
            let first_word = dst.read();
            let mut dst_word = WordBits::new_unchecked(first_word, dst_bit_offset as _);
            let remaining = WordBits::BITS as usize - dst_word.len();
            let w = iter.next_bits(remaining);
            dst_word.push_bits(w);
            if !dst_word.is_full() {
                // iterator ended without completing first word
                debug_assert!(iter.is_empty());
                dst.write(dst_word.raw() | (first_word & !last_word_mask(dst_word.len())));
                return;
            } else {
                // first word completed
                dst.write(dst_word.raw());
                dst = dst.add(1);
            }
        }

        loop {
            let next_word = iter.next_unaligned_word();
            if !next_word.is_full() {
                if !next_word.is_empty() {
                    // combine masked last word with the remaining bits and write
                    let masked_last_word = dst.read() & !last_word_mask(next_word.len());
                    dst.write(next_word.raw() | masked_last_word);
                }
                break;
            }
            dst.write(next_word.raw());
            dst = dst.add(1);
        }
    }
}

pub unsafe fn fill_bits(dst: *mut usize, dst_bit_offset: usize, bit_count: usize, value: bool) {
    if bit_count == 0 {
        return;
    }
    unsafe {
        let dst = dst.cast::<u8>().add(byte_index(dst_bit_offset));
        let dst_bit_offset = bit_in_byte_index(dst_bit_offset);
        if dst_bit_offset == 0 {
            // start of both regions is aligned to byte, use normal memcopy
            let whole_bytes_len = byte_index(bit_count);
            dst.write_bytes(if value { 0xff } else { 0 }, whole_bytes_len);
            if bit_in_byte_index(bit_count) != 0 {
                // last byte is not full
                let mask = last_byte_mask(bit_count);
                let dst_byte_ptr = dst.add(whole_bytes_len);
                let last_byte_masked = dst_byte_ptr.read();
                let new_byte = if value { last_byte_masked | mask } else { last_byte_masked & !mask };
                dst_byte_ptr.write(new_byte);
            }
            return;
        }
        let first_byte = dst.read();
        let mut end = dst_bit_offset + bit_count;
        println!("first_byte: {first_byte:08b}, dst_bit_offset: {dst_bit_offset}, end: {end}");
        if end < 8 {
            let mask = !last_byte_mask(dst_bit_offset + 1) & last_byte_mask(end);
            println!("mask: {:08b}", mask);
            let new_byte = if value { first_byte | mask } else { first_byte & !mask };
            dst.write(new_byte);
            return;
        } else {
            let mask = last_byte_mask(dst_bit_offset + 1);
            println!("mask rest: {:08b}", mask);
            let new_byte = if value { first_byte | mask } else { first_byte & !mask };
            dst.write(new_byte);
        }
        let dst = dst.add(1);
        end -= (7 - dst_bit_offset);
        let whole_bytes_len = byte_index(end);
        dst.write_bytes(if value { 0xff } else { 0 }, whole_bytes_len);
        println!("whole_bytes_len: {whole_bytes_len}, end: {end}");
        if bit_in_byte_index(end) != 0 {
            let mask = last_byte_mask(end);
            println!("mask: {:08b}", mask);
            let dst_byte_ptr = dst.add(whole_bytes_len);
            let new_byte = if value { dst_byte_ptr.read() | mask } else { dst_byte_ptr.read() & !mask };
            dst_byte_ptr.write(new_byte);
        }
    }
}

#[inline]
const fn byte_index(bit_index: usize) -> usize {
    const SHIFT: u32 = (8u32 - 1).count_ones();
    bit_index.wrapping_shr(SHIFT)
}
#[inline]
const fn bit_in_byte_index(bit_index: usize) -> usize {
    bit_index & (8 - 1)
}
#[inline]
const fn last_byte_mask(len: usize) -> u8 {
    let shift = bit_in_byte_index(len);
    if shift == 0 && len != 0 {
        return u8::MAX;
    }
    (1u8.wrapping_shl(shift as _)) - 1
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rand::prelude::*;

    use crate::{BitList, WordBits};

    use super::*;

    #[test]
    fn test_copy_bits_nonoverlapping_byte_aligned() {
        const MEM: usize = 1024;
        let src = &mut [0usize; MEM];
        let dst = &mut [0usize; MEM];
        let dst_snapshot = &mut [0usize; MEM];

        let rng = &mut StdRng::seed_from_u64(654321);

        unsafe {
            let mut time = Duration::ZERO;
            let mut total_bits = 0;
            let mut counter = 0;
            for i in 0..1000 {
                let (src_off, dst_off) = loop {
                    let src_off = rng.random_range(0..500);
                    let dst_off = rng.random_range(0..500);
                    // filter only byte ranges
                    //if bit_in_byte_index(src_off) == 0 && bit_in_byte_index(dst_off) == 0 {
                    break (src_off, dst_off);
                    //}
                };

                let src_ptr_off = rng.random_range(0..500);
                let dst_ptr_off = rng.random_range(0..500);
                let src_ptr = src.as_ptr().add(src_ptr_off);
                let dst_ptr = dst.as_mut_ptr().add(dst_ptr_off);
                let bit_len = rng.random_range(8000..1024 * 30);
                rng.fill_bytes(src.align_to_mut::<u8>().1);
                rng.fill_bytes(dst.align_to_mut::<u8>().1);
                dst_snapshot.copy_from_slice(dst);

                // SAFETY: all generate values should be in bounds of src and dst memory regions

                let start = Instant::now();
                copy_bits_nonoverlapping(src_ptr, src_off, dst_ptr, dst_off, bit_len);
                time += start.elapsed();
                total_bits += bit_len;
                counter += 1;

                // check that regions are equal
                let src_region = BitsIter::new_unchecked(src_ptr, src_off, bit_len);
                let dst_region = BitsIter::new_unchecked(dst_ptr, dst_off, bit_len);
                // println!("src: {:?}", src_region.copy().to_list());
                // println!("dst: {:?}", dst_region.copy().to_list());
                let equal = src_region.eq(dst_region);
                assert!(
                    equal,
                    "{i} Bits are not equal - src_ptr_off: {src_ptr_off}, dst_ptr_off: {dst_ptr_off}, src_off: {src_off}, dst_off: {dst_off}, bit_len: {bit_len}"
                );

                // check that regions outside are unchanged
                let dst_ptr_bit_off = dst_ptr_off * WordBits::BITS as usize;
                let s1 = BitsIter::new_unchecked(dst.as_ptr(), 0, dst_ptr_bit_off + dst_off);
                let d1 = BitsIter::new_unchecked(dst_snapshot.as_ptr(), 0, dst_ptr_bit_off + dst_off);
                let soff = dst_ptr_bit_off + dst_off + bit_len;
                let s2 = BitsIter::new_unchecked_range(dst.as_ptr(), soff..(MEM * WordBits::BITS as usize));
                let d2 = BitsIter::new_unchecked_range(dst_snapshot.as_ptr(), soff..(MEM * WordBits::BITS as usize));
                assert!(s1.eq(d1), "{i} s1 != d1");
                assert!(s2.eq(d2), "{i} s2 != d2");
            }
            println!("Total time: {time:.03?}, total bits: {total_bits}, counter: {counter}");
            let avg_bits = total_bits as f64 / counter as f64;
            let words = avg_bits as f64 / 64.0;
            let bytes = avg_bits as f64 / 8.0;
            println!(
                "Average time: {:.03?}, average bits: {avg_bits:.03} = {words:.03}W = {bytes:.03}B",
                time / counter
            );
        }
    }

    #[test]
    fn test_fill_bits_byte_aligned() {
        const MEM: usize = 64;
        let dst = &mut [0usize; MEM];
        let dst_snapshot = &mut [0usize; MEM];

        let rng = &mut StdRng::seed_from_u64(654321);

        unsafe {
            let mut time = Duration::ZERO;
            let mut total_bits = 0;
            let mut counter = 0;
            for i in 0..1000 {
                let dst_off = loop {
                    let dst_off = rng.random_range(0..128);
                    // filter only byte ranges
                    //if bit_in_byte_index(src_off) == 0 && bit_in_byte_index(dst_off) == 0 {
                    break dst_off;
                    //}
                };

                let dst_ptr_off = rng.random_range(0..9);
                let dst_ptr = dst.as_mut_ptr().add(dst_ptr_off);
                let bit_len = rng.random_range(0..130);
                rng.fill_bytes(dst.align_to_mut::<u8>().1);
                dst_snapshot.copy_from_slice(dst);
                let value = rng.random_bool(0.5);

                // SAFETY: all generate values should be in bounds of src and dst memory regions

                let start = Instant::now();
                fill_bits(dst_ptr, dst_off, bit_len, value);
                time += start.elapsed();
                total_bits += bit_len;
                counter += 1;

                // check that regions are equal
                let dst_region = BitsIter::new_unchecked(dst_ptr, dst_off, bit_len);
                // println!("src: {:?}", src_region.copy().to_list());
                println!("dst: {:?}", dst_region.copy().to_list());
                let equal = dst_region.all_value() == Some(value);
                assert!(
                    equal,
                    "{i} Bits are not equal - dst_ptr_off: {dst_ptr_off}, dst_off: {dst_off}, bit_len: {bit_len}"
                );

                // check that regions outside are unchanged
                let dst_ptr_bit_off = dst_ptr_off * WordBits::BITS as usize;
                let s1 = BitsIter::new_unchecked(dst.as_ptr(), 0, dst_ptr_bit_off + dst_off);
                let d1 = BitsIter::new_unchecked(dst_snapshot.as_ptr(), 0, dst_ptr_bit_off + dst_off);
                let soff = dst_ptr_bit_off + dst_off + bit_len;
                let s2 = BitsIter::new_unchecked_range(dst.as_ptr(), soff..(MEM * WordBits::BITS as usize));
                let d2 = BitsIter::new_unchecked_range(dst_snapshot.as_ptr(), soff..(MEM * WordBits::BITS as usize));
                println!("s1: {:b}", s1.copy().to_list());
                println!("d1: {:b}", d1.copy().to_list());
                assert!(s1.eq(d1), "{i} s1 != d1");
                assert!(s2.eq(d2), "{i} s2 != d2");
            }
            println!("Total time: {time:.03?}, total bits: {total_bits}, counter: {counter}");
            let avg_bits = total_bits as f64 / counter as f64;
            let words = avg_bits as f64 / 64.0;
            let bytes = avg_bits as f64 / 8.0;
            println!(
                "Average time: {:.03?}, average bits: {avg_bits:.03} = {words:.03}W = {bytes:.03}B",
                time / counter
            );
        }
    }
}

/*
/// Three argument multiply accumulate:
/// acc += b * c
#[allow(clippy::many_single_char_names)]
fn mac3(mut acc: &mut [usize], mut b: &[usize], mut c: &[usize]) {
    // Least-significant zeros have no effect on the output.
    if let Some(&0) = b.first() {
        if let Some(nz) = b.iter().position(|&d| d != 0) {
            b = &b[nz..];
            acc = &mut acc[nz..];
        } else {
            return;
        }
    }
    if let Some(&0) = c.first() {
        if let Some(nz) = c.iter().position(|&d| d != 0) {
            c = &c[nz..];
            acc = &mut acc[nz..];
        } else {
            return;
        }
    }

    let acc = acc;
    let (x, y) = if b.len() < c.len() { (b, c) } else { (c, b) };

    // We use three algorithms for different input sizes.
    //
    // - For small inputs, long multiplication is fastest.
    // - Next we use Karatsuba multiplication (Toom-2), which we have optimized
    //   to avoid unnecessary allocations for intermediate values.
    // - For the largest inputs we use Toom-3, which better optimizes the
    //   number of operations, but uses more temporary allocations.
    //
    // The thresholds are somewhat arbitrary, chosen by evaluating the results
    // of `cargo bench --bench bigint multiply`.

    if x.len() <= 32 {
        // Long multiplication:
        for (i, xi) in x.iter().enumerate() {
            mac_digit(&mut acc[i..], y, *xi);
        }
    } else if x.len() <= 256 {
        // Karatsuba multiplication:
        //
        // The idea is that we break x and y up into two smaller numbers that each have about half
        // as many digits, like so (note that multiplying by b is just a shift):
        //
        // x = x0 + x1 * b
        // y = y0 + y1 * b
        //
        // With some algebra, we can compute x * y with three smaller products, where the inputs to
        // each of the smaller products have only about half as many digits as x and y:
        //
        // x * y = (x0 + x1 * b) * (y0 + y1 * b)
        //
        // x * y = x0 * y0
        //       + x0 * y1 * b
        //       + x1 * y0 * b
        //       + x1 * y1 * b^2
        //
        // Let p0 = x0 * y0 and p2 = x1 * y1:
        //
        // x * y = p0
        //       + (x0 * y1 + x1 * y0) * b
        //       + p2 * b^2
        //
        // The real trick is that middle term:
        //
        //         x0 * y1 + x1 * y0
        //
        //       = x0 * y1 + x1 * y0 - p0 + p0 - p2 + p2
        //
        //       = x0 * y1 + x1 * y0 - x0 * y0 - x1 * y1 + p0 + p2
        //
        // Now we complete the square:
        //
        //       = -(x0 * y0 - x0 * y1 - x1 * y0 + x1 * y1) + p0 + p2
        //
        //       = -((x1 - x0) * (y1 - y0)) + p0 + p2
        //
        // Let p1 = (x1 - x0) * (y1 - y0), and substitute back into our original formula:
        //
        // x * y = p0
        //       + (p0 + p2 - p1) * b
        //       + p2 * b^2
        //
        // Where the three intermediate products are:
        //
        // p0 = x0 * y0
        // p1 = (x1 - x0) * (y1 - y0)
        // p2 = x1 * y1
        //
        // In doing the computation, we take great care to avoid unnecessary temporary variables
        // (since creating a BigUint requires a heap allocation): thus, we rearrange the formula a
        // bit so we can use the same temporary variable for all the intermediate products:
        //
        // x * y = p2 * b^2 + p2 * b
        //       + p0 * b + p0
        //       - p1 * b
        //
        // The other trick we use is instead of doing explicit shifts, we slice acc at the
        // appropriate offset when doing the add.

        // When x is smaller than y, it's significantly faster to pick b such that x is split in
        // half, not y:
        let b = x.len() / 2;
        let (x0, x1) = x.split_at(b);
        let (y0, y1) = y.split_at(b);

        // We reuse the same BigUint for all the intermediate multiplies and have to size p
        // appropriately here: x1.len() >= x0.len and y1.len() >= y0.len():
        let len = x1.len() + y1.len() + 1;
        let mut p = vec![0; len];

        // p2 = x1 * y1
        mac3(&mut p, x1, y1);

        // Not required, but the adds go faster if we drop any unneeded 0s from the end:
        p.normalize();

        add2(&mut acc[b..], &p);
        add2(&mut acc[b * 2..], &p);

        // Zero out p before the next multiply:
        p.truncate(0);
        p.resize(len, 0);

        // p0 = x0 * y0
        mac3(&mut p, x0, y0);
        p.normalize();

        add2(acc, &p);
        add2(&mut acc[b..], &p);

        // p1 = (x1 - x0) * (y1 - y0)
        // We do this one last, since it may be negative and acc can't ever be negative:
        let (j0_sign, j0) = sub_sign(x1, x0);
        let (j1_sign, j1) = sub_sign(y1, y0);

        match j0_sign * j1_sign {
            Plus => {
                p.data.truncate(0);
                p.data.resize(len, 0);

                mac3(&mut p.data, &j0.data, &j1.data);
                p.normalize();

                sub2(&mut acc[b..], &p.data);
            }
            Minus => {
                mac3(&mut acc[b..], &j0.data, &j1.data);
            }
            NoSign => (),
        }
    } else {
        // Toom-3 multiplication:
        //
        // Toom-3 is like Karatsuba above, but dividing the inputs into three parts.
        // Both are instances of Toom-Cook, using `k=3` and `k=2` respectively.
        //
        // The general idea is to treat the large integers digits as
        // polynomials of a certain degree and determine the coefficients/digits
        // of the product of the two via interpolation of the polynomial product.
        let i = y.len() / 3 + 1;

        let x0_len = Ord::min(x.len(), i);
        let x1_len = Ord::min(x.len() - x0_len, i);

        let y0_len = i;
        let y1_len = Ord::min(y.len() - y0_len, i);

        // Break x and y into three parts, representating an order two polynomial.
        // t is chosen to be the size of a digit so we can use faster shifts
        // in place of multiplications.
        //
        // x(t) = x2*t^2 + x1*t + x0
        let x0 = bigint_from_slice(&x[..x0_len]);
        let x1 = bigint_from_slice(&x[x0_len..x0_len + x1_len]);
        let x2 = bigint_from_slice(&x[x0_len + x1_len..]);

        // y(t) = y2*t^2 + y1*t + y0
        let y0 = bigint_from_slice(&y[..y0_len]);
        let y1 = bigint_from_slice(&y[y0_len..y0_len + y1_len]);
        let y2 = bigint_from_slice(&y[y0_len + y1_len..]);

        // Let w(t) = x(t) * y(t)
        //
        // This gives us the following order-4 polynomial.
        //
        // w(t) = w4*t^4 + w3*t^3 + w2*t^2 + w1*t + w0
        //
        // We need to find the coefficients w4, w3, w2, w1 and w0. Instead
        // of simply multiplying the x and y in total, we can evaluate w
        // at 5 points. An n-degree polynomial is uniquely identified by (n + 1)
        // points.
        //
        // It is arbitrary as to what points we evaluate w at but we use the
        // following.
        //
        // w(t) at t = 0, 1, -1, -2 and inf
        //
        // The values for w(t) in terms of x(t)*y(t) at these points are:
        //
        // let a = w(0)   = x0 * y0
        // let b = w(1)   = (x2 + x1 + x0) * (y2 + y1 + y0)
        // let c = w(-1)  = (x2 - x1 + x0) * (y2 - y1 + y0)
        // let d = w(-2)  = (4*x2 - 2*x1 + x0) * (4*y2 - 2*y1 + y0)
        // let e = w(inf) = x2 * y2 as t -> inf

        // x0 + x2, avoiding temporaries
        let p = &x0 + &x2;

        // y0 + y2, avoiding temporaries
        let q = &y0 + &y2;

        // x2 - x1 + x0, avoiding temporaries
        let p2 = &p - &x1;

        // y2 - y1 + y0, avoiding temporaries
        let q2 = &q - &y1;

        // w(0)
        let r0 = &x0 * &y0;

        // w(inf)
        let r4 = &x2 * &y2;

        // w(1)
        let r1 = (p + x1) * (q + y1);

        // w(-1)
        let r2 = &p2 * &q2;

        // w(-2)
        let r3 = ((p2 + x2) * 2 - x0) * ((q2 + y2) * 2 - y0);

        // Evaluating these points gives us the following system of linear equations.
        //
        //  0  0  0  0  1 | a
        //  1  1  1  1  1 | b
        //  1 -1  1 -1  1 | c
        // 16 -8  4 -2  1 | d
        //  1  0  0  0  0 | e
        //
        // The solved equation (after gaussian elimination or similar)
        // in terms of its coefficients:
        //
        // w0 = w(0)
        // w1 = w(0)/2 + w(1)/3 - w(-1) + w(2)/6 - 2*w(inf)
        // w2 = -w(0) + w(1)/2 + w(-1)/2 - w(inf)
        // w3 = -w(0)/2 + w(1)/6 + w(-1)/2 - w(1)/6
        // w4 = w(inf)
        //
        // This particular sequence is given by Bodrato and is an interpolation
        // of the above equations.
        let mut comp3: BigInt = (r3 - &r1) / 3u32;
        let mut comp1: BigInt = (r1 - &r2) >> 1;
        let mut comp2: BigInt = r2 - &r0;
        comp3 = ((&comp2 - comp3) >> 1) + (&r4 << 1);
        comp2 += &comp1 - &r4;
        comp1 -= &comp3;

        // Recomposition. The coefficients of the polynomial are now known.
        //
        // Evaluate at w(t) where t is our given base to get the result.
        //
        //     let bits = u64::from(big_digit::BITS) * i as u64;
        //     let result = r0
        //         + (comp1 << bits)
        //         + (comp2 << (2 * bits))
        //         + (comp3 << (3 * bits))
        //         + (r4 << (4 * bits));
        //     let result_pos = result.to_biguint().unwrap();
        //     add2(&mut acc[..], &result_pos.data);
        //
        // But with less intermediate copying:
        for (j, result) in [&r0, &comp1, &comp2, &comp3, &r4].iter().enumerate().rev() {
            match result.sign() {
                Plus => add2(&mut acc[i * j..], result.digits()),
                Minus => sub2(&mut acc[i * j..], result.digits()),
                NoSign => {}
            }
        }
    }
}

#[inline]
pub(super) fn __add2(a: &mut [usize], b: &[usize]) -> BigDigit {
    debug_assert!(a.len() >= b.len());

    let mut carry = 0;
    let (a_lo, a_hi) = a.split_at_mut(b.len());

    for (a, b) in a_lo.iter_mut().zip(b) {
        carry = adc(carry, *a, *b, a);
    }

    if carry != 0 {
        for a in a_hi {
            carry = adc(carry, *a, 0, a);
            if carry == 0 {
                break;
            }
        }
    }

    carry as BigDigit
}

/// Two argument addition of raw slices:
/// a += b
///
/// The caller _must_ ensure that a is big enough to store the result - typically this means
/// resizing a to max(a.len(), b.len()) + 1, to fit a possible carry.
pub(super) fn add2(a: &mut [BigDigit], b: &[BigDigit]) {
    let carry = __add2(a, b);

    debug_assert!(carry == 0);
}
 */
