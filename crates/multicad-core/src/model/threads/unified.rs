//! Bounded, uncoated UN geometry from the explicitly named historical tables.
use super::*;

pub const UNIFIED_THREAD_CAPABILITY: &str = "model.thread.unified.1";
pub const UNIFIED_THREAD_SOURCE: &str =
    "NBS Handbook H28 (1969), issued March 1970; USA B1.1-1960; Table 2.21, 2A/2B";
pub const UNIFIED_THREAD_SOURCE_URL: &str =
    "https://nvlpubs.nist.gov/nistpubs/hb/1970/hb28-scan2.pdf";
pub const UNIFIED_PROFILE_SOURCE_URL: &str =
    "https://nvlpubs.nist.gov/nistpubs/hb/1970/hb28-scan1.pdf";

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ThreadDiameterRange {
    pub min_mm: Option<f64>,
    pub max_mm: Option<f64>,
}
impl ThreadDiameterRange {
    pub(super) fn contains(self, value: f64) -> bool {
        value.is_finite()
            && self.min_mm.is_none_or(|min| value >= min - 1e-9)
            && self.max_mm.is_none_or(|max| value <= max + 1e-9)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ThreadFitLimits {
    pub major: ThreadDiameterRange,
    pub pitch: ThreadDiameterRange,
    pub minor: ThreadDiameterRange,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnifiedThreadPreset {
    pub family: ThreadFamily,
    pub designation: String,
    pub diameter_mm: f64,
    pub pitch_mm: f64,
    pub nominal_diameter_inch: f64,
    pub threads_per_inch: u32,
    pub external: ThreadFitLimits,
    pub internal: ThreadFitLimits,
    pub external_allowance_mm: f64,
    /// Design/reference column, not a two-sided manufacturing limit.
    pub external_minor_reference_mm: f64,
}
impl UnifiedThreadPreset {
    pub fn limits(&self, class: ThreadFit) -> Result<ThreadFitLimits, DocumentError> {
        match class {
            ThreadFit::Unified2A => Ok(self.external),
            ThreadFit::Unified2B => Ok(self.internal),
            _ => Err(invalid("UNC/UNF require an explicit 2A or 2B fit")),
        }
    }
}
/// Analytic intended profile after fabrication allowances, not measured gage
/// acceptance. Cosmetic envelopes still contain no manufactured thread flanks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedThreadFit {
    pub class: ThreadFit,
    pub designation: String,
    pub source: &'static str,
    pub source_url: &'static str,
    pub limits: ThreadFitLimits,
    pub major_diameter_mm: f64,
    pub pitch_diameter_mm: f64,
    pub minor_diameter_mm: f64,
    pub external_minor_reference_mm: Option<f64>,
    pub engagement_limit_mm: Option<f64>,
}

struct Row {
    family: ThreadFamily,
    name: &'static str,
    diameter: f64,
    tpi: u32,
    allowance: f64,
    external_major: [f64; 2],
    external_pitch: [f64; 2],
    external_minor_reference: f64,
    internal_minor: [f64; 2],
    internal_pitch: [f64; 2],
}
// All source numbers are inches, retaining printed rounding. Table 2.21,
// printed p2.26 / scan2 PDF p9. Internal major has only a minimum; the
// external minor column is reference information, not an invented min/max.
const ROWS: [Row; 4] = [
    Row {
        family: ThreadFamily::Unc,
        name: "1/4-20 UNC",
        diameter: 0.25,
        tpi: 20,
        allowance: 0.0011,
        external_major: [0.2408, 0.2489],
        external_pitch: [0.2127, 0.2164],
        external_minor_reference: 0.1876,
        internal_minor: [0.196, 0.207],
        internal_pitch: [0.2175, 0.2224],
    },
    Row {
        family: ThreadFamily::Unf,
        name: "1/4-28 UNF",
        diameter: 0.25,
        tpi: 28,
        allowance: 0.0010,
        external_major: [0.2425, 0.2490],
        external_pitch: [0.2225, 0.2258],
        external_minor_reference: 0.2052,
        internal_minor: [0.211, 0.220],
        internal_pitch: [0.2268, 0.2311],
    },
    Row {
        family: ThreadFamily::Unc,
        name: "5/16-18 UNC",
        diameter: 0.3125,
        tpi: 18,
        allowance: 0.0012,
        external_major: [0.3026, 0.3113],
        external_pitch: [0.2712, 0.2752],
        external_minor_reference: 0.2431,
        internal_minor: [0.252, 0.265],
        internal_pitch: [0.2764, 0.2817],
    },
    Row {
        family: ThreadFamily::Unf,
        name: "5/16-24 UNF",
        diameter: 0.3125,
        tpi: 24,
        allowance: 0.0011,
        external_major: [0.3042, 0.3114],
        external_pitch: [0.2806, 0.2843],
        external_minor_reference: 0.2603,
        internal_minor: [0.267, 0.277],
        internal_pitch: [0.2854, 0.2902],
    },
];
pub fn unified_thread_presets() -> Vec<UnifiedThreadPreset> {
    let bounded = |[min, max]: [f64; 2]| ThreadDiameterRange {
        min_mm: Some(min * 25.4),
        max_mm: Some(max * 25.4),
    };
    ROWS.iter()
        .map(|row| UnifiedThreadPreset {
            family: row.family,
            designation: row.name.into(),
            diameter_mm: row.diameter * 25.4,
            pitch_mm: 25.4 / f64::from(row.tpi),
            nominal_diameter_inch: row.diameter,
            threads_per_inch: row.tpi,
            external_allowance_mm: row.allowance * 25.4,
            external_minor_reference_mm: row.external_minor_reference * 25.4,
            external: ThreadFitLimits {
                major: bounded(row.external_major),
                pitch: bounded(row.external_pitch),
                minor: ThreadDiameterRange {
                    min_mm: None,
                    max_mm: None,
                },
            },
            internal: ThreadFitLimits {
                major: ThreadDiameterRange {
                    min_mm: Some(row.diameter * 25.4),
                    max_mm: None,
                },
                pitch: bounded(row.internal_pitch),
                minor: bounded(row.internal_minor),
            },
        })
        .collect()
}
pub(super) fn preset(
    family: ThreadFamily,
    diameter: f64,
    pitch: f64,
) -> Result<UnifiedThreadPreset, DocumentError> {
    unified_thread_presets().into_iter().find(|p| p.family == family
        && (p.diameter_mm - diameter).abs() < 1e-8 && (p.pitch_mm - pitch).abs() < 1e-8)
        .ok_or_else(|| invalid("This UNC/UNF diameter and TPI are not in the sourced series table; choose a listed preset or Custom"))
}
pub(super) fn resolve_fit(
    preset: UnifiedThreadPreset,
    class: ThreadFit,
    major: f64,
    pitch: Option<f64>,
    minor: f64,
    length: f64,
) -> Result<ResolvedThreadFit, DocumentError> {
    let limits = preset.limits(class)?;
    let pitch =
        pitch.ok_or_else(|| invalid("The fitted UN profile has no pitch-diameter flank"))?;
    let engagement_limit = 1.5 * preset.diameter_mm;
    if length > engagement_limit + 1e-9 {
        return Err(invalid(
            "Sourced UNC/UNF fit coverage supports threaded length up to 1.5 nominal diameters",
        ));
    }
    for (name, value, range) in [
        ("major", major, limits.major),
        ("pitch", pitch, limits.pitch),
        ("minor", minor, limits.minor),
    ] {
        if !range.contains(value) {
            return Err(invalid(format!(
                "{} {} {name} diameter is outside the sourced fit limits after allowances; reduce the allowance or use Custom",
                preset.designation,
                class.label()
            )));
        }
    }
    Ok(ResolvedThreadFit {
        class,
        designation: preset.designation,
        source: UNIFIED_THREAD_SOURCE,
        source_url: UNIFIED_THREAD_SOURCE_URL,
        limits,
        major_diameter_mm: major,
        pitch_diameter_mm: pitch,
        minor_diameter_mm: minor,
        external_minor_reference_mm: (class == ThreadFit::Unified2A)
            .then_some(preset.external_minor_reference_mm),
        engagement_limit_mm: Some(engagement_limit),
    })
}
