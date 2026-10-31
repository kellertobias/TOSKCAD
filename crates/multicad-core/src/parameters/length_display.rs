//! User-facing default length unit (CAD-187). It is a presentation and input
//! default only: canonical geometry stays in typed quantities, and values
//! written from input keep the unit they were entered in, so switching the
//! preference never reinterprets stored dimensions or expressions.
use super::{Dimension, Unit};

/// Length unit offered as the application-wide display/input default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LengthUnit {
    #[default]
    Millimeter,
    Inch,
}
impl LengthUnit {
    pub const ALL: [Self; 2] = [Self::Millimeter, Self::Inch];
    pub fn unit(self) -> Unit {
        match self {
            Self::Millimeter => Unit::Millimeter,
            Self::Inch => Unit::Inch,
        }
    }
    /// Stable unit symbol, also the persisted preference value.
    pub fn symbol(self) -> &'static str {
        self.unit().symbol()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Millimeter => "Millimeters (mm)",
            Self::Inch => "Inches (in)",
        }
    }
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.symbol() == symbol)
    }
    /// Decimal places that keep a displayed value at roughly micrometre
    /// resolution without noise.
    pub fn precision(self) -> usize {
        match self {
            Self::Millimeter => 3,
            Self::Inch => 4,
        }
    }
    /// Convert millimetres into this unit.
    pub fn from_mm(self, millimeters: f64) -> f64 {
        millimeters * Unit::Millimeter.scale() / self.unit().scale()
    }
    /// Convert a value in this unit into millimetres.
    pub fn to_mm(self, value: f64) -> f64 {
        value * self.unit().scale() / Unit::Millimeter.scale()
    }
    /// Format a millimetre quantity with this unit's label, e.g. `1.0000 in`.
    pub fn format_mm(self, millimeters: f64) -> String {
        format!(
            "{:.*} {}",
            self.precision(),
            normalized(self.from_mm(millimeters)),
            self.symbol()
        )
    }
    /// Signed variant for component deltas, e.g. `+0.500 mm`.
    pub fn format_signed_mm(self, millimeters: f64) -> String {
        format!(
            "{:+.*} {}",
            self.precision(),
            normalized(self.from_mm(millimeters)),
            self.symbol()
        )
    }
}
fn normalized(value: f64) -> f64 {
    if value == 0. { 0. } else { value }
}

/// A length literal parsed from user input. The value stays in the unit it
/// was entered in so the document records exactly what the user typed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthInput {
    pub value: f64,
    pub unit: Unit,
}
impl LengthInput {
    pub fn millimeters(self) -> f64 {
        self.value * self.unit.scale() / Unit::Millimeter.scale()
    }
}

/// Parse a length literal. Unqualified numbers use `default`; an explicit
/// suffix (`mm`, `cm`, `m`, `in`, `"`, `ft`, `'`) keeps its own meaning.
/// Commas are accepted as decimal separators. Non-length units, formulas and
/// nonfinite values are rejected.
pub fn parse_length(text: &str, default: LengthUnit) -> Result<LengthInput, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a length".into());
    }
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(text.len());
    let (number, suffix) = text.split_at(split);
    let number = number.trim().replace(',', ".");
    let suffix = suffix.trim();
    let unit = match suffix {
        "" => default.unit(),
        "\"" => Unit::Inch,
        "'" => Unit::Foot,
        other => Unit::parse(other).ok_or_else(|| format!("Unknown length unit '{other}'"))?,
    };
    if unit.dimension() != Dimension::LENGTH {
        return Err(format!("'{}' is not a length unit", unit.symbol()));
    }
    let value: f64 = number
        .parse()
        .map_err(|_| format!("'{text}' is not a number"))?;
    if !value.is_finite() {
        return Err("Enter a finite length".into());
    }
    Ok(LengthInput { value, unit })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unqualified_input_uses_the_preference_and_qualified_input_keeps_its_unit() {
        let inch = parse_length("2", LengthUnit::Inch).unwrap();
        assert_eq!(inch.unit, Unit::Inch);
        assert!((inch.millimeters() - 50.8).abs() < 1e-9);
        let mm = parse_length("2", LengthUnit::Millimeter).unwrap();
        assert_eq!((mm.value, mm.unit), (2., Unit::Millimeter));
        for (text, millimeters, unit) in [
            ("25 mm", 25., Unit::Millimeter),
            ("25mm", 25., Unit::Millimeter),
            ("1 in", 25.4, Unit::Inch),
            ("1\"", 25.4, Unit::Inch),
            ("2,5 cm", 25., Unit::Centimeter),
            ("-0.5 m", -500., Unit::Meter),
            ("1 ft", 304.8, Unit::Foot),
            ("1'", 304.8, Unit::Foot),
            ("1e1 mm", 10., Unit::Millimeter),
        ] {
            // The preference never changes explicitly qualified input.
            for default in LengthUnit::ALL {
                let parsed = parse_length(text, default).unwrap();
                assert_eq!(parsed.unit, unit, "{text}");
                assert!((parsed.millimeters() - millimeters).abs() < 1e-9, "{text}");
            }
        }
    }
    #[test]
    fn invalid_length_input_is_rejected() {
        for text in [
            "", "abc", "5 deg", "5 %", "5 yards", "1..2", "inf", "=$width",
        ] {
            assert!(parse_length(text, LengthUnit::Inch).is_err(), "{text}");
        }
    }
    #[test]
    fn formatting_converts_without_changing_the_quantity() {
        assert_eq!(LengthUnit::Millimeter.format_mm(25.4), "25.400 mm");
        assert_eq!(LengthUnit::Inch.format_mm(25.4), "1.0000 in");
        assert_eq!(LengthUnit::Inch.format_signed_mm(-12.7), "-0.5000 in");
        assert_eq!(LengthUnit::Inch.format_mm(-0.), "0.0000 in");
        for unit in LengthUnit::ALL {
            assert!((unit.to_mm(unit.from_mm(123.456)) - 123.456).abs() < 1e-9);
            assert_eq!(LengthUnit::from_symbol(unit.symbol()), Some(unit));
        }
        assert_eq!(LengthUnit::from_symbol("cm"), None);
        assert_eq!(LengthUnit::default(), LengthUnit::Millimeter);
    }
}
