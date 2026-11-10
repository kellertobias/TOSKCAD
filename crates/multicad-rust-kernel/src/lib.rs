//! Bounded exact lengths and one analytic curve probe for the kernel study.

pub mod annulus;
pub mod axial_profile_stack;
pub mod chamfered_bores;
pub mod circular_pockets;
pub mod cosmetic_fastener;
pub mod curved_projection;
pub mod cylinder;
pub mod display;
mod drawing_events;
pub mod headed_screw;
pub mod helical_thread;
pub mod helical_thread_mesh;
pub mod hex_nut;
pub mod linear_sweep;
pub mod linear_sweep_mesh;
pub mod manufacturing;
pub mod normal_sweep;
pub mod orthogonal;
pub mod orthogonal_boundary;
pub mod orthogonal_pockets;
pub mod orthogonal_section;
pub mod orthogonal_slots;
pub mod planar_boolean;
pub mod planar_distance;
pub mod planar_projection;
pub mod planar_section;
pub mod polygon;
pub mod prism;
pub mod retained_section;
pub mod revolution;
pub mod revolution_mesh;
pub mod round_occlusion;
pub mod section_hatch;
pub mod sketch_reference;
pub mod stadium;
pub mod step;
pub mod step_read;
pub mod straight_rack;
pub mod trim_curve;
pub mod trimmed_sweep;
pub mod warped_union;

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

impl Rational {
    pub fn new(numerator: i64, denominator: i64) -> Result<Self, Error> {
        Self::normalize(numerator.into(), denominator.into())
    }

    pub fn ratio(self) -> (i64, i64) {
        (self.numerator, self.denominator)
    }

    /// Approximate this value for consumers of floating-point coordinates.
    pub fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    fn normalize(numerator: i128, denominator: i128) -> Result<Self, Error> {
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

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn decimal(text: &str) -> Result<Rational, Error> {
    let text = text.trim();
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(Error::InvalidLength);
    }
    let fraction = fraction.trim_end_matches('0');
    let places = u32::try_from(fraction.len()).map_err(|_| Error::OutOfRange)?;
    let denominator = 10_i128.checked_pow(places).ok_or(Error::OutOfRange)?;
    let whole = whole.trim_start_matches('0');
    let magnitude = whole
        .bytes()
        .chain(fraction.bytes())
        .try_fold(0_i128, |acc, b| {
            acc.checked_mul(10)?.checked_add((b - b'0').into())
        })
        .ok_or(Error::OutOfRange)?;
    let numerator = if text.starts_with('-') {
        -magnitude
    } else {
        magnitude
    };
    Rational::normalize(numerator, denominator)
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

/// Read a decimal or single fraction in millimeters or inches without a float
/// conversion. A unit may follow the numerator or the whole fraction.
pub fn parse_length_mm(text: &str) -> Result<Rational, Error> {
    if let Some((value, factor)) = strip_unit(text) {
        let length = match value.split_once('/') {
            Some((a, b)) => decimal(a)?.checked_div(decimal(b)?)?,
            None => decimal(value)?,
        };
        return length.checked_mul(factor);
    }
    let (numerator, denominator) = text.split_once('/').ok_or(Error::InvalidLength)?;
    let (value, factor) = strip_unit(numerator).ok_or(Error::InvalidLength)?;
    decimal(value)?
        .checked_div(decimal(denominator)?)?
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Circle {
    center: [Rational; 2],
    radius: Rational,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalIntersection {
    Disjoint,
    Tangent {
        y: Rational,
    },
    /// The two ordinates are exactly `y_center ± sqrt(radicand)`.
    Secant {
        y_center: Rational,
        radicand: Rational,
    },
}

impl Circle {
    pub fn new(center: [Rational; 2], radius: Rational) -> Result<Self, Error> {
        if radius.numerator <= 0 {
            return Err(Error::NonPositiveRadius);
        }
        Ok(Self { center, radius })
    }

    pub fn center(self) -> [Rational; 2] {
        self.center
    }

    pub fn radius(self) -> Rational {
        self.radius
    }

    pub fn intersect_vertical(self, x: Rational) -> Result<VerticalIntersection, Error> {
        let [center_x, y_center] = self.center;
        let dx = x.checked_sub(center_x)?;
        let radius_squared = self.radius.checked_mul(self.radius)?;
        let radicand = radius_squared.checked_sub(dx.checked_mul(dx)?)?;
        Ok(match radicand.numerator.signum() {
            -1 => VerticalIntersection::Disjoint,
            0 => VerticalIntersection::Tangent { y: y_center },
            _ => VerticalIntersection::Secant { y_center, radicand },
        })
    }
}
