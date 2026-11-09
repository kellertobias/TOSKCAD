//! Unfolding a selected sheet Part: the developed pattern producer contract
//! (cut loops, bend lines and drill points in explicit millimeters, linked to
//! the Part and feature), the bend schedule and scoped process diagnostics.
use crate::{
    Model, ModelError, error,
    sheet_forming::flat_cutout,
    sheet_metal::{BendRecord, bend_records, flat_contours},
};
use multicad_core::{
    document::{Document, DocumentError, EntityId},
    manufacturing::{PlanarContours, PlanarEdge},
    materials::StockKind,
    model::{self, CutoutFace, ResolvedShape, ValueSource},
    references::ReferenceFrame,
};
use multicad_kernel::CurvedEdge2d;

/// One bend line of a developed pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct BendLine {
    /// Stable within the feature: `flange.<i>.bend.<j>`.
    pub id: String,
    /// Proposed bending order: flange order, then bend order within it.
    pub order: usize,
    pub record: BendRecord,
}
/// A circular cutout's center, for drilling or punching.
#[derive(Clone, Debug, PartialEq)]
pub struct DrillPoint {
    /// Stable within the feature: `cutout.<i>` or `fastener.<p>.<j>`.
    pub id: String,
    pub center: [f64; 2],
    pub diameter_mm: f64,
}
/// The scope of a diagnostic: what it is based on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticScope {
    /// Checked exactly against the model and its stock.
    Model,
    /// A general rule of thumb; real limits depend on material and tooling.
    RuleOfThumb,
    /// Needs data the document does not hold.
    NeedsData,
}
impl DiagnosticScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::RuleOfThumb => "rule_of_thumb",
            Self::NeedsData => "needs_data",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub kind: &'static str,
    pub scope: DiagnosticScope,
    /// The bend line or cutout it concerns, if any.
    pub subject: Option<String>,
    pub message: String,
}
/// The developed blank of one sheet Part. Coordinates are base-frame
/// millimeters; `frame` places them in the world. Cut loops are closed: the
/// outer loop counterclockwise, openings clockwise.
#[derive(Clone, Debug, PartialEq)]
pub struct DevelopedPattern {
    pub part: EntityId,
    pub feature: EntityId,
    pub name: String,
    pub units: &'static str,
    pub frame: ReferenceFrame,
    pub thickness_mm: f64,
    pub k_factor: f64,
    pub k_factor_source: ValueSource,
    pub cut: PlanarContours,
    pub bends: Vec<BendLine>,
    pub drills: Vec<DrillPoint>,
    pub size_mm: [f64; 2],
    pub area_mm2: f64,
    pub diagnostics: Vec<Diagnostic>,
}

impl Model {
    /// Unfold one sheet Part. The Part keeps its folded geometry; the
    /// pattern includes all compatible sketch cuts in the current producer.
    /// Other downstream modifications and consumed Parts cannot be developed.
    pub fn unfold(
        &self,
        document: &Document,
        part: EntityId,
    ) -> Result<DevelopedPattern, ModelError> {
        let name = document
            .get(part)
            .map(|e| e.name.clone())
            .ok_or_else(|| error(Some(part), "Choose a Part to unfold"))?;
        let feature = model::sheet_feature(document, part)
            .ok_or_else(|| error(Some(part), format!("{name} is not a sheet part")))?;
        let layout = self
            .sheet_head(document, part)
            .and_then(|head| self.snapshot().sheet_layouts.get(&head))
            .ok_or_else(|| {
                error(
                    Some(part),
                    format!(
                        "{name} is unevaluated, consumed, or has a feature that cannot be developed"
                    ),
                )
            })?;
        let e = |e: DocumentError| error(Some(feature), e);
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let source = model::read_sheet(document, feature).map_err(e)?;
        let sheet = model::resolve_sheet(document, &parameters, &source).map_err(e)?;
        let cut = flat_contours(layout, self.policy)
            .map_err(|m| error(Some(feature), format!("Development failed: {m}")))?;
        let [min, max] = cut
            .bounds_mm()
            .ok_or_else(|| error(Some(feature), "The flat pattern has no bounds"))?;
        let size_mm = [max[0] - min[0], max[1] - min[1]];
        let (t, k) = (layout.thickness_mm, layout.k_factor);
        let bends: Vec<_> = bend_records(layout)
            .into_iter()
            .enumerate()
            .map(|(order, record)| BendLine {
                id: format!("flange.{}.bend.{}", record.flange, record.bend),
                order,
                record,
            })
            .collect();
        let mut drills = vec![];
        let mut outlines = vec![];
        for c in &layout.cutouts {
            let outline = flat_cutout(c, &layout.flanges, t, k)
                .ok_or_else(|| error(Some(feature), "A cutout names a missing flange"))?;
            if let (
                ResolvedShape::Circle { diameter_mm, .. },
                Some(CurvedEdge2d::Circle { center, .. }),
            ) = (c.shape, outline.first())
            {
                drills.push(DrillPoint {
                    id: c.source.id(),
                    center: *center,
                    diameter_mm,
                });
            }
            outlines.push((c.source.id(), c.face, outline));
        }
        for c in &layout.profile_cuts {
            let id = format!("profile_cut.{}", c.feature);
            if let [CurvedEdge2d::Circle { center, radius_mm }] = c.outline.as_slice() {
                drills.push(DrillPoint {
                    id: id.clone(),
                    center: *center,
                    diameter_mm: radius_mm * 2.,
                });
            }
            outlines.push((id, c.face, c.outline.clone()));
        }
        let mut diagnostics = vec![];
        // Hole-to-bend proximity in flat coordinates.
        let reach = |rb: f64| 2. * t + rb;
        for (id, face, outline) in &outlines {
            for b in &bends {
                let f = layout
                    .flanges
                    .iter()
                    .find(|f| f.flange.index == b.record.flange)
                    .expect("bend flange");
                let [(a0, a1), (o0, o1)] = local_bounds(outline, f.start, f.along, f.outward);
                let [z0, z1] = b.record.zone_mm;
                let da = (a0 - f.width).max(-a1).max(0.);
                let dz = (z0 - o1).max(o0 - z1).max(0.);
                let distance = da.hypot(dz);
                let limit = reach(b.record.inside_radius_mm);
                if distance < limit {
                    let on = match face {
                        CutoutFace::Base => "base".to_string(),
                        CutoutFace::Flange { index } => format!("flange {}", index + 1),
                    };
                    diagnostics.push(Diagnostic {
                        kind: "hole_near_bend",
                        scope: DiagnosticScope::RuleOfThumb,
                        subject: Some(id.clone()),
                        message: format!(
                            "{id} on the {on} is {distance:.3} mm from the bend zone of {}; \
                             holes closer than 2·t + r ({limit:.3} mm) may distort when bent",
                            b.id
                        ),
                    });
                }
            }
        }
        for f in &layout.flanges {
            let which = format!("flange.{}", f.flange.index);
            let fr = f.flange.inside_radius_mm;
            if f.flange.leg_mm < reach(fr) {
                diagnostics.push(Diagnostic {
                    kind: "short_leg",
                    scope: DiagnosticScope::RuleOfThumb,
                    subject: Some(which.clone()),
                    message: format!(
                        "{which} has a {:.3} mm leg, below 2·t + r ({:.3} mm); the real minimum \
                         depends on the die opening",
                        f.flange.leg_mm,
                        reach(fr)
                    ),
                });
            }
            if fr < t && f.flange.kind != "hem" {
                diagnostics.push(Diagnostic {
                    kind: "tight_radius",
                    scope: DiagnosticScope::RuleOfThumb,
                    subject: Some(which.clone()),
                    message: format!(
                        "{which} bends with a {fr:.3} mm inside radius, below the {t} mm \
                         thickness; check the material's minimum bend radius"
                    ),
                });
            }
            let inset = f.flange.start_inset_mm > 1e-9 || f.flange.end_inset_mm > 1e-9;
            match f.flange.relief_mm {
                None if inset => diagnostics.push(Diagnostic {
                    kind: "missing_relief",
                    scope: DiagnosticScope::RuleOfThumb,
                    subject: Some(which.clone()),
                    message: format!(
                        "{which} stops short of its edge without a relief; the base may tear \
                         at the bend ends"
                    ),
                }),
                Some([_, depth]) if depth < fr + t => diagnostics.push(Diagnostic {
                    kind: "shallow_relief",
                    scope: DiagnosticScope::RuleOfThumb,
                    subject: Some(which.clone()),
                    message: format!(
                        "{which} has a {depth:.3} mm relief, shallower than r + t ({:.3} mm)",
                        fr + t
                    ),
                }),
                _ => {}
            }
        }
        if let StockKind::Sheet {
            length_mm,
            width_mm,
            ..
        } = sheet.stock_spec.specification
        {
            let fits = |a: f64, b: f64| a <= length_mm + 1e-9 && b <= width_mm + 1e-9;
            if !fits(size_mm[0], size_mm[1]) && !fits(size_mm[1], size_mm[0]) {
                diagnostics.push(Diagnostic {
                    kind: "exceeds_stock",
                    scope: DiagnosticScope::Model,
                    subject: None,
                    message: format!(
                        "The {:.3} × {:.3} mm blank does not fit the {length_mm} × {width_mm} mm \
                         stock sheet",
                        size_mm[0], size_mm[1]
                    ),
                });
            }
        }
        diagnostics.push(Diagnostic {
            kind: "tooling",
            scope: DiagnosticScope::NeedsData,
            subject: None,
            message: "No press-brake, die, punch or springback data is defined: bend \
                      feasibility, angle compensation and the bending sequence are not verified"
                .into(),
        });
        Ok(DevelopedPattern {
            part,
            feature,
            name,
            units: "mm",
            frame: ReferenceFrame {
                origin_mm: layout.frame.origin,
                u: layout.frame.u,
                v: layout.frame.v,
            },
            thickness_mm: t,
            k_factor: k,
            k_factor_source: sheet.k_factor_source,
            area_mm2: cut.area_mm2(),
            cut,
            bends,
            drills,
            size_mm,
            diagnostics,
        })
    }
}
/// Exact bounds of a cutout outline in a flange's (along, outward) frame.
fn local_bounds(
    outline: &[CurvedEdge2d],
    start: [f64; 2],
    along: [f64; 2],
    outward: [f64; 2],
) -> [(f64, f64); 2] {
    let local = |p: [f64; 2]| {
        let d = [p[0] - start[0], p[1] - start[1]];
        [
            d[0] * along[0] + d[1] * along[1],
            d[0] * outward[0] + d[1] * outward[1],
        ]
    };
    let mut a = (f64::INFINITY, f64::NEG_INFINITY);
    let mut o = (f64::INFINITY, f64::NEG_INFINITY);
    let mut add = |q: [f64; 2], r: f64| {
        a = (a.0.min(q[0] - r), a.1.max(q[0] + r));
        o = (o.0.min(q[1] - r), o.1.max(q[1] + r));
    };
    let reflected = along[0] * outward[1] - along[1] * outward[0] < 0.;
    for e in outline {
        match *e {
            CurvedEdge2d::Line { a, b } => {
                add(local(a), 0.);
                add(local(b), 0.);
            }
            CurvedEdge2d::Arc {
                a,
                b,
                center,
                clockwise,
            } => {
                let [a, b, center] = [a, b, center].map(local);
                let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                let end = (b[1] - center[1]).atan2(b[0] - center[0]);
                let tau = std::f64::consts::TAU;
                let sweep_rad = if clockwise ^ reflected {
                    -(start - end).rem_euclid(tau)
                } else {
                    (end - start).rem_euclid(tau)
                };
                let bounds = PlanarEdge::Arc {
                    a,
                    b,
                    center,
                    sweep_rad,
                }
                .bounds();
                for point in bounds {
                    add(point, 0.);
                }
            }
            CurvedEdge2d::Circle { center, radius_mm } => add(local(center), radius_mm),
            CurvedEdge2d::BSpline(_) => {}
        }
    }
    [a, o]
}
