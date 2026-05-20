use std::mem::MaybeUninit;
use std::ptr::NonNull;
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
