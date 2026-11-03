use super::*;

/// Family identity is explicit even while a roadmap family's geometry is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadFamily {
    IsoMetric,
    Unified,
    Unc,
    Unf,
    Whitworth,
    Bsw,
    Bsf,
    Bspp,
    Bspt,
    Npt,
    Trapezoidal,
    Acme,
    Buttress,
    Round,
    Custom,
}
impl ThreadFamily {
    pub const ALL: [Self; 15] = [
        Self::IsoMetric,
        Self::Unified,
        Self::Unc,
        Self::Unf,
        Self::Whitworth,
        Self::Bsw,
        Self::Bsf,
        Self::Bspp,
        Self::Bspt,
        Self::Npt,
        Self::Trapezoidal,
        Self::Acme,
        Self::Buttress,
        Self::Round,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::IsoMetric => "ISO metric basic profile",
            Self::Unified => "Unified UNC / UNF",
            Self::Unc => "Unified UNC",
            Self::Unf => "Unified UNF",
            Self::Whitworth => "Whitworth BSW / BSF (legacy unspecified series)",
            Self::Bsw => "Whitworth BSW",
            Self::Bsf => "Whitworth BSF",
            Self::Bspp => "BSPP parallel pipe",
            Self::Bspt => "BSPT tapered pipe",
            Self::Npt => "NPT tapered pipe",
            Self::Trapezoidal => "Trapezoidal",
            Self::Acme => "Acme",
            Self::Buttress => "Buttress",
            Self::Round => "Round",
            Self::Custom => "Custom axial profile",
        }
    }
    pub fn supported(self) -> bool {
        matches!(
            self,
            Self::IsoMetric
                | Self::Unc
                | Self::Unf
                | Self::Bsw
                | Self::Bsf
                | Self::Bspp
                | Self::Bspt
                | Self::Npt
                | Self::Trapezoidal
                | Self::Acme
                | Self::Buttress
                | Self::Custom
        )
    }
    pub fn is_rounded(self) -> bool {
        matches!(self, Self::Bsw | Self::Bsf | Self::Bspp)
    }
    pub fn is_lead_screw(self) -> bool {
        matches!(self, Self::Trapezoidal | Self::Acme)
    }
    pub fn is_unified(self) -> bool {
        matches!(self, Self::Unc | Self::Unf)
    }
    pub fn source(self) -> &'static str {
        match self {
            Self::IsoMetric => "NBK metric basic profile; JIS B 0205-2/4:2001; data revision 1",
            Self::Custom => "User-defined dimensions; no standard designation or fit class",
            Self::Unc | Self::Unf => UNIFIED_THREAD_SOURCE,
            Self::Bsw | Self::Bsf => WHITWORTH_THREAD_SOURCE,
            Self::Bspp => BSPP_THREAD_SOURCE,
            Self::Bspt => BSPT_THREAD_SOURCE,
            Self::Npt => NPT_THREAD_SOURCE,
            Self::Trapezoidal => TRAPEZOIDAL_THREAD_SOURCE,
            Self::Acme => ACME_THREAD_SOURCE,
            Self::Buttress => BUTTRESS_THREAD_SOURCE,
            _ => "Not implemented; no sourced geometry or tolerance coverage claimed",
        }
    }
}
pub const METRIC_THREAD_SOURCE_URL: &str =
    "https://www.nbk1560.com/images/en-US/product/technical_data/meternezi_NBK/meternezi_NBK_1.pdf";
/// Basic diameter/pitch combinations from the named manufacturer's table.
/// These are nominal geometry, not stocked hardware or ISO 965 fit limits.
const METRIC_PITCHES: &[(f64, &[f64])] = &[
    (2., &[0.4, 0.25]),
    (2.5, &[0.45, 0.35]),
    (3., &[0.5, 0.35]),
    (3.5, &[0.6, 0.35]),
    (4., &[0.7, 0.5]),
    (4.5, &[0.75, 0.5]),
    (5., &[0.8, 0.5]),
    (5.5, &[0.5]),
    (6., &[1., 0.75]),
    (7., &[1., 0.75]),
    (8., &[1.25, 1., 0.75]),
    (9., &[1.25, 1., 0.75]),
    (10., &[1.5, 1.25, 1., 0.75]),
    (11., &[1.5, 1., 0.75]),
    (12., &[1.75, 1.5, 1.25, 1.]),
    (14., &[2., 1.5, 1.25, 1.]),
    (16., &[2., 1.5, 1.]),
    (18., &[2.5, 2., 1.5, 1.]),
    (20., &[2.5, 2., 1.5, 1.]),
    (22., &[2.5, 2., 1.5, 1.]),
    (24., &[3., 2., 1.5, 1.]),
    (27., &[3., 2., 1.5, 1.]),
    (30., &[3.5, 3., 2., 1.5, 1.]),
];
#[derive(Clone, Debug, PartialEq)]
pub struct MetricThreadPreset {
    pub designation: String,
    pub diameter_mm: f64,
    pub pitch_mm: f64,
}
pub fn metric_thread_presets() -> Vec<MetricThreadPreset> {
    METRIC_PITCHES
        .iter()
        .flat_map(|(diameter, pitches)| {
            pitches.iter().map(move |pitch| MetricThreadPreset {
                designation: format!("M{diameter} × {pitch}"),
                diameter_mm: *diameter,
                pitch_mm: *pitch,
            })
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomThreadProfile {
    pub depth: Dimension,
    pub crest_width: Dimension,
    pub lower_flank: Dimension,
    pub upper_flank: Dimension,
    /// Radius change per millimetre along the axis. Diameter taper is twice this value.
    pub radial_taper: Dimension,
}
impl Default for CustomThreadProfile {
    fn default() -> Self {
        Self {
            depth: length(0.625),
            crest_width: length(0.25),
            lower_flank: angle(15.),
            upper_flank: angle(15.),
            radial_taper: Dimension::Literal {
                value: 0.,
                unit: "ratio".into(),
            },
        }
    }
}
impl CustomThreadProfile {
    fn dimensions(&self) -> [(&Dimension, Unit); 5] {
        [
            (&self.depth, Unit::Millimeter),
            (&self.crest_width, Unit::Millimeter),
            (&self.lower_flank, Unit::Degree),
            (&self.upper_flank, Unit::Degree),
            (&self.radial_taper, Unit::Ratio),
        ]
    }
    pub(super) fn validate(&self) -> Result<(), DocumentError> {
        for (dimension, unit) in self.dimensions() {
            check_dimension(dimension, unit)?;
        }
        Ok(())
    }
    pub(super) fn inputs(&self) -> Vec<&Dimension> {
        self.dimensions().into_iter().map(|(d, _)| d).collect()
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedThreadProfile {
    pub depth_mm: f64,
    pub crest_width_mm: f64,
    pub lower_flank_deg: f64,
    pub upper_flank_deg: f64,
    pub radial_taper: f64,
    pub rounding_radius_mm: Option<f64>,
}
pub(super) fn resolve_profile(
    family: ThreadFamily,
    custom: Option<&CustomThreadProfile>,
    diameter: f64,
    pitch: f64,
    mode: ThreadMode,
    parameters: &EvaluatedParameters,
) -> Result<ResolvedThreadProfile, DocumentError> {
    let profile = match family {
        ThreadFamily::Bsw | ThreadFamily::Bsf | ThreadFamily::Bspp | ThreadFamily::Bspt => {
            let (depth_mm, radius_mm) = whitworth::profile_dimensions(pitch);
            ResolvedThreadProfile {
                depth_mm,
                crest_width_mm: 0.,
                lower_flank_deg: 27.5,
                upper_flank_deg: 27.5,
                radial_taper: 0.,
                rounding_radius_mm: Some(radius_mm),
            }
        }
        ThreadFamily::Unc | ThreadFamily::Unf => ResolvedThreadProfile {
            depth_mm: if mode == ThreadMode::CutInternal {
                5. * 3_f64.sqrt() / 16. * pitch
            } else {
                17. * 3_f64.sqrt() / 48. * pitch
            },
            crest_width_mm: pitch / 8.,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            rounding_radius_mm: None,
        },
        ThreadFamily::IsoMetric => {
            if !METRIC_PITCHES.iter().any(|(d, pitches)| {
                (*d - diameter).abs() < 1e-8 && pitches.iter().any(|p| (*p - pitch).abs() < 1e-8)
            }) {
                return Err(invalid(
                    "This nominal metric diameter/pitch combination is not in the sourced table; select a listed preset or explicitly use Custom",
                ));
            }
            ResolvedThreadProfile {
                depth_mm: 5. * 3_f64.sqrt() / 16. * pitch,
                crest_width_mm: pitch / 8.,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                radial_taper: 0.,
                rounding_radius_mm: None,
            }
        }
        ThreadFamily::Custom => {
            let p =
                custom.ok_or_else(|| invalid("Custom thread needs its explicit axial profile"))?;
            p.validate()?;
            ResolvedThreadProfile {
                depth_mm: number(&p.depth, Unit::Millimeter, parameters)?,
                crest_width_mm: number(&p.crest_width, Unit::Millimeter, parameters)?,
                lower_flank_deg: number(&p.lower_flank, Unit::Degree, parameters)?,
                upper_flank_deg: number(&p.upper_flank, Unit::Degree, parameters)?,
                radial_taper: number(&p.radial_taper, Unit::Ratio, parameters)?,
                rounding_radius_mm: None,
            }
        }
        _ => {
            return Err(invalid(format!(
                "{} is not implemented; use an explicitly custom profile without a standard designation",
                family.label()
            )));
        }
    };
    if profile.depth_mm <= 0.
        || (profile.rounding_radius_mm.is_none() && profile.crest_width_mm <= 0.)
        || ![profile.lower_flank_deg, profile.upper_flank_deg]
            .into_iter()
            .all(|v| (1. ..=80.).contains(&v))
        || profile.radial_taper.abs() > 0.1
    {
        return Err(invalid(
            "Thread needs positive depth/crest width, 1–80 degree flanks and radius taper within ±0.1",
        ));
    }
    Ok(profile)
}
