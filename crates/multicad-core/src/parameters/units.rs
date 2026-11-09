use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Dimension {
    pub length: i8,
    pub angle: i8,
    pub count: i8,
}
impl Dimension {
    pub const RATIO: Self = Self {
        length: 0,
        angle: 0,
        count: 0,
    };
    pub const LENGTH: Self = Self {
        length: 1,
        ..Self::RATIO
    };
    pub const ANGLE: Self = Self {
        angle: 1,
        ..Self::RATIO
    };
    pub const COUNT: Self = Self {
        count: 1,
        ..Self::RATIO
    };
    pub(crate) fn combine(self, other: Self, sign: i8) -> Option<Self> {
        Some(Self {
            length: self.length.checked_add(other.length.checked_mul(sign)?)?,
            angle: self.angle.checked_add(other.angle.checked_mul(sign)?)?,
            count: self.count.checked_add(other.count.checked_mul(sign)?)?,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Unit {
    Millimeter,
    Centimeter,
    Meter,
    Inch,
    Foot,
    Degree,
    Radian,
    Count,
    TeethPerInch,
    Ratio,
    Percent,
}
impl Unit {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "mm" => Self::Millimeter,
            "cm" => Self::Centimeter,
            "m" => Self::Meter,
            "in" => Self::Inch,
            "ft" => Self::Foot,
            "deg" => Self::Degree,
            "rad" => Self::Radian,
            "count" => Self::Count,
            "teeth_per_in" => Self::TeethPerInch,
            "ratio" => Self::Ratio,
            "%" => Self::Percent,
            _ => return None,
        })
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Millimeter => "mm",
            Self::Centimeter => "cm",
            Self::Meter => "m",
            Self::Inch => "in",
            Self::Foot => "ft",
            Self::Degree => "deg",
            Self::Radian => "rad",
            Self::Count => "count",
            Self::TeethPerInch => "teeth_per_in",
            Self::Ratio => "ratio",
            Self::Percent => "%",
        }
    }
    pub fn dimension(self) -> Dimension {
        match self {
            Self::Millimeter | Self::Centimeter | Self::Meter | Self::Inch | Self::Foot => {
                Dimension::LENGTH
            }
            Self::Degree | Self::Radian => Dimension::ANGLE,
            Self::Count => Dimension::COUNT,
            Self::TeethPerInch => Dimension {
                length: -1,
                angle: 0,
                count: 1,
            },
            Self::Ratio | Self::Percent => Dimension::RATIO,
        }
    }
    pub fn scale(self) -> f64 {
        match self {
            Self::Millimeter => 0.001,
            Self::Centimeter => 0.01,
            Self::Meter | Self::Radian | Self::Count | Self::Ratio => 1.0,
            Self::Inch => 0.0254,
            Self::TeethPerInch => 1. / 0.0254,
            Self::Foot => 0.3048,
            Self::Degree => std::f64::consts::PI / 180.0,
            Self::Percent => 0.01,
        }
    }
}

/// Present degrees without the last-place noise of conversion from radians.
/// Keep the full value when trimming would lose more than floating-point noise.
pub fn format_degrees(value: f64) -> String {
    let decimal = format!("{value:.12}");
    if decimal
        .parse::<f64>()
        .is_ok_and(|rounded| (rounded - value).abs() <= value.abs() * f64::EPSILON)
    {
        decimal.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        value.to_string()
    }
}
/// A query result, never an accumulated transform or stored mesh coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Quantity {
    pub canonical: f64,
    pub dimension: Dimension,
}
impl Quantity {
    pub fn in_unit(self, unit: Unit) -> Option<f64> {
        (self.dimension == unit.dimension()).then_some(self.canonical / unit.scale())
    }
}
