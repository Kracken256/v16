use std::cmp::Ordering;
use std::fmt;

pub const MAX_BITS: u64 = 1 << 27;

const LIMB_BITS: u64 = 64;
const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const BASE_F64: f64 = 18446744073709551616.0;

#[derive(Clone, Debug)]
pub struct BigInt {
    negative: bool,
    limbs: Vec<u64>,
}

fn normalize(limbs: &mut Vec<u64>) {
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
}

fn from_magnitude(mut limbs: Vec<u64>) -> BigInt {
    normalize(&mut limbs);
    BigInt {
        negative: false,
        limbs,
    }
}

fn magnitude_cmp(a: &[u64], b: &[u64]) -> Ordering {
    let mut alen = a.len();
    while alen > 0 && a[alen - 1] == 0 {
        alen -= 1;
    }
    let mut blen = b.len();
    while blen > 0 && b[blen - 1] == 0 {
        blen -= 1;
    }
    if alen != blen {
        return alen.cmp(&blen);
    }
    for i in (0..alen).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn magnitude_add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for i in 0..long.len() {
        let s = long[i] as u128 + if i < short.len() { short[i] as u128 } else { 0 } + carry as u128;
        out.push(s as u64);
        carry = (s >> 64) as u64;
    }
    if carry > 0 {
        out.push(carry);
    }
    out
}

fn magnitude_sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i128;
    for i in 0..a.len() {
        let rhs = if i < b.len() { b[i] as i128 } else { 0 };
        let diff = a[i] as i128 - rhs - borrow;
        if diff < 0 {
            out.push((diff + (1i128 << 64)) as u64);
            borrow = 1;
        } else {
            out.push(diff as u64);
            borrow = 0;
        }
    }
    normalize(&mut out);
    out
}

fn magnitude_mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u64; a.len() + b.len()];
    for (i, &av) in a.iter().enumerate() {
        if av == 0 {
            continue;
        }
        let mut carry = 0u64;
        for (j, &bv) in b.iter().enumerate() {
            let cur = out[i + j] as u128 + av as u128 * bv as u128 + carry as u128;
            out[i + j] = cur as u64;
            carry = (cur >> 64) as u64;
        }
        let mut k = i + b.len();
        while carry > 0 {
            let cur = out[k] as u128 + carry as u128;
            out[k] = cur as u64;
            carry = (cur >> 64) as u64;
            k += 1;
        }
    }
    normalize(&mut out);
    out
}

fn magnitude_shl_small(a: &[u64], count: u64) -> Vec<u64> {
    if a.is_empty() || count == 0 {
        return a.to_vec();
    }
    let limb_shift = (count / LIMB_BITS) as usize;
    let bit_shift = (count % LIMB_BITS) as u32;
    let mut out = vec![0u64; a.len() + limb_shift + 1];
    for (i, &limb) in a.iter().enumerate() {
        if bit_shift == 0 {
            out[i + limb_shift] = limb;
        } else {
            out[i + limb_shift] |= limb << bit_shift;
            out[i + limb_shift + 1] |= limb >> (64 - bit_shift);
        }
    }
    normalize(&mut out);
    out
}

fn magnitude_shr_small(a: &[u64], count: u64) -> Vec<u64> {
    if a.is_empty() {
        return Vec::new();
    }
    let limb_shift = (count / LIMB_BITS) as usize;
    if limb_shift >= a.len() {
        return Vec::new();
    }
    let bit_shift = (count % LIMB_BITS) as u32;
    let mut out = vec![0u64; a.len() - limb_shift];
    if bit_shift == 0 {
        out.copy_from_slice(&a[limb_shift..]);
    } else {
        for i in 0..out.len() {
            let lo = a[i + limb_shift] >> bit_shift;
            let hi = if i + limb_shift + 1 < a.len() {
                a[i + limb_shift + 1] << (64 - bit_shift)
            } else {
                0
            };
            out[i] = lo | hi;
        }
    }
    normalize(&mut out);
    out
}

fn bit_len_of(a: &[u64]) -> u64 {
    let mut len = a.len();
    while len > 0 && a[len - 1] == 0 {
        len -= 1;
    }
    if len == 0 {
        0
    } else {
        (len as u64 - 1) * LIMB_BITS + (LIMB_BITS - a[len - 1].leading_zeros() as u64)
    }
}

fn magnitude_div_rem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    if magnitude_cmp(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let mut quotient = a.to_vec();
        let rem = div_rem_small(&mut quotient, b[0]);
        return (quotient, if rem == 0 { Vec::new() } else { vec![rem] });
    }
    let bits = bit_len_of(a);
    let mut quotient = vec![0u64; ((bits + 63) / 64) as usize];
    let mut rem: Vec<u64> = Vec::new();
    for i in (0..bits).rev() {
        if !rem.is_empty() {
            let bit_shift = 1u32;
            let mut carry = 0u64;
            for limb in rem.iter_mut() {
                let new_carry = *limb >> (64 - bit_shift);
                *limb = (*limb << bit_shift) | carry;
                carry = new_carry;
            }
            if carry > 0 {
                rem.push(carry);
            }
        }
        if (a[(i / LIMB_BITS) as usize] >> (i % LIMB_BITS)) & 1 == 1 {
            if rem.is_empty() {
                rem.push(1);
            } else {
                rem[0] |= 1;
            }
        }
        normalize(&mut rem);
        if magnitude_cmp(&rem, b) != Ordering::Less {
            rem = magnitude_sub(&rem, b);
            quotient[(i / LIMB_BITS) as usize] |= 1u64 << (i % LIMB_BITS);
        }
    }
    (quotient, rem)
}

fn div_rem_small(mag: &mut Vec<u64>, divisor: u64) -> u64 {
    let mut rem = 0u128;
    let d = divisor as u128;
    for limb in mag.iter_mut().rev() {
        let cur = (rem << 64) | *limb as u128;
        *limb = (cur / d) as u64;
        rem = cur % d;
    }
    normalize(mag);
    rem as u64
}

fn mul_small_add(mag: &mut Vec<u64>, multiplier: u64, addend: u64) {
    let mut carry = addend as u128;
    let m = multiplier as u128;
    for limb in mag.iter_mut() {
        let cur = *limb as u128 * m + carry;
        *limb = cur as u64;
        carry = cur >> 64;
    }
    while carry > 0 {
        mag.push(carry as u64);
        carry >>= 64;
    }
}

fn twos_complement(value: &BigInt, limb_count: usize) -> Vec<u64> {
    let mut out = vec![0u64; limb_count];
    for (i, &limb) in value.limbs.iter().enumerate() {
        if i >= limb_count {
            break;
        }
        out[i] = limb;
    }
    if value.negative {
        let mut carry = true;
        for limb in out.iter_mut() {
            *limb = !*limb;
            if carry {
                let (next, overflow) = limb.overflowing_add(1);
                *limb = next;
                carry = overflow;
            }
        }
    }
    out
}

fn from_twos_complement(mut limbs: Vec<u64>) -> BigInt {
    let negative = limbs.last().map_or(false, |&l| l >> 63 == 1);
    if !negative {
        return from_magnitude(limbs);
    }
    let mut carry = true;
    for limb in limbs.iter_mut() {
        *limb = !*limb;
        if carry {
            let (next, overflow) = limb.overflowing_add(1);
            *limb = next;
            carry = overflow;
        }
    }
    normalize(&mut limbs);
    BigInt {
        negative: true,
        limbs,
    }
}

impl BigInt {
    pub fn zero() -> BigInt {
        BigInt {
            negative: false,
            limbs: Vec::new(),
        }
    }

    pub fn one() -> BigInt {
        BigInt {
            negative: false,
            limbs: vec![1],
        }
    }

    pub fn from_u64(value: u64) -> BigInt {
        if value == 0 {
            BigInt::zero()
        } else {
            BigInt {
                negative: false,
                limbs: vec![value],
            }
        }
    }

    pub fn from_u128(value: u128) -> BigInt {
        let mut limbs = vec![value as u64, (value >> 64) as u64];
        normalize(&mut limbs);
        BigInt {
            negative: false,
            limbs,
        }
    }

    pub fn from_i64(value: i64) -> BigInt {
        BigInt::from_i128(value as i128)
    }

    pub fn from_i128(value: i128) -> BigInt {
        if value == 0 {
            return BigInt::zero();
        }
        let magnitude = value.unsigned_abs();
        let mut limbs = vec![magnitude as u64, (magnitude >> 64) as u64];
        normalize(&mut limbs);
        BigInt {
            negative: value < 0,
            limbs,
        }
    }

    pub fn from_f64_integral(value: f64) -> Option<BigInt> {
        if !value.is_finite() || value.fract() != 0.0 {
            return None;
        }
        if value == 0.0 {
            return Some(BigInt::zero());
        }
        let bits = value.to_bits();
        let negative = bits >> 63 == 1;
        let raw_exponent = ((bits >> 52) & 0x7ff) as i64;
        if raw_exponent == 0 {
            return Some(BigInt::zero());
        }
        let mantissa = (bits & 0x000f_ffff_ffff_ffff) | (1u64 << 52);
        let exponent = raw_exponent - 1075;
        let mut result = BigInt::from_u64(mantissa);
        if exponent >= 0 {
            result = result.shl(exponent as u64)?;
        } else {
            result = result.shr((-exponent) as u64);
        }
        if negative {
            result = result.negate();
        }
        Some(result)
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    #[inline]
    pub fn is_negative(&self) -> bool {
        self.negative
    }

    pub fn bit_len(&self) -> u64 {
        bit_len_of(&self.limbs)
    }

    pub fn bit(&self, index: u64) -> bool {
        let limb_index = (index / LIMB_BITS) as usize;
        if limb_index >= self.limbs.len() {
            return false;
        }
        (self.limbs[limb_index] >> (index % LIMB_BITS)) & 1 == 1
    }

    pub fn to_u128(&self) -> Option<u128> {
        if self.limbs.len() > 2 {
            return None;
        }
        let mut out = 0u128;
        for (i, &limb) in self.limbs.iter().enumerate() {
            out |= (limb as u128) << (64 * i);
        }
        Some(out)
    }

    pub fn to_i128(&self) -> Option<i128> {
        let magnitude = self.to_u128()?;
        if self.negative {
            if magnitude <= (i128::MAX as u128) + 1 {
                Some((magnitude as i128).wrapping_neg())
            } else {
                None
            }
        } else if magnitude <= i128::MAX as u128 {
            Some(magnitude as i128)
        } else {
            None
        }
    }

    pub fn to_u64_wrapping(&self) -> u64 {
        let low = self.limbs.first().copied().unwrap_or(0);
        if self.negative {
            low.wrapping_neg()
        } else {
            low
        }
    }

    pub fn to_i64_wrapping(&self) -> i64 {
        self.to_u64_wrapping() as i64
    }

    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let mut result = 0.0f64;
        for &limb in self.limbs.iter().rev() {
            result = result * BASE_F64 + limb as f64;
        }
        if self.negative {
            -result
        } else {
            result
        }
    }

    pub fn negate(&self) -> BigInt {
        if self.is_zero() {
            return BigInt::zero();
        }
        BigInt {
            negative: !self.negative,
            limbs: self.limbs.clone(),
        }
    }

    pub fn abs(&self) -> BigInt {
        BigInt {
            negative: false,
            limbs: self.limbs.clone(),
        }
    }

    pub fn add(&self, other: &BigInt) -> BigInt {
        if self.negative == other.negative {
            from_magnitude(magnitude_add(&self.limbs, &other.limbs)).with_sign(self.negative)
        } else {
            match magnitude_cmp(&self.limbs, &other.limbs) {
                Ordering::Equal => BigInt::zero(),
                Ordering::Greater => {
                    from_magnitude(magnitude_sub(&self.limbs, &other.limbs)).with_sign(self.negative)
                }
                Ordering::Less => {
                    from_magnitude(magnitude_sub(&other.limbs, &self.limbs)).with_sign(other.negative)
                }
            }
        }
    }

    pub fn sub(&self, other: &BigInt) -> BigInt {
        self.add(&other.negate())
    }

    pub fn mul(&self, other: &BigInt) -> BigInt {
        from_magnitude(magnitude_mul(&self.limbs, &other.limbs))
            .with_sign(self.negative != other.negative)
    }

    pub fn div_rem(&self, other: &BigInt) -> Option<(BigInt, BigInt)> {
        if other.is_zero() {
            return None;
        }
        let (quotient, remainder) = magnitude_div_rem(&self.limbs, &other.limbs);
        Some((
            from_magnitude(quotient).with_sign(self.negative != other.negative),
            from_magnitude(remainder).with_sign(self.negative),
        ))
    }

    pub fn pow(&self, exponent: &BigInt) -> Option<BigInt> {
        if exponent.negative {
            return None;
        }
        if exponent.is_zero() {
            return Some(BigInt::one());
        }
        if self.is_zero() {
            return Some(BigInt::zero());
        }
        if self.limbs.len() == 1 && self.limbs[0] == 1 {
            if self.negative && exponent.bit(0) {
                return Some(BigInt::from_i64(-1));
            }
            return Some(BigInt::one());
        }
        let exponent_value = exponent.to_u128()?;
        if exponent_value > u64::MAX as u128 {
            return None;
        }
        let base_bits = self.bit_len() as u128;
        if base_bits * exponent_value > MAX_BITS as u128 {
            return None;
        }
        let mut result = BigInt::one();
        let mut base = self.abs();
        let mut remaining = exponent_value as u64;
        while remaining > 0 {
            if remaining & 1 == 1 {
                result = result.mul(&base);
            }
            remaining >>= 1;
            if remaining > 0 {
                base = base.mul(&base);
            }
        }
        if self.negative && exponent.bit(0) {
            result = result.negate();
        }
        Some(result)
    }

    pub fn shl(&self, count: u64) -> Option<BigInt> {
        if self.is_zero() || count == 0 {
            return Some(self.clone());
        }
        if self.bit_len() + count > MAX_BITS {
            return None;
        }
        Some(BigInt {
            negative: self.negative,
            limbs: magnitude_shl_small(&self.limbs, count),
        })
    }

    pub fn shr(&self, count: u64) -> BigInt {
        if self.is_zero() {
            return BigInt::zero();
        }
        if !self.negative {
            return from_magnitude(magnitude_shr_small(&self.limbs, count));
        }
        if count >= self.bit_len() {
            return BigInt::from_i64(-1);
        }
        let minus_one = magnitude_sub(&self.limbs, &[1u64]);
        let shifted = magnitude_shr_small(&minus_one, count);
        let result = magnitude_add(&shifted, &[1u64]);
        BigInt {
            negative: true,
            limbs: result,
        }
    }

    pub fn bit_and(&self, other: &BigInt) -> BigInt {
        let width = self.limbs.len().max(other.limbs.len()) + 1;
        let a = twos_complement(self, width);
        let b = twos_complement(other, width);
        let limbs: Vec<u64> = a.iter().zip(b.iter()).map(|(x, y)| x & y).collect();
        from_twos_complement(limbs)
    }

    pub fn bit_or(&self, other: &BigInt) -> BigInt {
        let width = self.limbs.len().max(other.limbs.len()) + 1;
        let a = twos_complement(self, width);
        let b = twos_complement(other, width);
        let limbs: Vec<u64> = a.iter().zip(b.iter()).map(|(x, y)| x | y).collect();
        from_twos_complement(limbs)
    }

    pub fn bit_xor(&self, other: &BigInt) -> BigInt {
        let width = self.limbs.len().max(other.limbs.len()) + 1;
        let a = twos_complement(self, width);
        let b = twos_complement(other, width);
        let limbs: Vec<u64> = a.iter().zip(b.iter()).map(|(x, y)| x ^ y).collect();
        from_twos_complement(limbs)
    }

    pub fn bit_not(&self) -> BigInt {
        self.add(&BigInt::one()).negate()
    }

    pub fn as_uint_n(&self, bits: u64) -> Option<BigInt> {
        if bits == 0 {
            return Some(BigInt::zero());
        }
        if !self.negative && self.bit_len() <= bits {
            return Some(self.clone());
        }
        if bits > MAX_BITS {
            return None;
        }
        let width = ((bits + 63) / 64) as usize;
        let mut limbs = twos_complement(self, width);
        let extra = width as u64 * LIMB_BITS - bits;
        if extra > 0 {
            limbs[width - 1] &= u64::MAX >> extra;
        }
        Some(from_magnitude(limbs))
    }

    pub fn as_int_n(&self, bits: u64) -> Option<BigInt> {
        if bits == 0 {
            return Some(BigInt::zero());
        }
        if self.bit_len() < bits {
            return Some(self.clone());
        }
        let unsigned = self.as_uint_n(bits)?;
        if unsigned.bit(bits - 1) {
            let modulus = BigInt::one().shl(bits)?;
            Some(unsigned.sub(&modulus))
        } else {
            Some(unsigned)
        }
    }

    pub fn cmp_f64(&self, value: f64) -> Option<Ordering> {
        if value.is_nan() {
            return None;
        }
        if value == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if value == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let floor = BigInt::from_f64_integral(value.floor())?;
        match self.cmp(&floor) {
            Ordering::Equal => Some(if value.fract() == 0.0 {
                Ordering::Equal
            } else {
                Ordering::Less
            }),
            other => Some(other),
        }
    }

    pub fn from_str_radix(source: &str, radix: u32) -> Option<BigInt> {
        if !(2..=36).contains(&radix) {
            return None;
        }
        let mut chars = source.chars().peekable();
        let mut negative = false;
        match chars.peek() {
            Some('-') => {
                negative = true;
                chars.next();
            }
            Some('+') => {
                chars.next();
            }
            _ => {}
        }
        let mut limbs: Vec<u64> = Vec::new();
        let mut any_digit = false;
        for ch in chars {
            let digit = ch.to_digit(radix)? as u64;
            if limbs.is_empty() {
                if digit == 0 {
                    any_digit = true;
                    continue;
                }
                limbs.push(0);
            }
            mul_small_add(&mut limbs, radix as u64, digit);
            any_digit = true;
        }
        if !any_digit {
            return None;
        }
        let mut result = from_magnitude(limbs);
        if negative {
            result = result.negate();
        }
        Some(result)
    }

    pub fn to_string_radix(&self, radix: u32) -> String {
        if radix == 10 {
            return self.to_string();
        }
        if self.is_zero() {
            return "0".to_string();
        }
        let mut magnitude = self.limbs.clone();
        let mut digits: Vec<u8> = Vec::new();
        while !magnitude.is_empty() {
            let rem = div_rem_small(&mut magnitude, radix as u64);
            digits.push(DIGITS[rem as usize]);
        }
        if self.negative {
            digits.push(b'-');
        }
        digits.reverse();
        String::from_utf8(digits).unwrap_or_default()
    }

    fn with_sign(mut self, negative: bool) -> BigInt {
        if !self.is_zero() {
            self.negative = negative;
        }
        self
    }
}

impl fmt::Display for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        let mut magnitude = self.limbs.clone();
        let mut digits: Vec<u8> = Vec::new();
        while !magnitude.is_empty() {
            let rem = div_rem_small(&mut magnitude, 10);
            digits.push(DIGITS[rem as usize]);
        }
        if self.negative {
            digits.push(b'-');
        }
        digits.reverse();
        let text = String::from_utf8(digits).map_err(|_| fmt::Error)?;
        f.write_str(&text)
    }
}

impl PartialEq for BigInt {
    fn eq(&self, other: &Self) -> bool {
        self.negative == other.negative && self.limbs == other.limbs
    }
}

impl Eq for BigInt {}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let magnitude = magnitude_cmp(&self.limbs, &other.limbs);
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}

macro_rules! impl_from_signed {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for BigInt {
                fn from(value: $ty) -> BigInt {
                    BigInt::from_i128(value as i128)
                }
            }
        )*
    };
}

macro_rules! impl_from_unsigned {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for BigInt {
                fn from(value: $ty) -> BigInt {
                    BigInt::from_u128(value as u128)
                }
            }
        )*
    };
}

impl_from_signed!(i8, i16, i32, i64, i128);
impl_from_unsigned!(u8, u16, u32, u64, u128);

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> BigInt {
        BigInt::from_str_radix(text, 10).unwrap()
    }

    #[test]
    fn test_from_and_to_i128() {
        assert_eq!(BigInt::from_i128(i128::MIN).to_i128(), Some(i128::MIN));
        assert_eq!(BigInt::from_i128(i128::MAX).to_i128(), Some(i128::MAX));
        assert_eq!(BigInt::from_i128(-1).to_i128(), Some(-1));
        assert_eq!(BigInt::zero().to_i128(), Some(0));
        let big = parse("170141183460469231731687303715884105728");
        assert_eq!(big.to_i128(), None);
        assert_eq!(big.to_string(), "170141183460469231731687303715884105728");
    }

    #[test]
    fn test_add_sub_mul() {
        let a = parse("340282366920938463463374607431768211456");
        let b = parse("18446744073709551616");
        assert_eq!(a.add(&b).to_string(), "340282366920938463481821351505477763072");
        assert_eq!(a.sub(&b).to_string(), "340282366920938463444927863358058659840");
        assert_eq!(b.mul(&b).to_string(), "340282366920938463463374607431768211456");
        let neg = BigInt::from_i64(-5);
        assert_eq!(neg.add(&BigInt::from_i64(3)).to_i64_wrapping(), -2);
        assert_eq!(BigInt::from_i64(-5).mul(&BigInt::from_i64(-7)).to_i128(), Some(35));
    }

    #[test]
    fn test_div_rem() {
        let a = parse("123456789012345678901234567890");
        let b = parse("9876543210");
        let (q, r) = a.div_rem(&b).unwrap();
        assert_eq!(q.to_string(), "12499999887343749990");
        assert_eq!(r.to_string(), "1562499990");
        let (q, r) = BigInt::from_i64(-7).div_rem(&BigInt::from_i64(2)).unwrap();
        assert_eq!(q.to_i128(), Some(-3));
        assert_eq!(r.to_i128(), Some(-1));
    }

    #[test]
    fn test_shifts() {
        assert_eq!(
            BigInt::one().shl(128).unwrap().to_string(),
            "340282366920938463463374607431768211456"
        );
        assert_eq!(BigInt::from_i64(-1).shr(10).to_i128(), Some(-1));
        assert_eq!(BigInt::from_i64(-1024).shr(10).to_i128(), Some(-1));
        assert_eq!(BigInt::from_i64(-1025).shr(10).to_i128(), Some(-2));
        assert_eq!(BigInt::from_i64(1025).shr(10).to_i128(), Some(1));
    }

    #[test]
    fn test_bitwise() {
        let huge = parse("254125715536285641815112686497309415296");
        let other = parse("23109746335811388574424571749");
        assert_eq!(
            huge.bit_and(&other).to_string(),
            "20437079930624964712377882368"
        );
        assert_eq!(BigInt::from_i64(-1).bit_and(&BigInt::from_i64(255)).to_i128(), Some(255));
        assert_eq!(BigInt::from_i64(-2).bit_or(&BigInt::from_i64(1)).to_i128(), Some(-1));
        assert_eq!(BigInt::from_i64(-5).bit_xor(&BigInt::from_i64(3)).to_i128(), Some(-8));
        assert_eq!(BigInt::from_i64(5).bit_not().to_i128(), Some(-6));
    }

    #[test]
    fn test_pow() {
        assert_eq!(BigInt::from_i64(2).pow(&BigInt::from_i64(10)).unwrap().to_i128(), Some(1024));
        assert_eq!(BigInt::from_i64(0).pow(&BigInt::from_i64(0)).unwrap().to_i128(), Some(1));
        assert_eq!(
            BigInt::from_i64(2).pow(&BigInt::from_i64(128)).unwrap().to_string(),
            "340282366920938463463374607431768211456"
        );
        assert_eq!(BigInt::from_i64(-2).pow(&BigInt::from_i64(3)).unwrap().to_i128(), Some(-8));
    }

    #[test]
    fn test_as_n() {
        let value = BigInt::from_i64(-1);
        assert_eq!(value.as_uint_n(64).unwrap().to_string(), "18446744073709551615");
        assert_eq!(value.as_int_n(64).unwrap().to_i128(), Some(-1));
        let big = BigInt::one().shl(200).unwrap();
        assert_eq!(big.as_uint_n(64).unwrap().to_string(), "0");
        let x = parse("18446744073709551617");
        assert_eq!(x.as_uint_n(64).unwrap().to_string(), "1");
        assert_eq!(x.as_int_n(64).unwrap().to_i128(), Some(1));
    }

    #[test]
    fn test_strings() {
        assert_eq!(parse("123456789012345678901234567890").to_string(), "123456789012345678901234567890");
        assert_eq!(BigInt::from_i64(-255).to_string_radix(16), "-ff");
        assert_eq!(BigInt::from_i128(i128::MIN).to_string(), "-170141183460469231731687303715884105728");
        assert_eq!(BigInt::from_str_radix("", 10), None);
        assert_eq!(BigInt::from_str_radix("+", 10), None);
        assert_eq!(BigInt::from_str_radix("1_0", 10), None);
    }

    #[test]
    fn test_cmp_f64() {
        assert_eq!(BigInt::from_i64(2).cmp_f64(1.5), Some(Ordering::Greater));
        assert_eq!(BigInt::from_i64(1).cmp_f64(1.5), Some(Ordering::Less));
        assert_eq!(BigInt::from_i64(1).cmp_f64(1.0), Some(Ordering::Equal));
        assert_eq!(BigInt::from_i64(9).cmp_f64(f64::NAN), None);
    }
}
