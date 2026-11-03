//! Named BIS nominal buttress form, including its asymmetric root and clearance.
use super::*;
pub const BUTTRESS_THREAD_CAPABILITY: &str = "model.thread.buttress.1";
pub const BUTTRESS_THREAD_SOURCE: &str = "BIS IS 4696 Parts 1/2/3:2004 nominal metric ST 3/30-degree form; no 7e/7H fit class or current DIN compliance";
pub const BUTTRESS_PROFILE_SOURCE_URL: &str =
    "https://law.resource.org/pub/in/bis/S01/is.4696.1.2004.pdf";
pub const BUTTRESS_THREAD_SOURCE_URL: &str =
    "https://law.resource.org/pub/in/bis/S01/is.4696.3.2004.pdf";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtressSeries {
    #[serde(rename = "bis_4696_nominal_2004")]
    Bis4696Nominal2004,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtressLoadFlankSide {
    NegativeAxisSide,
    PositiveAxisSide,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtressIntent {
    pub series: ButtressSeries,
    /// Axial side of the external tooth's 3-degree face, not applied force.
    /// The mating material normal reverses after the internal subtraction.
    pub load_flank_side: ButtressLoadFlankSide,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ButtressPublishedDimensions {
    pub external_pitch_mm: f64,
    pub internal_pitch_mm: f64,
    pub external_minor_mm: f64,
    pub internal_minor_mm: f64,
    pub external_depth_mm: f64,
    pub engagement_height_mm: f64,
    pub root_radius_mm: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ButtressPreset {
    pub series: ButtressSeries,
    pub designation: &'static str,
    pub diameter_mm: f64,
    pub pitch_mm: f64,
    pub printed: ButtressPublishedDimensions,
    pub source: &'static str,
    pub source_url: &'static str,
    pub profile_source_url: &'static str,
}
pub fn buttress_thread_presets() -> Vec<ButtressPreset> {
    vec![ButtressPreset {
        series: ButtressSeries::Bis4696Nominal2004,
        designation: "ST20 × 4",
        diameter_mm: 20.,
        pitch_mm: 4.,
        printed: ButtressPublishedDimensions {
            external_pitch_mm: 17.,
            internal_pitch_mm: 17.635,
            external_minor_mm: 13.058,
            internal_minor_mm: 14.,
            external_depth_mm: 3.471,
            engagement_height_mm: 3.,
            root_radius_mm: 0.497,
        },
        source: BUTTRESS_THREAD_SOURCE,
        source_url: BUTTRESS_THREAD_SOURCE_URL,
        profile_source_url: BUTTRESS_PROFILE_SOURCE_URL,
    }]
}
impl ButtressPreset {
    pub fn parameters(&self, side: ButtressLoadFlankSide) -> ThreadParameters {
        ThreadParameters {
            family: ThreadFamily::Buttress,
            buttress: Some(ButtressIntent {
                series: self.series,
                load_flank_side: side,
            }),
            major_diameter: length(self.diameter_mm),
            pitch: length(self.pitch_mm),
            length: length(3. * self.pitch_mm),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ButtressRootCircle {
    pub radius_mm: f64,
    pub center_axial_mm: f64,
    pub center_radius_mm: f64,
    pub lower_tangent_axial_mm: f64,
    pub lower_tangent_radius_mm: f64,
    pub upper_tangent_axial_mm: f64,
    pub upper_tangent_radius_mm: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedButtressThread {
    pub intent: ButtressIntent,
    pub designation: String,
    pub preset: ButtressPreset,
    pub basic_pitch_diameter_mm: f64,
    pub nominal_construction_pitch_diameter_mm: f64,
    pub nominal_construction_major_diameter_mm: f64,
    pub nominal_construction_minor_diameter_mm: f64,
    pub nominal_engagement_height_mm: f64,
    pub nominal_radial_root_clearance_mm: f64,
    pub axial_relief_clearance_mm: f64,
    pub crest_phase_axial_mm: f64,
    pub phase_rotation_rad: f64,
    pub lower_flank_deg: f64,
    pub upper_flank_deg: f64,
    pub depth_mm: f64,
    pub crest_width_mm: f64,
    /// Actual external circle, after radial fabrication allowance; absent on
    /// the flat internal cutter. Its axial coordinates use the crest datum.
    pub root_circle: Option<ButtressRootCircle>,
    pub scope: &'static str,
}
pub(super) fn resolve_buttress(
    intent: ButtressIntent,
    mode: ThreadMode,
    diameter: f64,
    pitch: f64,
    radial: f64,
    axial: f64,
    hand: ThreadHand,
) -> Result<ResolvedButtressThread, DocumentError> {
    let preset = buttress_thread_presets()
        .into_iter()
        .find(|p| {
            p.series == intent.series
                && (p.diameter_mm - diameter).abs() < 1e-9
                && (p.pitch_mm - pitch).abs() < 1e-9
        })
        .ok_or_else(|| invalid("Select a listed BIS 2004 nominal ST buttress diameter/pitch"))?;
    if axial != 0. {
        return Err(invalid(
            "Circular-root buttress currently requires zero axial fabrication allowance",
        ));
    }
    let lower_side = intent.load_flank_side == ButtressLoadFlankSide::NegativeAxisSide;
    let (lower_flank_deg, upper_flank_deg) = if lower_side {
        (3_f64, 30_f64)
    } else {
        (30_f64, 3_f64)
    };
    let lower = lower_flank_deg.to_radians();
    let upper = upper_flank_deg.to_radians();
    let sum = lower.tan() + upper.tan();
    let width = (0.5 - 0.375 * sum) * pitch;
    let radius = 0.12427 * pitch;
    let intersection = (pitch - width) / sum;
    let half = (lower + upper) / 2.;
    let tilt = (upper - lower) / 2.;
    let cz = width / 2. + intersection * upper.tan() - radius * tilt.sin() / half.sin();
    let cr = diameter / 2. - intersection + radius * tilt.cos() / half.sin();
    let external_depth = diameter / 2. - cr + radius;
    let internal = mode == ThreadMode::CutInternal;
    // The empirical source expression uses pitch numerically in millimetres.
    let axial_relief = 0.1 * pitch.sqrt();
    let phase = if internal {
        if lower_side {
            axial_relief / 2.
        } else {
            -axial_relief / 2.
        }
    } else {
        0.
    };
    let hand_sign = if hand == ThreadHand::Right { 1. } else { -1. };
    let depth = if internal {
        0.75 * pitch
    } else {
        external_depth
    };
    let basic = diameter - 0.75 * pitch;
    let circle = (!internal).then(|| ButtressRootCircle {
        radius_mm: radius,
        center_axial_mm: cz,
        center_radius_mm: cr - radial,
        lower_tangent_axial_mm: cz - radius * upper.cos(),
        lower_tangent_radius_mm: cr - radius * upper.sin() - radial,
        upper_tangent_axial_mm: cz + radius * lower.cos(),
        upper_tangent_radius_mm: cr - radius * lower.sin() - radial,
    });
    Ok(ResolvedButtressThread {
        intent,
        designation: format!(
            "{} · {} · 3° face on {} local axis side · nominal profile",
            preset.designation,
            if hand == ThreadHand::Left { "LH" } else { "RH" },
            if lower_side { "negative" } else { "positive" }
        ),
        preset,
        basic_pitch_diameter_mm: basic,
        nominal_construction_pitch_diameter_mm: basic
            + if internal {
                2. * axial_relief / sum
            } else {
                0.
            },
        nominal_construction_major_diameter_mm: diameter,
        nominal_construction_minor_diameter_mm: diameter - 2. * depth,
        nominal_engagement_height_mm: 0.75 * pitch,
        nominal_radial_root_clearance_mm: external_depth - 0.75 * pitch,
        axial_relief_clearance_mm: axial_relief,
        crest_phase_axial_mm: phase,
        phase_rotation_rad: -hand_sign * std::f64::consts::TAU * phase / pitch,
        lower_flank_deg,
        upper_flank_deg,
        depth_mm: depth,
        crest_width_mm: width + if internal { axial_relief } else { 0. },
        root_circle: circle,
        scope: "Named BIS 2004 nominal ST geometry, not default 7e/7H classes or current DIN certification. The load-side value names the external tooth's local axial side, not applied force; hand is independent. External root is a tangent circle, internal material crest/root are flat. Source relief clearance and fabrication allowance are separate. No physical-fit or load-rating claim.",
    })
}
