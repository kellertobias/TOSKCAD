//! Panel and carcass resolution: finished panel boxes in world millimeters,
//! and core cut dimensions derived from finished sizes, banding and
//! allowances. The native evaluator builds the solids from these boxes; cut
//! records measure those solids.
use super::*;
use crate::{model::common::number, parameters::EvaluatedParameters};

const EPS: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedBand {
    pub thickness_mm: f64,
    pub allowance_mm: f64,
}
/// A finished rectangular panel: `origin` is a corner, `u` and `v` are unit
/// world directions and the thickness runs along u × v.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelBox {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub length_mm: f64,
    pub width_mm: f64,
    pub thickness_mm: f64,
}
impl PanelBox {
    pub fn normal(&self) -> [f64; 3] {
        cross(self.u, self.v)
    }
    pub fn volume_mm3(&self) -> f64 {
        self.length_mm * self.width_mm * self.thickness_mm
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum PanelShape {
    Box(PanelBox),
    Profile {
        profile: ProfileSource,
        thickness_mm: f64,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPanel {
    pub shape: PanelShape,
    pub stock: EntityId,
    pub stock_spec: Stock,
    pub grain: GrainAxis,
    /// Banding on u_start, u_end, v_start and v_end.
    pub edges: [Option<ResolvedBand>; 4],
    pub oversize_mm: f64,
}
/// Finished and core sizes, along then across the grain. With free grain,
/// the longer finished side comes first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutDimensions {
    pub finished_mm: [f64; 2],
    pub core_mm: [f64; 2],
    pub thickness_mm: f64,
    /// False when the panel grain is free and either orientation may be cut.
    pub grain_fixed: bool,
}

pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn mm(d: &Dimension, p: &EvaluatedParameters) -> Result<f64, DocumentError> {
    number(d, Unit::Millimeter, p)
}
fn band(b: &Band, p: &EvaluatedParameters) -> Result<ResolvedBand, DocumentError> {
    let thickness_mm = mm(&b.thickness, p)?;
    let allowance_mm = b.allowance.as_ref().map_or(Ok(0.), |a| mm(a, p))?;
    if thickness_mm <= 0. || allowance_mm < 0. {
        return Err(invalid(
            "Edge band thickness must be positive and its allowance not negative",
        ));
    }
    Ok(ResolvedBand {
        thickness_mm,
        allowance_mm,
    })
}
pub fn stock_thickness(stock: &Stock) -> f64 {
    match stock.specification {
        StockKind::Panel { thickness_mm, .. } => thickness_mm,
        _ => 0.,
    }
}

/// Resolve one panel against its stock, carcass and parameters.
pub fn resolve_panel(
    document: &Document,
    parameters: &EvaluatedParameters,
    panel: &PanelDefinition,
) -> Result<ResolvedPanel, DocumentError> {
    let oversize_mm = panel
        .oversize
        .as_ref()
        .map_or(Ok(0.), |d| mm(d, parameters))?;
    if oversize_mm < 0. {
        return Err(invalid("Panel oversize cannot be negative"));
    }
    let mut edges = [None; 4];
    for (slot, b) in edges.iter_mut().zip(panel.edges.bands()) {
        *slot = b.map(|b| band(b, parameters)).transpose()?;
    }
    let (shape, stock, grain) = match &panel.source {
        PanelSource::Rectangle {
            stock,
            plane,
            origin,
            length,
            width,
        } => {
            let spec = read_panel_stock(document, *stock)?;
            let (u, v) = plane.axes();
            let length_mm = mm(length, parameters)?;
            let width_mm = mm(width, parameters)?;
            if length_mm <= 0. || width_mm <= 0. {
                return Err(invalid("Panel length and width must be positive"));
            }
            let origin = [
                mm(&origin[0], parameters)?,
                mm(&origin[1], parameters)?,
                mm(&origin[2], parameters)?,
            ];
            (
                PanelShape::Box(PanelBox {
                    origin,
                    u,
                    v,
                    length_mm,
                    width_mm,
                    thickness_mm: stock_thickness(&spec),
                }),
                (*stock, spec),
                panel.grain,
            )
        }
        PanelSource::Profile { stock, profile } => {
            let spec = read_panel_stock(document, *stock)?;
            (
                PanelShape::Profile {
                    profile: profile.clone(),
                    thickness_mm: stock_thickness(&spec),
                },
                (*stock, spec),
                panel.grain,
            )
        }
        PanelSource::Carcass { carcass, role } => {
            let source = read_carcass(document, *carcass)?;
            let resolved = resolve_carcass(document, parameters, &source)?;
            let rp = resolved
                .panels
                .into_iter()
                .find(|p| p.role == *role)
                .ok_or_else(|| {
                    invalid(format!(
                        "The carcass no longer has a {} panel",
                        role.label()
                    ))
                })?;
            edges = rp.edges;
            let stock = source
                .stock_for(*role)
                .ok_or_else(|| invalid("The carcass has no back"))?;
            (
                PanelShape::Box(rp.panel),
                (stock, read_panel_stock(document, stock)?),
                rp.grain,
            )
        }
    };
    Ok(ResolvedPanel {
        shape,
        stock: stock.0,
        stock_spec: stock.1,
        grain,
        edges,
        oversize_mm,
    })
}

impl ResolvedPanel {
    /// Core cut dimensions for a measured finished size along u and v.
    pub fn cut(
        &self,
        finished_u: f64,
        finished_v: f64,
        thickness_mm: f64,
    ) -> Result<CutDimensions, DocumentError> {
        let removed = |a: Option<ResolvedBand>, b: Option<ResolvedBand>| {
            [a, b]
                .into_iter()
                .flatten()
                .map(|b| b.thickness_mm - b.allowance_mm)
                .sum::<f64>()
        };
        let core_u = finished_u - removed(self.edges[0], self.edges[1]) + self.oversize_mm;
        let core_v = finished_v - removed(self.edges[2], self.edges[3]) + self.oversize_mm;
        if core_u <= EPS || core_v <= EPS {
            return Err(invalid("Edge banding leaves no panel core"));
        }
        let (finished_mm, core_mm, grain_fixed) = match self.grain {
            GrainAxis::U => ([finished_u, finished_v], [core_u, core_v], true),
            GrainAxis::V => ([finished_v, finished_u], [core_v, core_u], true),
            GrainAxis::Free if finished_v > finished_u => {
                ([finished_v, finished_u], [core_v, core_u], false)
            }
            GrainAxis::Free => ([finished_u, finished_v], [core_u, core_v], false),
        };
        Ok(CutDimensions {
            finished_mm,
            core_mm,
            thickness_mm,
            grain_fixed,
        })
    }
}
/// Whether a core cut fits one stock sheet, honoring grain when both the
/// stock and the panel have one.
pub fn fits_stock(stock: &Stock, cut: &CutDimensions) -> bool {
    let StockKind::Panel {
        length_mm,
        width_mm,
        grain,
        ..
    } = stock.specification
    else {
        return false;
    };
    let [along, across] = cut.core_mm;
    let fits = |a: f64, b: f64| a <= length_mm + EPS && b <= width_mm + EPS;
    match (grain, cut.grain_fixed) {
        (materials::Grain::AlongLength, true) => fits(along, across),
        (materials::Grain::AcrossLength, true) => fits(across, along),
        _ => fits(along, across) || fits(across, along),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarcassPanel {
    pub role: PanelRole,
    pub panel: PanelBox,
    pub grain: GrainAxis,
    pub edges: [Option<ResolvedBand>; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCarcass {
    pub origin: [f64; 3],
    pub rotation_rad: f64,
    pub size_mm: [f64; 3],
    pub panels: Vec<CarcassPanel>,
}

/// Plan every panel of a carcass. Sizes that leave no interior, shelves that
/// do not fit or overlap, and a back deeper than the cabinet are refused.
pub fn resolve_carcass(
    document: &Document,
    parameters: &EvaluatedParameters,
    carcass: &CarcassDefinition,
) -> Result<ResolvedCarcass, DocumentError> {
    let p = parameters;
    carcass.validate()?;
    let t = stock_thickness(&read_panel_stock(document, carcass.stock)?);
    let (w, h, d) = (
        mm(&carcass.width, p)?,
        mm(&carcass.height, p)?,
        mm(&carcass.depth, p)?,
    );
    if w <= 2. * t + EPS || h <= 2. * t + EPS || d <= EPS {
        return Err(invalid(format!(
            "The carcass must be wider and taller than two {t} mm panels"
        )));
    }
    let origin = [
        mm(&carcass.origin[0], p)?,
        mm(&carcass.origin[1], p)?,
        mm(&carcass.origin[2], p)?,
    ];
    let rotation_rad = number(&carcass.rotation, Unit::Radian, p)?;
    let toe = carcass
        .cabinet
        .as_ref()
        .map_or(Ok(0.), |c| mm(&c.toe_clearance, p))?;
    if toe < 0. || toe + 2. * t >= h - EPS {
        return Err(invalid(
            "Toe clearance must be nonnegative and leave space above the bottom panel",
        ));
    }
    if toe > EPS && carcass.construction == Construction::TopBottomFullWidth {
        return Err(invalid(
            "Toe clearance requires full-height side panels; choose Sides full height",
        ));
    }
    let front = carcass
        .front_band
        .as_ref()
        .map(|b| band(b, p))
        .transpose()?;
    // Depth of the sides, top and bottom, and where shelves stop at the rear.
    let (body_depth, rear, back) = match &carcass.back {
        Back::None => (d, d, None),
        Back::Inset { stock, recess } => {
            let bt = stock_thickness(&read_panel_stock(document, *stock)?);
            let r = mm(recess, p)?;
            if r < 0. || r + bt >= d - EPS {
                return Err(invalid(
                    "The back recess and thickness must leave cabinet depth in front",
                ));
            }
            let y = d - r - bt;
            (d, y, Some(([t, y, toe + t], [w - t, d - r, h - t])))
        }
        Back::Overlay { stock } => {
            let bt = stock_thickness(&read_panel_stock(document, *stock)?);
            if bt >= d - EPS {
                return Err(invalid("The back is thicker than the cabinet depth"));
            }
            (d - bt, d - bt, Some(([0., d - bt, 0.], [w, d, h])))
        }
    };
    let (sides_z, plates_x) = match carcass.construction {
        Construction::SidesFullHeight => ([0., h], [t, w - t]),
        Construction::TopBottomFullWidth => ([t, h - t], [0., w]),
    };
    let mut boxes = vec![
        (
            PanelRole::LeftSide,
            [0., 0., sides_z[0]],
            [t, body_depth, sides_z[1]],
        ),
        (
            PanelRole::RightSide,
            [w - t, 0., sides_z[0]],
            [w, body_depth, sides_z[1]],
        ),
        (
            PanelRole::Top,
            [plates_x[0], 0., h - t],
            [plates_x[1], body_depth, h],
        ),
        (
            PanelRole::Bottom,
            [plates_x[0], 0., toe],
            [plates_x[1], body_depth, toe + t],
        ),
    ];
    if let Some((min, max)) = back {
        boxes.push((PanelRole::Back, min, max));
    }
    let setback = mm(&carcass.shelves.setback, p)?;
    let clearance = mm(&carcass.shelves.clearance, p)?;
    let count = carcass
        .cabinet
        .as_ref()
        .map_or_else(|| carcass.shelves.layout.count(), |c| c.shelves.len());
    if count > 0 {
        if setback < 0. || clearance < 0. {
            return Err(invalid("Shelf setback and clearance cannot be negative"));
        }
        if rear - setback <= EPS || w - 2. * t - 2. * clearance <= EPS {
            return Err(invalid("The shelf setback or clearance leaves no shelf"));
        }
    }
    let interior = h - 2. * t;
    let bottoms: Vec<f64> = if let Some(cabinet) = &carcass.cabinet {
        read_panel_stock(document, cabinet.shelf_stock)?;
        let mut intervals = vec![];
        for shelf in &cabinet.shelves {
            let z = mm(&shelf.underside, p)?;
            let depth = mm(&shelf.depth, p)?;
            let thickness = stock_thickness(&read_panel_stock(
                document,
                shelf.stock.unwrap_or(cabinet.shelf_stock),
            )?);
            if z < toe + t - EPS || z + thickness > h - t + EPS {
                return Err(invalid(format!(
                    "Shelf {} underside/thickness does not fit between the bottom and top panels",
                    shelf.key + 1
                )));
            }
            if depth <= EPS || setback + depth > rear + EPS {
                return Err(invalid(format!(
                    "Shelf {} requested depth {depth} mm does not fit before the back; change its depth or the back configuration",
                    shelf.key + 1
                )));
            }
            if intervals
                .iter()
                .any(|(lo, hi): &(f64, f64)| z < *hi - EPS && z + thickness > *lo + EPS)
            {
                return Err(invalid(format!(
                    "Shelf {} overlaps another shelf",
                    shelf.key + 1
                )));
            }
            if shelf.support == ShelfSupport::System32 {
                let diameter = mm(&cabinet.pins.diameter, p)?;
                let start = toe + t + mm(&cabinet.pins.start, p)?;
                let end = h - t - mm(&cabinet.pins.end_clearance, p)?;
                let center = z - diameter / 2.;
                let level = (center - start) / SYSTEM32_PITCH_MM;
                if diameter <= EPS
                    || center < start - EPS
                    || center > end + EPS
                    || (level - level.round()).abs() > EPS
                {
                    return Err(invalid(format!(
                        "Shelf {} must rest on a System 32 support level (underside = first hole center + 32 mm × level + pin radius)",
                        shelf.key + 1
                    )));
                }
            }
            intervals.push((z, z + thickness));
        }
        cabinet
            .shelves
            .iter()
            .map(|s| mm(&s.underside, p))
            .collect::<Result<_, _>>()?
    } else {
        match &carcass.shelves.layout {
            ShelfLayout::Even { count } => {
                let n = *count as f64;
                let gap = (interior - n * t) / (n + 1.);
                if *count > 0 && gap <= EPS {
                    return Err(invalid(format!(
                        "{count} shelves do not fit the cabinet interior"
                    )));
                }
                (0..*count)
                    .map(|i| t + (i as f64 + 1.) * gap + i as f64 * t)
                    .collect()
            }
            ShelfLayout::Offsets { offsets } => {
                let mut out = vec![];
                for (i, o) in offsets.iter().enumerate() {
                    let z = t + mm(o, p)?;
                    if z < t - EPS || z + t > h - t + EPS {
                        return Err(invalid(format!(
                            "Shelf {} is outside the cabinet interior",
                            i + 1
                        )));
                    }
                    if out.last().is_some_and(|prev: &f64| z < prev + t - EPS) {
                        return Err(invalid(format!(
                            "Shelf {} overlaps the shelf below; list offsets bottom to top",
                            i + 1
                        )));
                    }
                    out.push(z);
                }
                out
            }
        }
    };
    for (i, z) in bottoms.into_iter().enumerate() {
        let (key, thickness, end, gap) = if let Some(cabinet) = &carcass.cabinet {
            let shelf = &cabinet.shelves[i];
            (
                shelf.key,
                stock_thickness(&read_panel_stock(
                    document,
                    shelf.stock.unwrap_or(cabinet.shelf_stock),
                )?),
                setback + mm(&shelf.depth, p)?,
                if shelf.support == ShelfSupport::Fixed {
                    0.
                } else {
                    clearance
                },
            )
        } else {
            (i as u32, t, rear, clearance)
        };
        boxes.push((
            PanelRole::Shelf { index: key },
            [t + gap, setback, z],
            [w - t - gap, end, z + thickness],
        ));
    }
    let (s, c) = rotation_rad.sin_cos();
    let rotate = |a: [f64; 3]| [a[0] * c - a[1] * s, a[0] * s + a[1] * c, a[2]];
    let place = |a: [f64; 3]| {
        let r = rotate(a);
        [origin[0] + r[0], origin[1] + r[1], origin[2] + r[2]]
    };
    let panels = boxes
        .into_iter()
        .map(|(role, min, max)| {
            let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
            // (u axis, v axis, corner); thickness runs along u × v.
            let (u, v, corner, grain, edges) = match role {
                PanelRole::LeftSide | PanelRole::RightSide => {
                    (1, 2, min, GrainAxis::V, [front, None, None, None])
                }
                PanelRole::Back => (0, 2, [min[0], max[1], min[2]], GrainAxis::V, [None; 4]),
                _ => (0, 1, min, GrainAxis::U, [None, None, front, None]),
            };
            let axis = |i: usize| {
                let mut a = [0.; 3];
                a[i] = 1.;
                rotate(a)
            };
            let thickness = 3 - u - v;
            CarcassPanel {
                role,
                panel: PanelBox {
                    origin: place(corner),
                    u: axis(u),
                    v: axis(v),
                    length_mm: size[u],
                    width_mm: size[v],
                    thickness_mm: size[thickness],
                },
                grain,
                edges,
            }
        })
        .collect();
    Ok(ResolvedCarcass {
        origin,
        rotation_rad,
        size_mm: [w, h, d],
        panels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(thickness: f64, grain: materials::Grain) -> Stock {
        Stock {
            material: EntityId::new(),
            specification: StockKind::Panel {
                thickness_mm: thickness,
                length_mm: 2800.,
                width_mm: 2070.,
                grain,
            },
        }
    }
    #[test]
    fn banding_and_allowances_change_core_but_not_finished_size() {
        let band = ResolvedBand {
            thickness_mm: 2.,
            allowance_mm: 0.5,
        };
        let panel = ResolvedPanel {
            shape: PanelShape::Box(PanelBox {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                length_mm: 500.,
                width_mm: 700.,
                thickness_mm: 18.,
            }),
            stock: EntityId::new(),
            stock_spec: spec(18., materials::Grain::AlongLength),
            grain: GrainAxis::V,
            edges: [Some(band), None, Some(band), Some(band)],
            oversize_mm: 1.,
        };
        let cut = panel.cut(500., 700., 18.).unwrap();
        // Grain along v: 700 first. v loses two bands, u loses one.
        assert_eq!(cut.finished_mm, [700., 500.]);
        assert_eq!(cut.core_mm, [700. - 3. + 1., 500. - 1.5 + 1.]);
        assert!(fits_stock(&panel.stock_spec, &cut));
        let long = panel.cut(500., 2900., 18.).unwrap();
        assert!(!fits_stock(&panel.stock_spec, &long));
        // Across-length stock grain turns the cut; free grain may turn too.
        let across = spec(18., materials::Grain::AcrossLength);
        assert!(!fits_stock(&across, &panel.cut(500., 2500., 18.).unwrap()));
        let free = ResolvedPanel {
            grain: GrainAxis::Free,
            ..panel.clone()
        };
        assert!(fits_stock(&across, &free.cut(500., 2500., 18.).unwrap()));
    }
}
