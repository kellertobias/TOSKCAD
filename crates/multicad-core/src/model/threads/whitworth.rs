//! Bounded true rounded Whitworth profiles and independently sourced fit limits.
use super::*;

pub const ROUNDED_THREAD_CAPABILITY: &str = "model.thread.rounded.1";
pub const WHITWORTH_THREAD_SOURCE: &str =
    "BS 84:2007, uncoated Medium BSW/BSF; engagement up to 15 pitches; bounded table rows";
pub const WHITWORTH_THREAD_SOURCE_URL: &str =
    "https://ssmalloys.com/wp-content/uploads/2024/11/BS-84-2007.pdf";
pub const WHITWORTH_PUBLISHER_URL: &str =
    "https://knowledge.bsigroup.com/products/parallel-screw-threads-of-whitworth-form-requirements";
pub const BSPP_THREAD_SOURCE: &str = "Elesa+Ganter Technical Data 2018 section 10.6; external Class B/internal; NBK JIS B 0202:1999 basic form; no pressure-seal or full ISO 228 compliance claim";
pub const BSPP_THREAD_SOURCE_URL: &str =
    "https://www.elesa-ganter.com/siteassets/technical/Technical_Data_General_EN.pdf";
pub const BSPP_PROFILE_SOURCE_URL: &str = "https://www.nbk1560.com/images/en-US/product/technical_data/heikounezi_NBK/heikounezi_NBK_1.pdf";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WhitworthThreadPreset {
    pub family: ThreadFamily,
    pub designation: String,
    /// Physical basic major diameter. G pipe designations are not inch diameters.
    pub diameter_mm: f64,
    pub pitch_mm: f64,
    pub threads_per_inch: u32,
    pub nominal_diameter_inch: Option<f64>,
    pub external: ThreadFitLimits,
    pub internal: ThreadFitLimits,
    pub source: &'static str,
    pub source_url: &'static str,
    pub profile_source_url: &'static str,
}
impl WhitworthThreadPreset {
    pub fn limits(&self, class: ThreadFit) -> Result<ThreadFitLimits, DocumentError> {
        match (self.family, class) {
            (ThreadFamily::Bsw | ThreadFamily::Bsf, ThreadFit::WhitworthMediumExternal)
            | (ThreadFamily::Bspp, ThreadFit::BsppClassBExternal) => Ok(self.external),
            (ThreadFamily::Bsw | ThreadFamily::Bsf, ThreadFit::WhitworthMediumInternal)
            | (ThreadFamily::Bspp, ThreadFit::BsppInternal) => Ok(self.internal),
            _ => Err(invalid(
                "Select the explicitly sourced class for this rounded thread family and mode",
            )),
        }
    }
}
struct Row {
    family: ThreadFamily,
    name: &'static str,
    diameter: f64,
    inch: Option<f64>,
    tpi: u32,
    scale: f64,
    external: [[Option<f64>; 2]; 3],
    internal: [[Option<f64>; 2]; 3],
}
// BSW/BSF printed inch limits: BS 84:2007 Tables 3/4 and 9/10,
// printed pp17/18 and 23/24 (PDF pp23/24 and 29/30). Internal major is
// minimum-only. BSPP limits retain millimeter table rounding: Elesa+Ganter
// Technical Data copyright 2018, section 10.6, full PDF p21/footer A21.
const ROWS: [Row; 6] = [
    Row {
        family: ThreadFamily::Bsw,
        name: "1/4-20 BSW",
        diameter: 0.25,
        inch: Some(0.25),
        tpi: 20,
        scale: 25.4,
        external: [
            [Some(0.2427), Some(0.2488)],
            [Some(0.2129), Some(0.2168)],
            [Some(0.1764), Some(0.1848)],
        ],
        internal: [
            [Some(0.25), None],
            [Some(0.218), Some(0.2219)],
            [Some(0.186), Some(0.203)],
        ],
    },
    Row {
        family: ThreadFamily::Bsw,
        name: "5/16-18 BSW",
        diameter: 0.3125,
        inch: Some(0.3125),
        tpi: 18,
        scale: 25.4,
        external: [
            [Some(0.3046), Some(0.3112)],
            [Some(0.2714), Some(0.2756)],
            [Some(0.2311), Some(0.24)],
        ],
        internal: [
            [Some(0.3125), None],
            [Some(0.2769), Some(0.2811)],
            [Some(0.2413), Some(0.2594)],
        ],
    },
    Row {
        family: ThreadFamily::Bsf,
        name: "1/4-26 BSF",
        diameter: 0.25,
        inch: Some(0.25),
        tpi: 26,
        scale: 25.4,
        external: [
            [Some(0.2432), Some(0.2489)],
            [Some(0.2206), Some(0.2243)],
            [Some(0.1921), Some(0.1997)],
        ],
        internal: [
            [Some(0.25), None],
            [Some(0.2254), Some(0.2291)],
            [Some(0.2008), Some(0.2125)],
        ],
    },
    Row {
        family: ThreadFamily::Bsf,
        name: "5/16-22 BSF",
        diameter: 0.3125,
        inch: Some(0.3125),
        tpi: 22,
        scale: 25.4,
        external: [
            [Some(0.3051), Some(0.3113)],
            [Some(0.2781), Some(0.2822)],
            [Some(0.2447), Some(0.2531)],
        ],
        internal: [
            [Some(0.3125), None],
            [Some(0.2834), Some(0.2875)],
            [Some(0.2543), Some(0.2684)],
        ],
    },
    Row {
        family: ThreadFamily::Bspp,
        name: "G1/8",
        diameter: 9.728,
        inch: None,
        tpi: 28,
        scale: 1.,
        external: [
            [Some(9.514), Some(9.728)],
            [Some(8.933), Some(9.147)],
            [Some(8.298), Some(8.566)],
        ],
        internal: [
            [Some(9.728), None],
            [Some(9.147), Some(9.254)],
            [Some(8.566), Some(8.848)],
        ],
    },
    Row {
        family: ThreadFamily::Bspp,
        name: "G1/4",
        diameter: 13.157,
        inch: None,
        tpi: 19,
        scale: 1.,
        external: [
            [Some(12.907), Some(13.157)],
            [Some(12.051), Some(12.301)],
            [Some(11.133), Some(11.445)],
        ],
        internal: [
            [Some(13.157), None],
            [Some(12.301), Some(12.426)],
            [Some(11.445), Some(11.89)],
        ],
    },
];
pub fn whitworth_thread_presets() -> Vec<WhitworthThreadPreset> {
    ROWS.iter()
        .map(|row| {
            let limits = |values: [[Option<f64>; 2]; 3]| {
                let range = |[min, max]: [Option<f64>; 2]| ThreadDiameterRange {
                    min_mm: min.map(|v| v * row.scale),
                    max_mm: max.map(|v| v * row.scale),
                };
                ThreadFitLimits {
                    major: range(values[0]),
                    pitch: range(values[1]),
                    minor: range(values[2]),
                }
            };
            let pipe = row.family == ThreadFamily::Bspp;
            WhitworthThreadPreset {
                family: row.family,
                designation: row.name.into(),
                diameter_mm: row.diameter * row.scale,
                pitch_mm: 25.4 / f64::from(row.tpi),
                threads_per_inch: row.tpi,
                nominal_diameter_inch: row.inch,
                external: limits(row.external),
                internal: limits(row.internal),
                source: if pipe {
                    BSPP_THREAD_SOURCE
                } else {
                    WHITWORTH_THREAD_SOURCE
                },
                source_url: if pipe {
                    BSPP_THREAD_SOURCE_URL
                } else {
                    WHITWORTH_THREAD_SOURCE_URL
                },
                profile_source_url: if pipe {
                    BSPP_PROFILE_SOURCE_URL
                } else {
                    WHITWORTH_THREAD_SOURCE_URL
                },
            }
        })
        .collect()
}
pub(super) fn profile_dimensions(pitch: f64) -> (f64, f64) {
    let angle = 27.5_f64.to_radians();
    (
        pitch / (3. * angle.tan()),
        pitch * angle.cos() / (12. * (1. - angle.sin())),
    )
}
pub(super) fn rounded_preset(
    family: ThreadFamily,
    diameter: f64,
    pitch: f64,
) -> Result<WhitworthThreadPreset, DocumentError> {
    whitworth_thread_presets().into_iter().find(|p|p.family==family&&(p.diameter_mm-diameter).abs()<1e-8&&(p.pitch_mm-pitch).abs()<1e-8)
        .ok_or_else(||invalid("This rounded thread diameter/TPI is outside the six sourced rows; choose a listed preset or Custom"))
}
/// Intersect pitch limits with translated major and minor limits. A fixed true
/// rounded profile cannot independently choose the three diameter midpoints.
pub(super) fn fitted_pitch(
    preset: &WhitworthThreadPreset,
    class: ThreadFit,
    depth: f64,
) -> Result<f64, DocumentError> {
    let limits = preset.limits(class)?;
    let mut minimum = f64::NEG_INFINITY;
    let mut maximum = f64::INFINITY;
    for (range, shift) in [
        (limits.pitch, 0.),
        (limits.major, -depth),
        (limits.minor, depth),
    ] {
        if let Some(min) = range.min_mm {
            minimum = minimum.max(min + shift);
        }
        if let Some(max) = range.max_mm {
            maximum = maximum.min(max + shift);
        }
    }
    if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
        return Err(invalid(
            "The sourced limits have no feasible fully rounded profile; independent truncation is unavailable",
        ));
    }
    Ok((minimum + maximum) / 2.)
}
pub(super) fn resolve_rounded_fit(
    preset: WhitworthThreadPreset,
    class: ThreadFit,
    major: f64,
    pitch: f64,
    minor: f64,
    length: f64,
) -> Result<ResolvedThreadFit, DocumentError> {
    let limits = preset.limits(class)?;
    let engagement_limit_mm =
        (preset.family != ThreadFamily::Bspp).then_some(15. * preset.pitch_mm);
    if engagement_limit_mm.is_some_and(|limit| length > limit + 1e-9) {
        return Err(invalid(
            "BS 84 Medium pitch-diameter coverage is bounded to engagement up to 15 pitches",
        ));
    }
    for (name, value, range) in [
        ("major", major, limits.major),
        ("pitch", pitch, limits.pitch),
        ("minor", minor, limits.minor),
    ] {
        if !range.contains(value) {
            return Err(invalid(format!(
                "{} {} {name} diameter is outside sourced limits after radial allowance",
                preset.designation,
                class.label()
            )));
        }
    }
    Ok(ResolvedThreadFit {
        class,
        designation: preset.designation,
        source: preset.source,
        source_url: preset.source_url,
        limits,
        major_diameter_mm: major,
        pitch_diameter_mm: pitch,
        minor_diameter_mm: minor,
        external_minor_reference_mm: None,
        engagement_limit_mm,
    })
}
