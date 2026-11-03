//! Bounded ISO 7-1:1994 R/Rc intent; not an ISO 228 G fastening thread.
use super::*;

pub const BSPT_THREAD_CAPABILITY: &str = "model.thread.bspt.1";
pub const BSPT_THREAD_SOURCE: &str = "ISO 7-1:1994 + Cor 1:2007; IS 554:1999 identical adoption profile; nominal gauge R/Rc 1/8 and 1/4 only";
pub const BSPT_THREAD_SOURCE_URL: &str = "https://www.iso.org/standard/20819.html";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipeThreadForm {
    R,
    Rc,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipeThreadSize {
    #[serde(rename = "1/8")]
    OneEighth,
    #[serde(rename = "1/4")]
    OneQuarter,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipeGaugeConvention {
    /// R: small-end reference, gauge 4/6mm inward. Rc: fitting-face
    /// reference, gauge half a pitch inward (corrected Figure 5).
    Iso7Nominal1994,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipeThreadIntent {
    pub form: PipeThreadForm,
    pub size: PipeThreadSize,
    pub gauge: PipeGaugeConvention,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PipeThreadPreset {
    pub size: PipeThreadSize,
    pub size_label: &'static str,
    pub threads_per_inch: u32,
    pub pitch_mm: f64,
    /// Printed source values, independent of analytic profile rounding.
    pub gauge_major_mm: f64,
    pub gauge_pitch_mm: f64,
    pub gauge_minor_mm: f64,
    pub external_gauge_length_mm: f64,
    pub external_useful_length_min_mm: f64,
    pub internal_length_min_mm: f64,
    pub external_gauge_tolerance_turns: f64,
    pub internal_gauge_tolerance_turns: f64,
    pub fitting_allowance_turns: f64,
}
pub fn pipe_thread_presets() -> Vec<PipeThreadPreset> {
    [
        (
            PipeThreadSize::OneEighth,
            "1/8",
            28,
            9.728,
            9.147,
            8.566,
            4.,
            6.5,
            7.4,
        ),
        (
            PipeThreadSize::OneQuarter,
            "1/4",
            19,
            13.157,
            12.301,
            11.445,
            6.,
            9.7,
            11.,
        ),
    ]
    .into_iter()
    .map(
        |(size, size_label, tpi, major, pitch, minor, gauge, useful, receiver)| {
            let axial_pitch = 25.4 / f64::from(tpi);
            PipeThreadPreset {
                size,
                size_label,
                threads_per_inch: tpi,
                pitch_mm: axial_pitch,
                gauge_major_mm: major,
                gauge_pitch_mm: pitch,
                gauge_minor_mm: minor,
                external_gauge_length_mm: gauge,
                external_useful_length_min_mm: useful,
                // Conservative complete internal length: accepts maximum external
                // useful length (Table 1 column 16, clause 7.2.2), not merely
                // maximum gauge length. Free/incomplete runout is unavailable.
                internal_length_min_mm: receiver,
                external_gauge_tolerance_turns: 1.,
                internal_gauge_tolerance_turns: 1.25,
                fitting_allowance_turns: 2.75,
            }
        },
    )
    .collect()
}
impl PipeThreadIntent {
    pub fn preset(self) -> PipeThreadPreset {
        pipe_thread_presets()
            .into_iter()
            .find(|p| p.size == self.size)
            .expect("closed pipe size enum")
    }
    pub fn radial_taper(self) -> f64 {
        match self.form {
            PipeThreadForm::R => 1. / 32.,
            PipeThreadForm::Rc => -1. / 32.,
        }
    }
    pub fn gauge_offset_mm(self) -> f64 {
        let row = self.preset();
        match self.form {
            PipeThreadForm::R => row.external_gauge_length_mm,
            PipeThreadForm::Rc => row.pitch_mm / 2.,
        }
    }
    pub fn base_major_mm(self) -> f64 {
        self.preset().gauge_major_mm - 2. * self.radial_taper() * self.gauge_offset_mm()
    }
    pub fn default_length_mm(self) -> f64 {
        let row = self.preset();
        match self.form {
            PipeThreadForm::R => row.external_useful_length_min_mm,
            PipeThreadForm::Rc => row.internal_length_min_mm,
        }
    }
    pub fn parameters(self) -> ThreadParameters {
        ThreadParameters {
            family: ThreadFamily::Bspt,
            pipe: Some(self),
            fit: ThreadFit::BsptNominal,
            major_diameter: length(self.base_major_mm()),
            pitch: length(self.preset().pitch_mm),
            length: length(self.default_length_mm()),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedPipeThread {
    pub intent: PipeThreadIntent,
    pub designation: String,
    pub source: &'static str,
    pub source_url: &'static str,
    /// Nominal source datum, before fabrication allowance.
    pub gauge_offset_mm: f64,
    /// Plane where the allowed construction reaches the basic gauge diameter.
    pub actual_gauge_offset_mm: f64,
    pub radial_taper: f64,
    pub printed: PipeThreadPreset,
    /// Construction dimensions at the fixed nominal gauge plane.
    pub actual_gauge_major_mm: f64,
    pub actual_gauge_pitch_mm: f64,
    pub actual_gauge_minor_mm: f64,
    /// Equivalent axial gauge displacement caused by radial allowance;
    /// mandatory tolerance units are turns, not rounded printed millimetres.
    pub gauge_displacement_turns: f64,
    pub minimum_length_mm: f64,
}
pub(super) fn tapered_profile(pitch: f64, k: f64) -> (f64, f64) {
    let a = 27.5_f64.to_radians();
    let m = 1. / a.tan();
    let h = pitch / (3. * a.tan());
    let fundamental = pitch * (m * m - k * k) / (2. * m);
    (
        h,
        (fundamental - h) / (2. * (1. / a.sin() - (1. + k * k).sqrt())),
    )
}
pub(super) fn resolve_pipe(
    intent: PipeThreadIntent,
    mode: ThreadMode,
    diameter: f64,
    pitch: f64,
    length: f64,
    radial: f64,
    axial: f64,
) -> Result<ResolvedPipeThread, DocumentError> {
    let row = intent.preset();
    if (mode == ThreadMode::CutInternal) != (intent.form == PipeThreadForm::Rc) {
        return Err(invalid(
            "R requires external mode; Rc requires internal mode",
        ));
    }
    if (diameter - intent.base_major_mm()).abs() > 1e-9 || (pitch - row.pitch_mm).abs() > 1e-9 {
        return Err(invalid(
            "BSPT base diameter/pitch must match the explicit sourced pipe size and gauge convention",
        ));
    }
    let minimum_length = if intent.form == PipeThreadForm::R {
        // Clearance moves the actual gauge plane inward. Preserve enough
        // useful thread beyond it for the mandatory fitting allowance.
        intent
            .default_length_mm()
            .max(row.external_gauge_length_mm + radial * 32. + row.fitting_allowance_turns * pitch)
    } else {
        intent.default_length_mm()
    };
    if length + 1e-9 < minimum_length || axial != 0. {
        return Err(invalid(format!(
            "BSPT requires length at least {minimum_length:.6} mm and zero axial allowance; washout/incomplete runout is unavailable",
        )));
    }
    let turns = radial / (pitch / 32.);
    let limit = if intent.form == PipeThreadForm::R {
        row.external_gauge_tolerance_turns
    } else {
        row.internal_gauge_tolerance_turns
    };
    if turns > limit + 1e-9 {
        return Err(invalid(
            "BSPT radial allowance exceeds the sourced gauge-position tolerance in turns",
        ));
    }
    let sign = if intent.form == PipeThreadForm::R {
        -1.
    } else {
        1.
    };
    let actual_major = row.gauge_major_mm + 2. * sign * radial;
    let (h, _) = tapered_profile(pitch, intent.radial_taper());
    Ok(ResolvedPipeThread {
        intent,
        designation: format!(
            "{} {}",
            if intent.form == PipeThreadForm::R {
                "R"
            } else {
                "Rc"
            },
            row.size_label
        ),
        source: BSPT_THREAD_SOURCE,
        source_url: BSPT_THREAD_SOURCE_URL,
        gauge_offset_mm: intent.gauge_offset_mm(),
        actual_gauge_offset_mm: intent.gauge_offset_mm() + turns * pitch,
        radial_taper: intent.radial_taper(),
        actual_gauge_major_mm: actual_major,
        actual_gauge_pitch_mm: actual_major - h,
        actual_gauge_minor_mm: actual_major - 2. * h,
        gauge_displacement_turns: turns,
        minimum_length_mm: minimum_length,
        printed: row,
    })
}
