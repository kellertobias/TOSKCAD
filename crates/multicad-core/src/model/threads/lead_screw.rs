//! Pinned basic lead-screw profiles; fit-class limits remain separate.
use super::*;

pub const LEAD_SCREW_CAPABILITY: &str = "model.thread.lead_screw.1";
pub const TRAPEZOIDAL_THREAD_SOURCE: &str = "ISO 2901:2016 design profiles and ISO 2904:2020 basic dimensions; nominal sharp limiting profile, no ISO 2903 tolerance class";
pub const TRAPEZOIDAL_PROFILE_SOURCE_URL: &str = "https://cdn.standards.iteh.ai/samples/68337/c87112d8e3dc487c95fd964d13101ca4/ISO-2901-2016.pdf";
pub const TRAPEZOIDAL_THREAD_SOURCE_URL: &str = "https://cdn.standards.iteh.ai/samples/78729/7d158726108b478c889fa887d460ba78/ISO-2904-2020.pdf";
pub const ACME_THREAD_SOURCE: &str = "NBS H28(1957) Part III, issued 1960-10-07, section XII basic form plus general-purpose crest/root clearance; nominal geometry without a G-class fit";
pub const ACME_THREAD_SOURCE_URL: &str =
    "https://nvlpubs.nist.gov/nistpubs/Legacy/hb/nbshandbook28supp1957pt3.pdf";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LeadScrewPreset {
    pub family: ThreadFamily,
    pub designation: &'static str,
    pub diameter_mm: f64,
    pub pitch_mm: f64,
    pub nominal_diameter_inch: Option<f64>,
    pub threads_per_inch: Option<u32>,
    pub included_angle_deg: f64,
    /// Source clearance on radius, separate from fabrication allowance.
    pub basic_radial_clearance_mm: f64,
    pub basic_pitch_diameter_mm: f64,
    pub external_minor_diameter_mm: f64,
    pub internal_major_diameter_mm: f64,
    pub internal_minor_diameter_mm: f64,
    pub source: &'static str,
    pub source_url: &'static str,
    pub profile_source_url: &'static str,
}
pub fn lead_screw_presets() -> Vec<LeadScrewPreset> {
    let metric = [(10., 2., "Tr10 × 2"), (12., 3., "Tr12 × 3")].map(|(d, p, label)| {
        let clearance = 0.25;
        LeadScrewPreset {
            family: ThreadFamily::Trapezoidal,
            designation: label,
            diameter_mm: d,
            pitch_mm: p,
            nominal_diameter_inch: None,
            threads_per_inch: None,
            included_angle_deg: 30.,
            basic_radial_clearance_mm: clearance,
            basic_pitch_diameter_mm: d - p / 2.,
            external_minor_diameter_mm: d - p - 2. * clearance,
            internal_major_diameter_mm: d + 2. * clearance,
            internal_minor_diameter_mm: d - p,
            source: TRAPEZOIDAL_THREAD_SOURCE,
            source_url: TRAPEZOIDAL_THREAD_SOURCE_URL,
            profile_source_url: TRAPEZOIDAL_PROFILE_SOURCE_URL,
        }
    });
    let inch = [(0.375, 12, "3/8-12 Acme"), (0.5, 10, "1/2-10 Acme")].map(|(d, tpi, label)| {
        let p = 25.4 / f64::from(tpi);
        let diameter = d * 25.4;
        let clearance = if tpi <= 10 { 0.010 } else { 0.005 } * 25.4;
        LeadScrewPreset {
            family: ThreadFamily::Acme,
            designation: label,
            diameter_mm: diameter,
            pitch_mm: p,
            nominal_diameter_inch: Some(d),
            threads_per_inch: Some(tpi),
            included_angle_deg: 29.,
            basic_radial_clearance_mm: clearance,
            basic_pitch_diameter_mm: diameter - p / 2.,
            external_minor_diameter_mm: diameter - p - 2. * clearance,
            internal_major_diameter_mm: diameter + 2. * clearance,
            internal_minor_diameter_mm: diameter - p,
            source: ACME_THREAD_SOURCE,
            source_url: ACME_THREAD_SOURCE_URL,
            profile_source_url: ACME_THREAD_SOURCE_URL,
        }
    });
    metric.into_iter().chain(inch).collect()
}
impl LeadScrewPreset {
    pub fn parameters(&self) -> ThreadParameters {
        let major_diameter = self.nominal_diameter_inch.map_or_else(
            || length(self.diameter_mm),
            |value| Dimension::Literal {
                value,
                unit: "in".into(),
            },
        );
        let pitch = self.threads_per_inch.map_or_else(
            || length(self.pitch_mm),
            |tpi| Dimension::Literal {
                value: 1. / f64::from(tpi),
                unit: "in".into(),
            },
        );
        ThreadParameters {
            family: self.family,
            major_diameter,
            pitch,
            length: length(3. * self.pitch_mm),
            ..Default::default()
        }
    }
    pub fn construction(&self, mode: ThreadMode) -> (f64, ResolvedThreadProfile) {
        let internal = mode == ThreadMode::CutInternal;
        let major = if internal {
            self.internal_major_diameter_mm
        } else {
            self.diameter_mm
        };
        let minor = if internal {
            self.internal_minor_diameter_mm
        } else {
            self.external_minor_diameter_mm
        };
        let half = self.included_angle_deg / 2.;
        (
            major / 2.,
            ResolvedThreadProfile {
                depth_mm: (major - minor) / 2.,
                crest_width_mm: self.pitch_mm / 2.
                    - (major - self.basic_pitch_diameter_mm) * half.to_radians().tan(),
                lower_flank_deg: half,
                upper_flank_deg: half,
                radial_taper: 0.,
                rounding_radius_mm: None,
            },
        )
    }
}
pub(super) fn lead_screw_preset(
    family: ThreadFamily,
    diameter: f64,
    pitch: f64,
) -> Result<LeadScrewPreset, DocumentError> {
    lead_screw_presets().into_iter().find(|p|p.family==family&&(p.diameter_mm-diameter).abs()<1e-9&&(p.pitch_mm-pitch).abs()<1e-9)
        .ok_or_else(||invalid("Choose a listed basic Trapezoidal or Acme diameter/pitch; unlisted sizes and other variants are unavailable"))
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedLeadScrew {
    pub designation: String,
    pub preset: LeadScrewPreset,
    pub nominal_pitch_diameter_mm: f64,
    pub nominal_construction_major_diameter_mm: f64,
    pub nominal_construction_minor_diameter_mm: f64,
    pub nominal_depth_mm: f64,
    pub nominal_crest_width_mm: f64,
    pub lead_mm: f64,
    pub starts: u32,
    pub hand: ThreadHand,
    pub scope: &'static str,
}
pub(super) fn lead_screw_receipt(
    preset: LeadScrewPreset,
    mode: ThreadMode,
    starts: u32,
    hand: ThreadHand,
) -> ResolvedLeadScrew {
    let (radius, profile) = preset.construction(mode);
    ResolvedLeadScrew {
        designation: format!(
            "{} · {starts} starts · lead {:.6} mm · {} · nominal basic",
            preset.designation,
            preset.pitch_mm * f64::from(starts),
            if hand == ThreadHand::Left { "LH" } else { "RH" }
        ),
        nominal_pitch_diameter_mm: preset.basic_pitch_diameter_mm,
        nominal_construction_major_diameter_mm: 2. * radius,
        nominal_construction_minor_diameter_mm: 2. * (radius - profile.depth_mm),
        nominal_depth_mm: profile.depth_mm,
        nominal_crest_width_mm: profile.crest_width_mm,
        lead_mm: preset.pitch_mm * f64::from(starts),
        starts,
        hand,
        preset,
        scope: "Nominal sharp limiting profile and source crest/root clearance. Equal basic pitch diameters have zero theoretical flank allowance. Added fabrication allowance is separate. Multiple starts are engineering geometry, not a certified fit class or physical mating result.",
    }
}
