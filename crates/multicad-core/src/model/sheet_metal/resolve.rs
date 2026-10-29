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
    })
}
