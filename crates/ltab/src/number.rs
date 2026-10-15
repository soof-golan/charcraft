//! Number literals: classification of source text and `printf("%.17g")`-exact printing.
//!
//! Character Animator prints every number through C's `%.17g`, which is why a stored `0.2` appears as
//! `0.20000000000000001` and an integral value as a bare `100`. We keep the original text of every non-integer so
//! files round-trip byte for byte, and use [`format_g17`] for numbers we create ourselves.

/// A number literal after classification.
#[derive(Debug, Clone, PartialEq)]
pub enum Number {
    /// A decimal or hexadecimal integer that fits `i64`.
    Int(i64),
    /// Anything else (fractions, exponents, integers beyond `i64`, `-0`, `inf`, `nan`) with its source text.
    Float {
        /// The parsed value.
        value: f64,
        /// The literal exactly as written.
        text: String,
    },
}

/// Classifies a complete number literal, or returns `None` if the text is not exactly one.
///
/// A literal is an optional `-`, then a decimal integer, a hexadecimal integer (`0x1F`, wrapping into `i64` as
/// Lua 5.3 does), a decimal float (`.5`, `5.`, `1e-3`, `0.20000000000000001`) or one of the words `inf` and `nan`
/// (any case, as C's `%g` prints non-finite values).
#[must_use]
pub fn parse_number_text(text: &str) -> Option<Number> {
    let (negative, body) = text.strip_prefix('-').map_or((false, text), |rest| (true, rest));
    if body.is_empty() {
        return None;
    }
    if body.eq_ignore_ascii_case("inf") {
        let value = if negative { f64::NEG_INFINITY } else { f64::INFINITY };
        return Some(Number::Float { value, text: text.to_owned() });
    }
    if body.eq_ignore_ascii_case("nan") {
        let value = if negative { f64::NAN.copysign(-1.0) } else { f64::NAN };
        return Some(Number::Float { value, text: text.to_owned() });
    }
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        return parse_hex(negative, hex);
    }
    if body.bytes().all(|b| b.is_ascii_digit()) {
        match text.parse::<i64>() {
            // "-0" must stay a float to keep its sign (and its text) through a round trip.
            Ok(0) if negative => return Some(Number::Float { value: -0.0, text: text.to_owned() }),
            Ok(int) => return Some(Number::Int(int)),
            // Too large for i64: falls through and becomes a float, as in Lua 5.3.
            Err(_) => {}
        }
    }
    if !body.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let value = text.parse::<f64>().ok()?;
    Some(Number::Float { value, text: text.to_owned() })
}

fn parse_hex(negative: bool, digits: &str) -> Option<Number> {
    if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let unsigned = u64::from_str_radix(digits, 16).ok()?;
    let value = i64::from_ne_bytes(unsigned.to_ne_bytes());
    Some(Number::Int(if negative { value.wrapping_neg() } else { value }))
}

/// Formats `value` exactly as C's `printf("%.17g", value)` does.
///
/// That is 17 significant digits with trailing zeros removed, and exponent form (`1e+20`, `1.0000000000000001e-05`)
/// when the decimal exponent is below -4 or at least 17. Non-finite values print as `inf`, `-inf`, `nan`, `-nan`.
/// Every finite `f64` survives `format_g17(v).parse::<f64>()` unchanged.
#[must_use]
pub fn format_g17(value: f64) -> String {
    if value.is_nan() {
        return if value.is_sign_negative() { "-nan" } else { "nan" }.to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() { "-inf" } else { "inf" }.to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    // `{:.16e}` is correctly rounded to 17 significant digits, like glibc and the UCRT: "d.dddddddddddddddde<exp>".
    let scientific = format!("{:.16e}", value.abs());
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return format!("{value}");
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return format!("{value}");
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let body = if (-4..17).contains(&exponent) { fixed(&digits, exponent) } else { exponential(&digits, exponent) };
    format!("{sign}{body}")
}

/// `%g` fixed notation: `digits` are the significant digits, `exponent` the decimal exponent of the first one.
fn fixed(digits: &str, exponent: i32) -> String {
    if exponent >= 0 {
        let integer_len = usize::try_from(exponent).unwrap_or(0).saturating_add(1);
        let integer = digits.get(..integer_len).unwrap_or_default();
        let fraction = digits.get(integer_len..).unwrap_or_default().trim_end_matches('0');
        if fraction.is_empty() { integer.to_owned() } else { format!("{integer}.{fraction}") }
    } else {
        let zeros = usize::try_from(exponent.unsigned_abs()).unwrap_or(0).saturating_sub(1);
        format!("0.{}{}", "0".repeat(zeros), digits.trim_end_matches('0'))
    }
}

/// `%g` exponent notation: `d.ddde+XX` with at least two exponent digits.
fn exponential(digits: &str, exponent: i32) -> String {
    let trimmed = digits.trim_end_matches('0');
    let first = trimmed.get(..1).unwrap_or("0");
    let rest = trimmed.get(1..).unwrap_or_default();
    let sign = if exponent < 0 { '-' } else { '+' };
    let magnitude = exponent.unsigned_abs();
    if rest.is_empty() { format!("{first}e{sign}{magnitude:02}") } else { format!("{first}.{rest}e{sign}{magnitude:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g17_matches_printf_for_values_seen_in_character_animator_files() {
        assert_eq!(format_g17(0.2), "0.20000000000000001");
        assert_eq!(format_g17(0.7), "0.69999999999999996");
        assert_eq!(format_g17(0.1), "0.10000000000000001");
        assert_eq!(format_g17(0.25), "0.25");
        assert_eq!(format_g17(1.5), "1.5");
        assert_eq!(format_g17(100.0), "100");
        assert_eq!(format_g17(-20.0), "-20");
        assert_eq!(format_g17(0.0), "0");
        assert_eq!(format_g17(-0.0), "-0");
    }

    #[test]
    fn g17_switches_to_exponent_form_like_printf() {
        assert_eq!(format_g17(1e20), "1e+20");
        assert_eq!(format_g17(1e17), "1e+17");
        assert_eq!(format_g17(1e16), "10000000000000000");
        assert_eq!(format_g17(1e-5), "1.0000000000000001e-05");
        assert_eq!(format_g17(0.0001), "0.0001");
        assert_eq!(format_g17(1.5e-7), "1.4999999999999999e-07");
        assert_eq!(format_g17(1.5e300), "1.5000000000000001e+300");
        assert_eq!(format_g17(9_007_199_254_740_992.0), "9007199254740992");
        assert_eq!(format_g17(f64::MAX), "1.7976931348623157e+308");
        assert_eq!(format_g17(f64::MIN_POSITIVE), "2.2250738585072014e-308");
        assert_eq!(format_g17(5e-324), "4.9406564584124654e-324");
    }

    #[test]
    fn g17_prints_non_finite_values_like_printf() {
        assert_eq!(format_g17(f64::INFINITY), "inf");
        assert_eq!(format_g17(f64::NEG_INFINITY), "-inf");
        assert_eq!(format_g17(f64::NAN), "nan");
        assert_eq!(format_g17(f64::NAN.copysign(-1.0)), "-nan");
    }

    #[test]
    fn integers_and_floats_are_classified() {
        assert_eq!(parse_number_text("30"), Some(Number::Int(30)));
        assert_eq!(parse_number_text("-20"), Some(Number::Int(-20)));
        assert_eq!(parse_number_text("007"), Some(Number::Int(7)));
        assert_eq!(parse_number_text("-9223372036854775808"), Some(Number::Int(i64::MIN)));
        assert_eq!(parse_number_text("0x10"), Some(Number::Int(16)));
        assert_eq!(parse_number_text("-0x10"), Some(Number::Int(-16)));
        assert_eq!(parse_number_text("0xFFFFFFFFFFFFFFFF"), Some(Number::Int(-1)));
        assert_eq!(parse_number_text("0.20000000000000001"), Some(Number::Float { value: 0.2, text: "0.20000000000000001".to_owned() }));
        assert_eq!(parse_number_text("1e+20"), Some(Number::Float { value: 1e20, text: "1e+20".to_owned() }));
        assert_eq!(parse_number_text(".5"), Some(Number::Float { value: 0.5, text: ".5".to_owned() }));
        assert_eq!(parse_number_text("5."), Some(Number::Float { value: 5.0, text: "5.".to_owned() }));
        assert_eq!(parse_number_text("9223372036854775808"), Some(Number::Float { value: 9.223_372_036_854_776e18, text: "9223372036854775808".to_owned() }));
    }

    #[test]
    fn negative_zero_keeps_its_sign_and_text() {
        let Some(Number::Float { value, text }) = parse_number_text("-0") else { panic!("expected a float") };
        assert!(value.is_sign_negative() && value == 0.0);
        assert_eq!(text, "-0");
    }

    #[test]
    fn non_finite_words_are_numbers() {
        assert!(matches!(parse_number_text("inf"), Some(Number::Float { value, .. }) if value == f64::INFINITY));
        assert!(matches!(parse_number_text("-INF"), Some(Number::Float { value, .. }) if value == f64::NEG_INFINITY));
        assert!(matches!(parse_number_text("nan"), Some(Number::Float { value, .. }) if value.is_nan()));
        assert!(matches!(parse_number_text("-nan"), Some(Number::Float { value, .. }) if value.is_nan() && value.is_sign_negative()));
    }

    #[test]
    fn garbage_is_rejected() {
        for bad in ["", "-", "--5", "abc", "1.2.3", "1e", "0x", "0xZZ", "0x11111111111111111", "1_000", "5abc", "+5", "e5", "0b11", "1 2"] {
            assert_eq!(parse_number_text(bad), None, "{bad:?}");
        }
    }
}
