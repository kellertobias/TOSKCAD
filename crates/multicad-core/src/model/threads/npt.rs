//! Explicit historical NPT construction; NPTF and ISO 7 forms are separate.
use super::*;
pub const NPT_THREAD_CAPABILITY: &str = "model.thread.npt.1";
pub const NPT_THREAD_SOURCE: &str = "NBS H28(1957) Part II, issued 1959-11-16, VII.1/VII.2; Hy-Lok H-310PF May2016 cross-check; midpoint truncation engineering target, not current ASME certification";
pub const NPT_THREAD_SOURCE_URL: &str =
    "https://nvlpubs.nist.gov/nistpubs/Legacy/hb/nbshandbook28supp1957pt2.pdf";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NptThreadForm {
    External,
    Internal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NptThreadSize {
    #[serde(rename = "1/8")]
    OneEighth,
    #[serde(rename = "1/4")]
    OneQuarter,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NptGaugeConvention {
    Nbs1959PlainFace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NptTruncationTarget {
    MidpointConeParallel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NptThreadIntent {
    pub form: NptThreadForm,
    pub size: NptThreadSize,
    pub gauge: NptGaugeConvention,
    pub truncation: NptTruncationTarget,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NptPublishedDimensions {
    pub pipe_od_inch: f64,
    pub e0_inch: f64,
    pub e1_inch: f64,
    pub e2_inch: f64,
    pub e3_inch: f64,
    pub l1_inch: f64,
    pub l2_inch: f64,
    pub l3_inch: f64,
    pub l4_inch: f64,
    pub l5_inch: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NptThreadPreset {
    pub size: NptThreadSize,
    pub size_label: &'static str,
    pub threads_per_inch: u32,
    pub pitch_mm: f64,
    pub published: NptPublishedDimensions,
    pub minimum_truncation_over_pitch: f64,
    pub maximum_truncation_over_pitch: f64,
    pub gauge_tolerance_turns: f64,
}
pub fn npt_thread_presets() -> Vec<NptThreadPreset> {
    [
        (
            NptThreadSize::OneEighth,
            "1/8",
            27,
            0.405,
            0.36351,
            0.37360,
            0.38000,
            0.35656,
            0.1615,
            0.2639,
            0.1111,
            0.3924,
            0.1898,
            0.096,
        ),
        (
            NptThreadSize::OneQuarter,
            "1/4",
            18,
            0.540,
            0.47739,
            0.49163,
            0.50250,
            0.46697,
            0.2278,
            0.4018,
            0.1667,
            0.5946,
            0.2907,
            0.088,
        ),
    ]
    .into_iter()
    .map(
        |(size, label, tpi, od, e0, e1, e2, e3, l1, l2, l3, l4, l5, max)| NptThreadPreset {
            size,
            size_label: label,
            threads_per_inch: tpi,
            pitch_mm: 25.4 / f64::from(tpi),
            published: NptPublishedDimensions {
                pipe_od_inch: od,
                e0_inch: e0,
                e1_inch: e1,
                e2_inch: e2,
                e3_inch: e3,
                l1_inch: l1,
                l2_inch: l2,
                l3_inch: l3,
                l4_inch: l4,
                l5_inch: l5,
            },
            minimum_truncation_over_pitch: 0.033,
            maximum_truncation_over_pitch: max,
            gauge_tolerance_turns: 1.,
        },
    )
    .collect()
}
impl NptThreadIntent {
    pub fn preset(self) -> NptThreadPreset {
        npt_thread_presets()
            .into_iter()
            .find(|p| p.size == self.size)
            .expect("closed NPT size enum")
    }
    pub fn radial_taper(self) -> f64 {
        if self.form == NptThreadForm::External {
            1. / 32.
        } else {
            -1. / 32.
        }
    }
    pub fn gauge_offset_mm(self) -> f64 {
        if self.form == NptThreadForm::External {
            self.preset().published.l1_inch * 25.4
        } else {
            0.
        }
    }
    pub fn profile_dimensions(self) -> (f64, f64) {
        let row = self.preset();
        let k = self.radial_taper();
        let m = 3_f64.sqrt();
        let slopes = 1. / (m - k) + 1. / (m + k);
        let fundamental = row.pitch_mm / slopes;
        let truncation = (row.minimum_truncation_over_pitch + row.maximum_truncation_over_pitch)
            / 2.
            * row.pitch_mm;
        let depth = fundamental - 2. * truncation;
        (depth, (row.pitch_mm - depth * slopes) / 2.)
    }
    pub fn base_pitch_mm(self) -> f64 {
        // Anchor E1 once. Published E0/E1/L1 are independently rounded and
        // cannot all lie exactly on one 1:16 cone.
        self.preset().published.e1_inch * 25.4 - 2. * self.radial_taper() * self.gauge_offset_mm()
    }
    pub fn base_major_mm(self) -> f64 {
        self.base_pitch_mm() + self.profile_dimensions().0
    }
    pub fn default_length_mm(self) -> f64 {
        let row = self.preset();
        if self.form == NptThreadForm::External {
            row.published.l2_inch * 25.4
        } else {
            // Engineering receiving convention: handtight plus three pitches
            // of wrench allowance and one pitch for the supported gauge shift.
            row.published.l1_inch * 25.4 + 4. * row.pitch_mm
        }
    }
    pub fn parameters(self) -> ThreadParameters {
        ThreadParameters {
            family: ThreadFamily::Npt,
            npt: Some(self),
            fit: ThreadFit::NptHistoricalMidpoint,
            major_diameter: length(self.base_major_mm()),
            pitch: length(self.preset().pitch_mm),
            length: length(self.default_length_mm()),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedNptThread {
    pub intent: NptThreadIntent,
    pub designation: String,
    pub source: &'static str,
    pub source_url: &'static str,
    pub printed: NptThreadPreset,
    pub radial_taper: f64,
    pub gauge_offset_mm: f64,
    pub actual_gauge_offset_mm: f64,
    pub gauge_displacement_turns: f64,
    pub nominal_gauge_pitch_mm: f64,
    pub actual_pitch_at_nominal_gauge_mm: f64,
    pub derived_base_pitch_mm: f64,
    pub depth_mm: f64,
    pub flat_width_mm: f64,
    pub target_truncation_mm: f64,
    pub source_basic_complete_length_mm: f64,
    pub effective_length_mm: f64,
    pub internal_wrench_allowance_mm: f64,
    pub minimum_receiving_length_mm: f64,
    pub stock_crest_radius_limit_mm: Option<f64>,
    pub actual_stock_intersection_mm: Option<f64>,
    pub complete_crest_length_mm: Option<f64>,
    pub imperfect_crest_length_mm: Option<f64>,
}
pub(super) fn resolve_npt(
    intent: NptThreadIntent,
    mode: ThreadMode,
    diameter: f64,
    pitch: f64,
    length: f64,
    radial: f64,
    axial: f64,
) -> Result<ResolvedNptThread, DocumentError> {
    let row = intent.preset();
    let internal = intent.form == NptThreadForm::Internal;
    if (mode == ThreadMode::CutInternal) != internal {
        return Err(invalid(
            "NPT external/internal intent must match the thread mode",
        ));
    }
    if (diameter - intent.base_major_mm()).abs() > 1e-9 || (pitch - row.pitch_mm).abs() > 1e-9 {
        return Err(invalid(
            "NPT base diameter/pitch must match its explicit size, E1 anchor and truncation target",
        ));
    }
    let minimum = row.published.l1_inch * 25.4 + 3. * pitch + radial * 32.;
    if axial != 0. {
        return Err(invalid("Sourced NPT requires zero axial allowance"));
    }
    if internal {
        if length + 1e-9 < minimum {
            return Err(invalid(format!(
                "NPT receiving depth must be at least {minimum:.6} mm for the declared wrench/gauge travel"
            )));
        }
    } else if (length - row.published.l2_inch * 25.4).abs() > 1e-9 {
        return Err(invalid(format!(
            "Supported external NPT effective extent is L2={:.6} mm; vanish/runout extensions are unavailable",
            row.published.l2_inch * 25.4
        )));
    }
    let turns = radial * 32. / pitch;
    if radial < 0. || turns > row.gauge_tolerance_turns + 1e-9 {
        return Err(invalid(
            "NPT clearance must stay in the supported nonnegative half of the historical ±one-turn gauge range",
        ));
    }
    let (depth, width) = intent.profile_dimensions();
    let actual_radius = diameter / 2. + if internal { radial } else { -radial };
    let limit = (!internal).then_some(row.published.pipe_od_inch * 25.4 / 2.);
    let intersection = limit.map(|limit| (limit - actual_radius) / intent.radial_taper());
    let complete = intersection.map(|z| z.clamp(0., length));
    let nominal_gauge = row.published.e1_inch * 25.4;
    Ok(ResolvedNptThread {
        intent,
        designation: format!(
            "{}-{} NPT ({})",
            row.size_label,
            row.threads_per_inch,
            if internal { "internal" } else { "external" }
        ),
        source: NPT_THREAD_SOURCE,
        source_url: NPT_THREAD_SOURCE_URL,
        radial_taper: intent.radial_taper(),
        gauge_offset_mm: intent.gauge_offset_mm(),
        actual_gauge_offset_mm: intent.gauge_offset_mm() + radial * 32.,
        gauge_displacement_turns: turns,
        nominal_gauge_pitch_mm: nominal_gauge,
        actual_pitch_at_nominal_gauge_mm: nominal_gauge
            + if internal { 2. * radial } else { -2. * radial },
        derived_base_pitch_mm: intent.base_pitch_mm(),
        depth_mm: depth,
        flat_width_mm: width,
        target_truncation_mm: (row.minimum_truncation_over_pitch
            + row.maximum_truncation_over_pitch)
            / 2.
            * pitch,
        source_basic_complete_length_mm: row.published.l5_inch * 25.4,
        effective_length_mm: row.published.l2_inch * 25.4,
        internal_wrench_allowance_mm: 3. * pitch,
        minimum_receiving_length_mm: minimum,
        stock_crest_radius_limit_mm: limit,
        actual_stock_intersection_mm: intersection,
        complete_crest_length_mm: complete,
        imperfect_crest_length_mm: complete.map(|z| length - z),
        printed: row,
    })
}
