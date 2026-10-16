//! Times are stored as text: a double printed with C's `%.36g`.
//!
//! That is the exact decimal expansion of the double rounded to 36 significant digits with trailing zeros dropped
//! (`323638.973301917023491114377975463867`). Reading uses the standard library's correctly rounded parser; writing needs the exact expansion, which this
//! module computes with a tiny big-integer, so that `format(parse(text)) == text` holds for every spelling the
//! recorder wrote.

use std::fmt::Write;

/// Significant digits Character Animator prints times with.
pub const PRECISION: usize = 36;

/// Little-endian base-2^32 integer, just enough to expand a double exactly.
struct Big(Vec<u32>);

fn low(v: u64) -> u32 {
    u32::try_from(v & 0xFFFF_FFFF).unwrap_or(0)
}

impl Big {
    fn from_u64(v: u64) -> Self {
        let mut limbs = vec![low(v), low(v >> 32)];
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self(limbs)
    }

    fn mul_small(&mut self, k: u32) {
        let mut carry = 0_u64;
        for limb in &mut self.0 {
            let cur = u64::from(*limb).wrapping_mul(u64::from(k)).wrapping_add(carry);
            *limb = low(cur);
            carry = cur >> 32;
        }
        if carry != 0 {
            self.0.push(low(carry));
        }
    }

    fn shl(&mut self, bits: u32) {
        let whole = usize::try_from(bits / 32).unwrap_or(0);
        let part = bits % 32;
        if part != 0 {
            let mut carry = 0_u32;
            for limb in &mut self.0 {
                let next = *limb >> 32_u32.wrapping_sub(part);
                *limb = (*limb << part) | carry;
                carry = next;
            }
            if carry != 0 {
                self.0.push(carry);
            }
        }
        if !self.0.is_empty() {
            let mut shifted = vec![0_u32; whole];
            shifted.append(&mut self.0);
            self.0 = shifted;
        }
    }

    /// Divides by `d` in place and returns the remainder.
    fn div_small(&mut self, d: u32) -> u32 {
        let mut rem = 0_u64;
        let divisor = u64::from(d.max(1));
        for limb in self.0.iter_mut().rev() {
            let cur = (rem << 32) | u64::from(*limb);
            *limb = low(cur.checked_div(divisor).unwrap_or(0));
            rem = cur.checked_rem(divisor).unwrap_or(0);
        }
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
        low(rem)
    }

    fn into_decimal(mut self) -> String {
        if self.0.is_empty() {
            return "0".to_owned();
        }
        let mut groups = Vec::new();
        while !self.0.is_empty() {
            groups.push(self.div_small(1_000_000_000));
        }
        let mut out = String::new();
        for (i, g) in groups.iter().rev().enumerate() {
            let _ = if i == 0 { write!(out, "{g}") } else { write!(out, "{g:09}") };
        }
        out
    }
}

/// The exact decimal digits of a positive finite double and the power of ten of the first one.
fn exact_digits(value: f64) -> (Vec<u8>, i32) {
    let bits = value.to_bits();
    let raw_exp = i32::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    let fraction = bits & ((1_u64 << 52) - 1);
    let (mantissa, exp2) = if raw_exp == 0 { (fraction, -1074) } else { (fraction | (1_u64 << 52), raw_exp.wrapping_sub(1075)) };
    let mut n = Big::from_u64(mantissa);
    let mut exp10 = 0_i32;
    if exp2 >= 0 {
        n.shl(u32::try_from(exp2).unwrap_or(0));
    } else {
        // m / 2^k = m * 5^k / 10^k
        let mut k = exp2.unsigned_abs();
        exp10 = 0_i32.saturating_sub(i32::try_from(k).unwrap_or(0));
        while k > 0 {
            let step = k.min(13);
            n.mul_small(5_u32.pow(step));
            k = k.wrapping_sub(step);
        }
    }
    let digits: Vec<u8> = n.into_decimal().into_bytes();
    let first = i32::try_from(digits.len()).unwrap_or(0).wrapping_sub(1).wrapping_add(exp10);
    (digits, first)
}

/// Rounds `digits` to `precision` significant digits, half to even on the exact value. Returns the new first-digit
/// exponent.
fn round_digits(digits: &mut Vec<u8>, mut exp: i32, precision: usize) -> i32 {
    if digits.len() <= precision {
        return exp;
    }
    let next = digits.get(precision).copied().unwrap_or(b'0');
    let rest_nonzero = digits.get(precision.saturating_add(1)..).is_some_and(|r| r.iter().any(|&d| d != b'0'));
    let last_odd = precision.checked_sub(1).and_then(|i| digits.get(i)).is_some_and(|d| d & 1 == 1);
    let up = next > b'5' || (next == b'5' && (rest_nonzero || last_odd));
    digits.truncate(precision);
    if up {
        let mut i = precision;
        loop {
            let Some(j) = i.checked_sub(1) else {
                digits.insert(0, b'1');
                digits.truncate(precision);
                exp = exp.wrapping_add(1);
                break;
            };
            match digits.get_mut(j) {
                Some(d) if *d == b'9' => {
                    *d = b'0';
                    i = j;
                }
                Some(d) => {
                    *d = d.wrapping_add(1);
                    break;
                }
                None => break,
            }
        }
    }
    exp
}

/// `%.{precision}g` of a double, as C prints it.
#[must_use]
pub fn format_g(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let precision = precision.max(1);
    let (mut digits, first) = exact_digits(value.abs());
    let exp = round_digits(&mut digits, first, precision);
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
    let text = |d: &[u8]| String::from_utf8_lossy(d).into_owned();
    let limit = i32::try_from(precision).unwrap_or(i32::MAX);
    if exp < -4 || exp >= limit {
        let (head, tail) = digits.split_at(1);
        let mantissa = if tail.is_empty() { text(head) } else { format!("{}.{}", text(head), text(tail)) };
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{mantissa}e{exp_sign}{:02}", exp.unsigned_abs())
    } else if exp < 0 {
        format!("{sign}0.{}{}", "0".repeat(usize::try_from((-1_i32).wrapping_sub(exp)).unwrap_or(0)), text(&digits))
    } else {
        let int_len = usize::try_from(exp).unwrap_or(0).saturating_add(1);
        if digits.len() <= int_len {
            let zeros = int_len.saturating_sub(digits.len());
            format!("{sign}{}{}", text(&digits), "0".repeat(zeros))
        } else {
            let (int, frac) = digits.split_at(int_len);
            format!("{sign}{}.{}", text(int), text(frac))
        }
    }
}

/// How Character Animator spells a time.
#[must_use]
pub fn format_time(value: f64) -> String {
    format_g(value, PRECISION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_what_the_recorder_wrote() {
        assert_eq!(format_time(0.0), "0");
        assert_eq!(format_time(0.25), "0.25");
        assert_eq!(format_time(0.5), "0.5");
        assert_eq!(format_time(1.0), "1");
        assert_eq!(format_time(5.0), "5");
        for text in [
            "103.796979166666673677354992832988501",
            "90.435937499999994315658113919198513",
            "323638.973301917023491114377975463867",
            "400249.485187041980680078268051147461",
            "90.420000000000101181285572238266468",
        ] {
            let value: f64 = text.parse().unwrap();
            assert_eq!(format_time(value), text);
        }
    }

    #[test]
    fn follows_printf_g() {
        assert_eq!(format_g(0.1, 17), "0.10000000000000001");
        assert_eq!(format_g(123_456.0, 3), "1.23e+05");
        assert_eq!(format_g(1.234e-4, 3), "0.000123");
        assert_eq!(format_g(1.234e-5, 3), "1.23e-05");
        assert_eq!(format_g(1e100, 17), "1e+100");
        assert_eq!(format_g(-2.5, 36), "-2.5");
        assert_eq!(format_g(2.5, 1), "2");
        assert_eq!(format_g(3.5, 1), "4");
        assert_eq!(format_g(9.99, 2), "10");
        assert_eq!(format_g(f64::MIN_POSITIVE, 5), "2.2251e-308");
        assert_eq!(format_g(5e-324, 3), "4.94e-324");
        assert_eq!(format_g(f64::MAX, 6), "1.79769e+308");
    }

    #[test]
    fn every_double_reads_back_exactly() {
        for value in [0.1, 1.0 / 3.0, 12345.678, 1e-7, 2.0_f64.powi(60), 323_638.973_301_917_f64, f64::MAX, f64::MIN_POSITIVE, 5e-324] {
            assert_eq!(format_time(value).parse::<f64>().unwrap().to_bits(), value.to_bits(), "{value}");
        }
    }
}
