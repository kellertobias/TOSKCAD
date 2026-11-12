//! Bounded exact admission of literal IEEE-754 coordinates.
use crate::Rational;

// Exact binary64 admission for the bounded rational proof below. Decimal
// round-trip text would preserve the float but not its exact binary value.
pub(crate) fn binary_fraction(value: f64) -> Option<Rational> {
    if !value.is_finite() {
        return None;
    }
    if value == 0. {
        return Rational::new(0, 1).ok();
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mut significand = bits & ((1_u64 << 52) - 1);
    let mut power = if exponent == 0 {
        -1074
    } else {
        significand |= 1_u64 << 52;
        exponent - 1075
    };
    let reduction = significand.trailing_zeros().min((-power).max(0) as u32);
    significand >>= reduction;
    power += reduction as i32;
    if power.unsigned_abs() > 62 {
        return None;
    }
    let mut numerator = i64::try_from(significand).ok()?;
    let denominator = if power >= 0 {
        numerator = numerator.checked_mul(1_i64 << power)?;
        1
    } else {
        1_i64 << -power
    };
    if value.is_sign_negative() {
        numerator = -numerator;
    }
    Rational::new(numerator, denominator).ok()
}
