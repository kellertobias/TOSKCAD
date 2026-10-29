//! Sheet resolution: stock thickness, bend inputs with their sources, and the
//! base in world millimeters. The native evaluator builds the folded solid
//! and the developed flat pattern from these values.
use super::*;
use crate::{model::common::number, parameters::EvaluatedParameters};

pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Where a bend input came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSource {
    Stock,
    Part,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedBase {
    /// The rectangle corners are (0, 0) and (length, width) in `frame`.
    Rectangle {
        frame: SheetFrame,
        length_mm: f64,
        width_mm: f64,
    },
    Profile {
        profile: ProfileSource,
    },
    Contour {
        frame: SheetFrame,
        loops: Vec<PlanarLoop>,
    },
}
/// One step along a flange's cross-section. A positive bend angle turns
/// toward the material side of the strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StripSegment {
    Straight { length_mm: f64 },
    Bend { angle_rad: f64, radius_mm: f64 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFlange {
    pub index: usize,
    pub edge: usize,
    /// "bend", "hem" or "jog".
    pub kind: &'static str,
    pub direction: BendDirection,
    pub segments: Vec<StripSegment>,
    pub inside_radius_mm: f64,
    pub leg_mm: f64,
    pub start_inset_mm: f64,
    pub end_inset_mm: f64,
    /// Relief width along the edge and depth into the base.
    pub relief_mm: Option<[f64; 2]>,
}
impl ResolvedFlange {
    /// Developed length from the base edge to the leg end: straight
    /// lengths plus the bend allowance of every bend.
    pub fn developed_mm(&self, thickness_mm: f64, k: f64) -> f64 {
        self.segments
            .iter()
            .map(|s| match *s {
                StripSegment::Straight { length_mm } => length_mm,
                StripSegment::Bend {
                    angle_rad,
                    radius_mm,
                } => angle_rad.abs() * (radius_mm + k * thickness_mm),
            })
            .sum()
    }
    /// Developed distance from the base edge to the start of the leg.
    pub fn leg_offset_mm(&self, thickness_mm: f64, k: f64) -> f64 {
        self.developed_mm(thickness_mm, k) - self.leg_mm
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedShape {
    Circle { center: [f64; 2], diameter_mm: f64 },
    Rectangle { center: [f64; 2], size_mm: [f64; 2] },
}
impl ResolvedShape {
    pub fn area_mm2(&self) -> f64 {
        match *self {
            Self::Circle { diameter_mm, .. } => std::f64::consts::PI * diameter_mm.powi(2) / 4.,
            Self::Rectangle { size_mm, .. } => size_mm[0] * size_mm[1],
        }
    }
    pub fn moved(&self, by: [f64; 2]) -> Self {
        match *self {
            Self::Circle {
                center,
                diameter_mm,
            } => Self::Circle {
                center: [center[0] + by[0], center[1] + by[1]],
                diameter_mm,
            },
            Self::Rectangle { center, size_mm } => Self::Rectangle {
                center: [center[0] + by[0], center[1] + by[1]],
                size_mm,
            },
        }
    }
}
/// Where a cut opening comes from: an explicit cutout or a mounting
/// pattern's receiving hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutoutSource {
    Cutout { index: usize },
    Fastener { pattern: usize, point: usize },
}
impl CutoutSource {
    /// Stable id within the feature: `cutout.<i>` or `fastener.<p>.<j>`.
    pub fn id(self) -> String {
        match self {
            Self::Cutout { index } => format!("cutout.{index}"),
            Self::Fastener { pattern, point } => format!("fastener.{pattern}.{point}"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedCutout {
    pub face: CutoutFace,
    pub shape: ResolvedShape,
    pub source: CutoutSource,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSheet {
    pub stock: EntityId,
    pub stock_spec: Stock,
    pub thickness_mm: f64,
    pub inside_radius_mm: f64,
    pub radius_source: ValueSource,
    pub k_factor: f64,
    pub k_factor_source: ValueSource,
    pub reverse: bool,
    pub base: ResolvedBase,
    pub flanges: Vec<ResolvedFlange>,
    /// Explicit cutouts, then every mounting pattern's receiving holes.
    pub cutouts: Vec<ResolvedCutout>,
    pub patterns: Vec<ResolvedPattern>,
}
impl ResolvedSheet {
    /// Bend allowance: the developed length of the neutral axis through a
    /// bend of `angle_rad`.
    pub fn bend_allowance_mm(&self, angle_rad: f64, inside_radius_mm: f64, k: f64) -> f64 {
        angle_rad * (inside_radius_mm + k * self.thickness_mm)
    }
}

pub fn sheet_stock_values(stock: &Stock) -> Option<(f64, f64, f64)> {
    match stock.specification {
        StockKind::Sheet {
            thickness_mm,
            inside_bend_radius_mm,
            k_factor,
            ..
        } => Some((thickness_mm, inside_bend_radius_mm, k_factor)),
        _ => None,
    }
}

/// Resolve a sheet part against its stock and parameters.
pub fn resolve_sheet(
    document: &Document,
    parameters: &EvaluatedParameters,
    sheet: &SheetDefinition,
) -> Result<ResolvedSheet, DocumentError> {
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let spec = read_sheet_stock(document, sheet.stock)?;
    let (thickness_mm, stock_radius, stock_k) =
        sheet_stock_values(&spec).ok_or_else(|| invalid("Sheet stock has no sheet values"))?;
    if thickness_mm <= 0. {
        return Err(invalid("Sheet stock has no thickness"));
    }
    let (inside_radius_mm, radius_source) = match &sheet.inside_radius {
        Some(d) => (mm(d)?, ValueSource::Part),
        None => (stock_radius, ValueSource::Stock),
    };
    if !(0. ..=1e5).contains(&inside_radius_mm) {
        return Err(invalid("The inside bend radius cannot be negative"));
    }
    let (k_factor, k_factor_source) = match &sheet.k_factor {
        Some(d) => (number(d, Unit::Ratio, parameters)?, ValueSource::Part),
        None => (stock_k, ValueSource::Stock),
    };
    if !(0. ..=1.).contains(&k_factor) {
        return Err(invalid("The K-factor must lie between 0 and 1"));
    }
    let base = match &sheet.base {
        SheetBase::Rectangle {
            plane,
            origin,
            length,
            width,
        } => {
            let (u, v) = plane.axes();
            let length_mm = mm(length)?;
            let width_mm = mm(width)?;
            if length_mm <= 0. || width_mm <= 0. {
                return Err(invalid("Sheet length and width must be positive"));
            }
            ResolvedBase::Rectangle {
                frame: SheetFrame {
                    origin: [mm(&origin[0])?, mm(&origin[1])?, mm(&origin[2])?],
                    u,
                    v,
                },
                length_mm,
                width_mm,
            }
        }
        SheetBase::Profile { profile } => ResolvedBase::Profile {
            profile: profile.clone(),
        },
        SheetBase::Contour { frame, loops, .. } => ResolvedBase::Contour {
            frame: *frame,
            loops: loops.clone(),
        },
    };
    let deg = |d: &Dimension| number(d, Unit::Radian, parameters);
    let mut flanges = vec![];
    for (index, f) in sheet.flanges.iter().enumerate() {
        let what = |m: &str| invalid(format!("Flange {}: {m}", index + 1));
        let radius = match &f.inside_radius {
            Some(d) => mm(d)?,
            None => inside_radius_mm,
        };
        let leg = mm(&f.length)?;
        let start = f.start_inset.as_ref().map_or(Ok(0.), mm)?;
        let end = f.end_inset.as_ref().map_or(Ok(0.), mm)?;
        if radius < 0. || leg < 0. || start < 0. || end < 0. {
            return Err(what("radius, length and insets cannot be negative"));
        }
        let sign = match f.direction {
            BendDirection::Up => 1.,
            BendDirection::Down => -1.,
        };
        let pi = std::f64::consts::PI;
        let (kind, radius, segments) = match &f.kind {
            FlangeKind::Bend { angle } => {
                let a = deg(angle)?;
                if !(a > 1e-9 && a <= pi + 1e-12) {
                    return Err(what("the bend angle must be above 0° and at most 180°"));
                }
                if (a - pi).abs() < 1e-12 && radius <= 0. {
                    return Err(what("a 180° bend needs an inside radius; use a hem"));
                }
                (
                    "bend",
                    radius,
                    vec![StripSegment::Bend {
                        angle_rad: sign * a,
                        radius_mm: radius,
                    }],
                )
            }
            FlangeKind::Hem { gap } => {
                let gap = mm(gap)?;
                if gap <= 0. {
                    return Err(what("a hem needs a positive gap"));
                }
                (
                    "hem",
                    gap / 2.,
                    vec![StripSegment::Bend {
                        angle_rad: sign * pi,
                        radius_mm: gap / 2.,
                    }],
                )
            }
            FlangeKind::Jog { angle, offset } => {
                let a = deg(angle)?;
                let h = mm(offset)?;
                if !(a > 1e-9 && a <= pi / 2. + 1e-12) {
                    return Err(what("a jog angle must be above 0° and at most 90°"));
                }
                let t = thickness_mm;
                let straight = (h - t - 2. * radius + (2. * radius + t) * a.cos()) / a.sin();
                if straight < -1e-9 {
                    return Err(what(&format!(
                        "a {h} mm jog offset is too small for its bends; it needs at least {:.3} mm",
                        t + 2. * radius - (2. * radius + t) * a.cos()
                    )));
                }
                (
                    "jog",
                    radius,
                    vec![
                        StripSegment::Bend {
                            angle_rad: sign * a,
                            radius_mm: radius,
                        },
                        StripSegment::Straight {
                            length_mm: straight.max(0.),
                        },
                        StripSegment::Bend {
                            angle_rad: -sign * a,
                            radius_mm: radius,
                        },
                    ],
                )
            }
        };
        let mut segments = segments;
        segments.push(StripSegment::Straight { length_mm: leg });
        let relief_mm = match &f.relief {
            Some(r) => {
                let w = mm(&r.width)?;
                let d = mm(&r.depth)?;
                if w <= 0. || d <= 0. {
                    return Err(what("relief width and depth must be positive"));
                }
                if start <= 0. && end <= 0. {
                    return Err(what("a relief needs an inset flange end"));
                }
                Some([w, d])
            }
            None => None,
        };
        flanges.push(ResolvedFlange {
            index,
            edge: f.edge as usize,
            kind,
            direction: f.direction,
            segments,
            inside_radius_mm: radius,
            leg_mm: leg,
            start_inset_mm: start,
            end_inset_mm: end,
            relief_mm,
        });
    }
    let mut cutouts = vec![];
    for (i, c) in sheet.cutouts.iter().enumerate() {
        let shape = match &c.shape {
            CutoutShape::Circle { center, diameter } => ResolvedShape::Circle {
                center: [mm(&center[0])?, mm(&center[1])?],
                diameter_mm: mm(diameter)?,
            },
            CutoutShape::Rectangle { center, size } => ResolvedShape::Rectangle {
                center: [mm(&center[0])?, mm(&center[1])?],
                size_mm: [mm(&size[0])?, mm(&size[1])?],
            },
        };
        let positive = match shape {
            ResolvedShape::Circle { diameter_mm, .. } => diameter_mm > 0.,
            ResolvedShape::Rectangle { size_mm, .. } => size_mm[0] > 0. && size_mm[1] > 0.,
        };
        if !positive {
            return Err(invalid(format!(
                "Cutout {}: its size must be positive",
                i + 1
            )));
        }
        cutouts.push(ResolvedCutout {
            face: c.face,
            shape,
            source: CutoutSource::Cutout { index: i },
        });
    }
    let patterns = resolve_patterns(&sheet.fasteners, &mm)?;
    for p in &patterns {
        for (point, center) in p.points.iter().enumerate() {
            cutouts.push(ResolvedCutout {
                face: p.face,
                shape: ResolvedShape::Circle {
                    center: *center,
                    diameter_mm: p.preset.hole_mm(),
                },
                source: CutoutSource::Fastener {
                    pattern: p.index,
                    point,
                },
            });
        }
    }
    Ok(ResolvedSheet {
        stock: sheet.stock,
        stock_spec: spec,
        thickness_mm,
        inside_radius_mm,
        radius_source,
        k_factor,
        k_factor_source,
        reverse: sheet.reverse,
        base,
        flanges,
        cutouts,
        patterns,
    })
}
