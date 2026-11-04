//! Bounded nominal Rd contours; source rounding remains distinct from tangency.
use super::*;
pub const ROUND_THREAD_CAPABILITY: &str = "model.thread.round.1";
pub const ROUND_THREAD_SOURCE: &str = "Mitsubishi Materials C010B G009 round/DIN405 nominal profile; BAER 2022 p134 named sizes; no DIN405-2 tolerance class or full DIN compliance";
pub const ROUND_PROFILE_SOURCE_URL: &str =
    "https://data.mmc-carbide.com/4116/8197/5012/catalog_c010b_threading.pdf";
pub const ROUND_SIZE_SOURCE_URL: &str = "https://www.gewindebohrer.de/_Resources/Persistent/281607539e8454e0872443e825ba712c98d64b82/Gewindebohrer_Katalog_2022_DE.pdf";
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoundThreadPreset {
    pub designation: &'static str,
    pub diameter_mm: f64,
    pub threads_per_inch: u32,
    pub pitch_mm: f64,
    pub manufacturer_tap: &'static str,
    pub manufacturer_die: &'static str,
    pub source: &'static str,
    pub profile_source_url: &'static str,
    pub size_source_url: &'static str,
}
pub fn round_thread_presets() -> Vec<RoundThreadPreset> {
    [
        (10., 10, "Rd10 × 1/10″", "350501003", "351401001"),
        (20., 8, "Rd20 × 1/8″", "350501009", "351401007"),
    ]
    .into_iter()
    .map(|(diameter, tpi, label, tap, die)| RoundThreadPreset {
        designation: label,
        diameter_mm: diameter,
        threads_per_inch: tpi,
        pitch_mm: 25.4 / f64::from(tpi),
        manufacturer_tap: tap,
        manufacturer_die: die,
        source: ROUND_THREAD_SOURCE,
        profile_source_url: ROUND_PROFILE_SOURCE_URL,
        size_source_url: ROUND_SIZE_SOURCE_URL,
    })
    .collect()
}
impl RoundThreadPreset {
    pub fn parameters(&self) -> ThreadParameters {
        ThreadParameters {
            family: ThreadFamily::Round,
            major_diameter: length(self.diameter_mm),
            pitch: Dimension::Literal {
                value: 1. / f64::from(self.threads_per_inch),
                unit: "in".into(),
            },
            length: length(3. * self.pitch_mm),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoundPublishedCoefficients {
    pub external_material_crest_and_root: f64,
    pub internal_material_crest: f64,
    pub internal_material_root: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoundCircle {
    pub radius_mm: f64,
    pub center_radius_mm: f64,
    pub center_axial_mm: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedRoundThread {
    pub preset: RoundThreadPreset,
    pub designation: String,
    pub nominal_pitch_diameter_mm: f64,
    pub nominal_construction_major_diameter_mm: f64,
    pub nominal_construction_minor_diameter_mm: f64,
    pub profile_depth_mm: f64,
    pub source_radial_crest_clearance_mm: f64,
    pub cutter_crest_radius_mm: f64,
    pub cutter_root_radius_mm: f64,
    pub material_crest_radius_mm: f64,
    pub material_root_radius_mm: f64,
    pub actual_cutter_crest_circle: RoundCircle,
    pub actual_cutter_root_circle: RoundCircle,
    pub printed_coefficients: RoundPublishedCoefficients,
    pub scope: &'static str,
}
pub(super) fn resolve_round(
    diameter: f64,
    pitch: f64,
    mode: ThreadMode,
    radial: f64,
    axial: f64,
    hand: ThreadHand,
) -> Result<ResolvedRoundThread, DocumentError> {
    let preset = round_thread_presets()
        .into_iter()
        .find(|p| (p.diameter_mm - diameter).abs() < 1e-9 && (p.pitch_mm - pitch).abs() < 1e-9)
        .ok_or_else(|| invalid("Select a listed Rd nominal diameter and reciprocal-inch pitch"))?;
    if axial != 0. {
        return Err(invalid(
            "True circular Rd profiles require zero axial fabrication allowance",
        ));
    }
    let a = 15_f64.to_radians();
    let base = pitch * (a.cos() - a.sin()) / (4. * (1. - a.sin()));
    let clearance = 0.05 * pitch;
    let delta = clearance * a.sin() / (1. - a.sin());
    let internal = mode == ThreadMode::CutInternal;
    let crest = base - if internal { delta } else { 0. };
    let root = base + if internal { delta } else { 0. };
    let depth = pitch / 2.;
    let major = diameter + if internal { 2. * clearance } else { 0. };
    let actual_major = major / 2. + if internal { radial } else { -radial };
    Ok(ResolvedRoundThread {
        designation: format!(
            "{} {}, single start",
            preset.designation,
            if hand == ThreadHand::Right {
                "RH"
            } else {
                "LH"
            }
        ),
        preset,
        nominal_pitch_diameter_mm: diameter - pitch / 2.,
        nominal_construction_major_diameter_mm: major,
        nominal_construction_minor_diameter_mm: major - pitch,
        profile_depth_mm: depth,
        source_radial_crest_clearance_mm: clearance,
        cutter_crest_radius_mm: crest,
        cutter_root_radius_mm: root,
        material_crest_radius_mm: if internal { root } else { crest },
        material_root_radius_mm: if internal { crest } else { root },
        actual_cutter_crest_circle: RoundCircle {
            radius_mm: crest,
            center_radius_mm: actual_major - crest,
            center_axial_mm: 0.,
        },
        actual_cutter_root_circle: RoundCircle {
            radius_mm: root,
            center_radius_mm: actual_major - depth + root,
            center_axial_mm: pitch / 2.,
        },
        printed_coefficients: RoundPublishedCoefficients {
            external_material_crest_and_root: 0.238507,
            internal_material_crest: 0.255967,
            internal_material_root: 0.221047,
        },
        scope: "Source nominal contour, exact tangent circles with printed coefficients retained separately; RH/LH engineering geometry, one start; no fitted tolerance class, full DIN compliance, physical fit or load certification",
    })
}
