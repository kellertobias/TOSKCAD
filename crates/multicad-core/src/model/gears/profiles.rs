use serde::{Deserialize, Serialize};
pub const GEAR_DIMENSIONS_SOURCE_URL: &str = "https://khkgears.net/new/gear_knowledge/gear_technical_reference/calculation_gear_dimensions.html";
pub const GEAR_PROFILE_SOURCE_URL: &str =
    "https://khkgears.us/media/fqrlut1m/abcs-of-gearing-b.pdf";
pub const GEAR_FILLET_SOURCE_URL: &str = "https://www.akgears.com/papers/direct_gear_design.htm";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearFamily {
    Spur,
    Helical,
    Herringbone,
    DoubleHelical,
    InternalRing,
    Rack,
    Bevel,
    Worm,
}
impl GearFamily {
    pub const ALL: [Self; 8] = [
        Self::Spur,
        Self::Helical,
        Self::Herringbone,
        Self::DoubleHelical,
        Self::InternalRing,
        Self::Rack,
        Self::Bevel,
        Self::Worm,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Spur => "Spur",
            Self::Helical => "Helical",
            Self::Herringbone => "Herringbone (continuous center)",
            Self::DoubleHelical => "Double-helical (center gap)",
            Self::InternalRing => "Internal ring",
            Self::Rack => "Rack",
            Self::Bevel => "Bevel",
            Self::Worm => "Worm and wheel",
        }
    }
    pub fn supported(self) -> bool {
        matches!(
            self,
            Self::Spur
                | Self::Helical
                | Self::Herringbone
                | Self::DoubleHelical
                | Self::InternalRing
                | Self::Rack
        )
    }
    pub fn source(self) -> &'static str {
        if self == Self::Rack {
            super::RACK_PROFILE_SOURCE
        } else if self == Self::InternalRing {
            "KHK unshifted 20-degree internal spur geometry; 48/60 teeth, custom circular root fillets; source revision 1"
        } else if self.supported() {
            "KHK unshifted full-depth involute dimensions; custom circular root fillet; source revision 1"
        } else {
            "Not implemented; no family-correct geometry or compatibility coverage claimed"
        }
    }
}
