//! Bounded exact arithmetic shared by document and geometry evaluation.
use std::cmp::Ordering;

const MAX_COMPONENT: i128 = 1_000_000_000_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidLength,
    DivisionByZero,
    OutOfRange,
    NonPositiveQuantum,
    NonPositiveRadius,
}

/// A reduced rational with positive denominator and bounded components.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rational {
    numerator: i64,
    denominator: i64,
}

#[cfg(test)]
mod rational_float_tests;

impl Rational {
    pub const fn new(numerator: i64, denominator: i64) -> Result<Self, Error> {
        Self::normalize(numerator as i128, denominator as i128)
    }

    pub fn ratio(self) -> (i64, i64) {
        (self.numerator, self.denominator)
    }

    /// Read a signed ASCII decimal with an optional base-ten exponent.
    /// Numeric tokens are limited to 256 bytes after trimming whitespace.
    pub fn parse_decimal(text: &str) -> Result<Self, Error> {
        decimal(text.trim())
    }

    /// Round the exact ratio once to the nearest binary64, ties to even.
    pub fn as_f64(self) -> f64 {
        let magnitude = self.numerator.unsigned_abs();
        // These integer operands are exactly representable in binary64, so
        // hardware division already rounds the exact ratio just once.
        if magnitude <= 1_u64 << 53 && self.denominator <= 1_i64 << 53 {
            return self.numerator as f64 / self.denominator as f64;
        }
        let numerator = u128::from(magnitude);
        let denominator = self.denominator as u128;
        let mut exponent = denominator.leading_zeros() as i32 - numerator.leading_zeros() as i32;
        let below_power = if exponent >= 0 {
            numerator < denominator << exponent
        } else {
            numerator << -exponent < denominator
        };
        exponent -= i32::from(below_power);
        let shift = 52 - exponent;
        let (scaled, divisor) = if shift >= 0 {
            (numerator << shift, denominator)
        } else {
            (numerator, denominator << -shift)
        };
        // Components are bounded by 10^18: exponents lie in [-60,59],
        // and the scaled numerator has at most 113 bits. No float conversion
        // or approximate division participates in the rounding decision.
        let quotient = scaled / divisor;
        let twice_remainder = 2 * (scaled % divisor);
        let round_up =
            twice_remainder > divisor || (twice_remainder == divisor && quotient & 1 != 0);
        let significand = (quotient + u128::from(round_up)) as u64;
        // Keeping the implicit leading bit makes a rounded 53-bit carry
        // advance the exponent automatically.
        let magnitude = (((exponent + 1022) as u64) << 52) + significand;
        let sign = u64::from(self.numerator < 0) << 63;
        f64::from_bits(sign | magnitude)
    }

    const fn normalize(numerator: i128, denominator: i128) -> Result<Self, Error> {
        if denominator == 0 {
            return Err(Error::DivisionByZero);
        }
        let (mut a, mut b) = (numerator.abs(), denominator.abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let denominator_positive = denominator.abs() / a;
        let numerator_magnitude = numerator.abs() / a;
        if numerator_magnitude > MAX_COMPONENT || denominator_positive > MAX_COMPONENT {
            return Err(Error::OutOfRange);
        }
        let sign = numerator.signum() * denominator.signum();
        Ok(Self {
            numerator: (sign * numerator_magnitude) as i64,
            denominator: denominator_positive as i64,
        })
    }

    fn wide(self) -> (i128, i128) {
        (self.numerator.into(), self.denominator.into())
    }

    // With both reduced components bounded by 10^18, these intermediate
    // products and sums stay within the signed 128-bit range.
    pub fn checked_add(self, other: Self) -> Result<Self, Error> {
        let ((a, b), (c, d)) = (self.wide(), other.wide());
        Self::normalize(a * d + c * b, b * d)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, Error> {
        let ((a, b), (c, d)) = (self.wide(), other.wide());
        Self::normalize(a * d - c * b, b * d)
    }

    pub fn checked_mul(self, other: Self) -> Result<Self, Error> {
        let ((a, b), (c, d)) = (self.wide(), other.wide());
        Self::normalize(a * c, b * d)
    }

    pub fn checked_div(self, other: Self) -> Result<Self, Error> {
        let ((a, b), (c, d)) = (self.wide(), other.wide());
        Self::normalize(a * d, b * c)
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        let ((a, b), (c, d)) = (self.wide(), other.wide());
        (a * d).cmp(&(b * c))
    }
}

impl std::ops::Neg for Rational {
    type Output = Self;

    fn neg(self) -> Self {
        Self {
            numerator: -self.numerator,
            ..self
        }
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const MAX_DECIMAL_BYTES: usize = 4096;
// An unreduced numerator can grow by at most 10^places. With a nonzero
// final digit, at least 2^places remains in the reduced denominator.
const MAX_DECIMAL_DIGITS: usize = 1 + (MAX_COMPONENT.ilog10() + MAX_COMPONENT.ilog2()) as usize;

fn decimal(text: &str) -> Result<Rational, Error> {
    if text.len() > MAX_DECIMAL_BYTES {
        return Err(Error::OutOfRange);
    }
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text, None), |(mantissa, exponent)| {
            (mantissa, Some(exponent))
        });
    let unsigned = mantissa.strip_prefix(['+', '-']).unwrap_or(mantissa);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(Error::InvalidLength);
    }
    let exponent = exponent.map_or(Ok(0), decimal_exponent)?;
    let count = whole.len() + fraction.len();
    let bytes = || whole.bytes().chain(fraction.bytes());
    let Some(first) = bytes().position(|byte| byte != b'0') else {
        return Rational::new(0, 1);
    };
    let trailing = bytes().rev().take_while(|&byte| byte == b'0').count();
    let mut storage = [0; MAX_DECIMAL_DIGITS];
    let digits = storage
        .get_mut(..count - first - trailing)
        .ok_or(Error::OutOfRange)?;
    for (digit, byte) in digits.iter_mut().zip(bytes().skip(first)) {
        *digit = byte - b'0';
    }
    let shift = i64::from(exponent) - fraction.len() as i64 + trailing as i64;
    let (magnitude, denominator) = if shift >= 0 {
        if shift > i64::from(MAX_COMPONENT.ilog10()) {
            return Err(Error::OutOfRange);
        }
        (decimal_component(digits)? * 10_i128.pow(shift as u32), 1)
    } else {
        decimal_fraction(digits, -shift)?
    };
    let numerator = if text.starts_with('-') {
        -magnitude
    } else {
        magnitude
    };
    Rational::normalize(numerator, denominator)
}

fn decimal_exponent(text: &str) -> Result<i32, Error> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::InvalidLength);
    }
    // A larger exponent cannot place a nonzero, bounded mantissa in range.
    // Saturation lets an authored zero retain its value with any valid exponent.
    Ok(text.parse().unwrap_or_else(|_| {
        if text.starts_with('-') {
            i32::MIN
        } else {
            i32::MAX
        }
    }))
}

fn decimal_component(digits: &[u8]) -> Result<i128, Error> {
    digits
        .iter()
        .try_fold(0_i128, |value, &digit| {
            let value = value * 10 + i128::from(digit);
            (value <= MAX_COMPONENT).then_some(value)
        })
        .ok_or(Error::OutOfRange)
}

fn decimal_fraction(digits: &mut [u8], places: i64) -> Result<(i128, i128), Error> {
    // The last significant digit is nonzero, so at least one prime retains
    // all of its powers in the reduced denominator. Two gives the looser cap.
    if places > i64::from(MAX_COMPONENT.ilog2()) {
        return Err(Error::OutOfRange);
    }
    let mut twos = places as u32;
    let mut fives = twos;
    // Divide the decimal digits directly. The unreduced significand can
    // exceed i128 even when both reduced components fit.
    for (prime, remaining) in [(2, &mut twos), (5, &mut fives)] {
        while *remaining > 0 && digits[digits.len() - 1].is_multiple_of(prime) {
            let mut carry = 0;
            for digit in &mut *digits {
                let value = carry * 10 + *digit;
                *digit = value / prime;
                carry = value % prime;
            }
            *remaining -= 1;
        }
    }
    if fives > MAX_COMPONENT.ilog(5) {
        return Err(Error::OutOfRange);
    }
    Ok((
        decimal_component(digits)?,
        2_i128.pow(twos) * 5_i128.pow(fives),
    ))
}

fn strip_unit(text: &str) -> Option<(&str, Rational)> {
    const MILLIMETER: Rational = Rational {
        numerator: 1,
        denominator: 1,
    };
    const INCH: Rational = Rational {
        numerator: 127,
        denominator: 5,
    };
    let text = text.trim();
    if let Some(value) = text.strip_suffix("mm") {
        Some((value, MILLIMETER))
    } else {
        text.strip_suffix("in").map(|value| (value, INCH))
    }
}

/// Read a decimal, optionally in scientific notation, or a single fraction
/// in millimeters or inches. A unit may follow the numerator or the fraction.
pub fn parse_length_mm(text: &str) -> Result<Rational, Error> {
    if let Some((value, factor)) = strip_unit(text) {
        let length = match value.split_once('/') {
            Some((a, b)) => Rational::parse_decimal(a)?.checked_div(Rational::parse_decimal(b)?)?,
            None => Rational::parse_decimal(value)?,
        };
        return length.checked_mul(factor);
    }
    let (numerator, denominator) = text.split_once('/').ok_or(Error::InvalidLength)?;
    let (value, factor) = strip_unit(numerator).ok_or(Error::InvalidLength)?;
    Rational::parse_decimal(value)?
        .checked_div(Rational::parse_decimal(denominator)?)?
        .checked_mul(factor)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridCoordinate {
    Ticks(i64),
    OffGrid(Rational),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Quantum(Rational);

impl Quantum {
    pub fn new(step_mm: Rational) -> Result<Self, Error> {
        if step_mm.numerator <= 0 {
            return Err(Error::NonPositiveQuantum);
        }
        Ok(Self(step_mm))
    }

    pub fn classify(self, length_mm: Rational) -> Result<GridCoordinate, Error> {
        let ((a, b), (c, d)) = (length_mm.wide(), self.0.wide());
        let (numerator, denominator) = (a * d, b * c);
        if numerator % denominator == 0 {
            let ticks = i64::try_from(numerator / denominator).map_err(|_| Error::OutOfRange)?;
            Ok(GridCoordinate::Ticks(ticks))
        } else {
            Ok(GridCoordinate::OffGrid(length_mm))
        }
    }
}
