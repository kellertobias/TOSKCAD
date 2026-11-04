//! Bounded, uncoated historical 6g/6H geometry; targets are engineering choices.
use super::*;

pub const METRIC_FIT_CAPABILITY: &str = "model.thread.metric_fit.1";
pub const METRIC_FIT_SOURCE: &str = "BIS IS 14962 Parts 1/2:2001, identical ISO 965-1/2:1998; M3/M6 coarse, uncoated 6g/6H, normal engagement";
pub const METRIC_FIT_SOURCE_URL: &str =
    "https://law.resource.org/pub/in/bis/S01/is.14962.2.2001.pdf";
pub const METRIC_ROOT_SOURCE_URL: &str =
    "https://law.resource.org/pub/in/bis/S01/is.14962.1.2001.pdf";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MetricFitPreset {
    pub designation: &'static str,
    pub diameter_mm: f64,
    pub pitch_mm: f64,
    pub engagement_over_mm: f64,
    pub engagement_up_to_mm: f64,
    pub external: ThreadFitLimits,
    pub internal: ThreadFitLimits,
    pub printed_external_minimum_root_radius_mm: f64,
}
fn range(min: f64, max: f64) -> ThreadDiameterRange {
    ThreadDiameterRange {
        min_mm: Some(min),
        max_mm: Some(max),
    }
}
pub fn metric_fit_presets() -> Vec<MetricFitPreset> {
    // Part2: printed p2 Table1 and p3 Table2. No external minor limits or
    // internal major limits are invented from a stress-calculation reference.
    [
        (
            "M3 × 0.5",
            3.,
            0.5,
            1.5,
            4.5,
            [2.874, 2.980],
            [2.580, 2.655],
            [2.675, 2.775],
            [2.459, 2.599],
            0.063,
        ),
        (
            "M6 × 1",
            6.,
            1.,
            3.,
            9.,
            [5.794, 5.974],
            [5.212, 5.324],
            [5.350, 5.500],
            [4.917, 5.153],
            0.125,
        ),
    ]
    .into_iter()
    .map(
        |(
            designation,
            diameter_mm,
            pitch_mm,
            engagement_over_mm,
            engagement_up_to_mm,
            em,
            ep,
            ip,
            im,
            printed_external_minimum_root_radius_mm,
        )| MetricFitPreset {
            designation,
            diameter_mm,
            pitch_mm,
            engagement_over_mm,
            engagement_up_to_mm,
            external: ThreadFitLimits {
                major: range(em[0], em[1]),
                pitch: range(ep[0], ep[1]),
                minor: ThreadDiameterRange {
                    min_mm: None,
                    max_mm: None,
                },
            },
            internal: ThreadFitLimits {
                major: ThreadDiameterRange {
                    min_mm: None,
                    max_mm: None,
                },
                pitch: range(ip[0], ip[1]),
                minor: range(im[0], im[1]),
            },
            printed_external_minimum_root_radius_mm,
        },
    )
    .collect()
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedMetricFit {
    pub preset: MetricFitPreset,
    pub nominal_basic_pitch_diameter_mm: f64,
    pub target_major_diameter_mm: f64,
    pub target_pitch_diameter_mm: f64,
    pub target_minor_diameter_mm: f64,
    pub external_root_radius_mm: Option<f64>,
    pub scope: &'static str,
}
pub(super) fn metric_fit_construction(
    class: ThreadFit,
    diameter: f64,
    pitch: f64,
) -> Result<(ResolvedMetricFit, ResolvedThreadProfile), DocumentError> {
    let preset=metric_fit_presets().into_iter().find(|p| (p.diameter_mm-diameter).abs()<1e-8 && (p.pitch_mm-pitch).abs()<1e-8)
        .ok_or_else(||invalid("Sourced metric 6g/6H coverage is M3×0.5 and M6×1 only; choose a covered size or Nominal basic"))?;
    let internal = class == ThreadFit::Metric6HInternal;
    let midpoint = |r: ThreadDiameterRange| {
        (r.min_mm.expect("bounded source") + r.max_mm.expect("bounded source")) / 2.
    };
    let target_pitch = midpoint(if internal {
        preset.internal.pitch
    } else {
        preset.external.pitch
    });
    let major = if internal {
        diameter
    } else {
        midpoint(preset.external.major)
    };
    let crest = pitch / 2. - (major - target_pitch) / 3_f64.sqrt();
    let radius = (!internal).then_some(pitch / (4. * 3_f64.sqrt()));
    let depth = if internal {
        (major - midpoint(preset.internal.minor)) / 2.
    } else {
        (pitch - crest) / (2. / 3_f64.sqrt()) - radius.expect("external circle")
    };
    Ok((
        ResolvedMetricFit {
            preset,
            nominal_basic_pitch_diameter_mm: diameter - 3. * 3_f64.sqrt() / 8. * pitch,
            target_major_diameter_mm: major,
            target_pitch_diameter_mm: target_pitch,
            target_minor_diameter_mm: major - 2. * depth,
            external_root_radius_mm: radius,
            scope: "Historical uncoated dimensional fit; midpoint targets, external tangent root R=P/(4sqrt(3)), internal flat root; no gage, strength or current-standard certification",
        },
        ResolvedThreadProfile {
            depth_mm: depth,
            crest_width_mm: crest,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            rounding_radius_mm: None,
        },
    ))
}
pub(super) fn resolve_metric_fit(
    construction: &ResolvedMetricFit,
    class: ThreadFit,
    major: f64,
    pitch: f64,
    minor: f64,
    length: f64,
) -> Result<ResolvedThreadFit, DocumentError> {
    let p = &construction.preset;
    if length <= p.engagement_over_mm || length > p.engagement_up_to_mm {
        return Err(invalid(format!(
            "{} fit requires normal engagement over {} and up to {} mm",
            p.designation, p.engagement_over_mm, p.engagement_up_to_mm
        )));
    }
    let limits = if class == ThreadFit::Metric6gExternal {
        p.external
    } else {
        p.internal
    };
    for (name, value, range) in [
        ("major", major, limits.major),
        ("pitch", pitch, limits.pitch),
        ("minor", minor, limits.minor),
    ] {
        if !range.contains(value) {
            return Err(invalid(format!(
                "{} {} {name} diameter is outside its sourced limits after fabrication allowances",
                p.designation,
                class.label()
            )));
        }
    }
    if let Some(radius) = construction.external_root_radius_mm {
        if radius
            < p.printed_external_minimum_root_radius_mm
                .max(0.125 * p.pitch_mm)
            || minor >= p.diameter_mm - 5. * 3_f64.sqrt() / 8. * p.pitch_mm
        {
            return Err(invalid(
                "Metric external root violates its sourced radius/basic-profile envelope",
            ));
        }
    } else if major < p.diameter_mm {
        return Err(invalid(
            "Internal root contour crosses the nominal basic major cylinder",
        ));
    }
    Ok(ResolvedThreadFit {
        class,
        designation: p.designation.into(),
        source: METRIC_FIT_SOURCE,
        source_url: METRIC_FIT_SOURCE_URL,
        limits,
        major_diameter_mm: major,
        pitch_diameter_mm: pitch,
        minor_diameter_mm: minor,
        external_minor_reference_mm: None,
        engagement_limit_mm: Some(p.engagement_up_to_mm),
    })
}
