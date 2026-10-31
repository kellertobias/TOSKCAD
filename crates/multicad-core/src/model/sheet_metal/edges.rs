//! Base edges and flange guides for choosing flanges visually.
//!
//! The window draws the base outline with numbered edges, the flange each
//! edge carries and its bend and leg dimensions; MCP reports the same data.
//! Everything here comes from the resolved sheet (stock thickness, the part
//! or flange inside radius and the flange intent), so a guide shows the
//! values the native evaluator builds. A guide is an annotation of the
//! folded cross-section, not manufacturing geometry.
use super::*;
use crate::parameters::EvaluatedParameters;
use std::f64::consts::PI;

/// One edge of the base outer loop, in world millimeters on the base face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetEdge {
    /// Index in the base outer loop: the `edge` of a flange.
    pub index: usize,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub length_mm: f64,
    /// In-plane unit direction away from the base, for a straight edge.
    pub outward: [f64; 3],
    /// Only straight edges can carry a flange.
    pub straight: bool,
}
impl SheetEdge {
    pub fn along(&self) -> [f64; 3] {
        let d = sub(self.end_mm, self.start_mm);
        scale(d, 1. / self.length_mm.max(1e-12))
    }
    pub fn midpoint_mm(&self) -> [f64; 3] {
        lerp(self.start_mm, self.end_mm, 0.5)
    }
}

/// The base outline of a sheet: its edges and the material direction.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetOutline {
    pub edges: Vec<SheetEdge>,
    /// Unit thickness direction from the base face (u × v, or its opposite
    /// when the part is reversed). An `Up` flange bends toward it.
    pub material: [f64; 3],
    pub thickness_mm: f64,
}

/// A flange's folded cross-section at one position along its edge.
#[derive(Clone, Debug, PartialEq)]
pub struct FlangeGuide {
    pub flange: usize,
    pub edge: usize,
    /// The flange span on the base edge: start and end of the flange after
    /// its insets, on the base face.
    pub span_mm: [[f64; 3]; 2],
    /// Mid-thickness cross-section polyline in section coordinates:
    /// x outward from the edge, y along the material direction.
    pub section: Vec<[f64; 2]>,
    /// Section points where the straight leg starts and ends.
    pub leg: [[f64; 2]; 2],
    /// Each bend: the section point halfway round it and its signed angle
    /// in degrees (positive toward the material side).
    pub bends: Vec<([f64; 2], f64)>,
    pub leg_mm: f64,
    pub inside_radius_mm: f64,
    pub direction: BendDirection,
    pub kind: &'static str,
}
impl FlangeGuide {
    /// A section point in world millimeters at `along_mm` from the span
    /// start.
    pub fn world(&self, outline: &SheetOutline, point: [f64; 2], along_mm: f64) -> [f64; 3] {
        let edge = &outline.edges[self.edge];
        let base = add(self.span_mm[0], scale(edge.along(), along_mm));
        add(
            base,
            add(
                scale(edge.outward, point[0]),
                scale(outline.material, point[1]),
            ),
        )
    }
    pub fn width_mm(&self) -> f64 {
        norm(sub(self.span_mm[1], self.span_mm[0]))
    }
}

/// A finding about the flanges of a sheet definition, before or instead of
/// native evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlangeIssue {
    /// Indices into `SheetDefinition::flanges`.
    pub flanges: Vec<usize>,
    /// Base edges involved.
    pub edges: Vec<usize>,
    pub message: String,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    add(a, scale(sub(b, a), t))
}

/// Start, end and straightness of an outline edge in base-frame mm.
type LoopEdge = ([f64; 2], [f64; 2], bool);

/// The base outline of a rectangle or captured-contour base. A sketch
/// region base needs the solved sketch and returns `None`.
pub fn sheet_outline(sheet: &ResolvedSheet) -> Option<SheetOutline> {
    let (frame, loop2d): (SheetFrame, Vec<LoopEdge>) = match &sheet.base {
        ResolvedBase::Rectangle {
            frame,
            length_mm: l,
            width_mm: w,
        } => {
            let c = [[0., 0.], [*l, 0.], [*l, *w], [0., *w]];
            (
                *frame,
                (0..4).map(|i| (c[i], c[(i + 1) % 4], true)).collect(),
            )
        }
        ResolvedBase::Contour { frame, loops } => (
            *frame,
            loops[0]
                .edges
                .iter()
                .map(|e| {
                    let (a, b) = e.ends();
                    (
                        a,
                        b,
                        matches!(e, crate::manufacturing::PlanarEdge::Line { .. }),
                    )
                })
                .collect(),
        ),
        ResolvedBase::Profile { .. } => return None,
    };
    let n = frame.normal();
    let material = if sheet.reverse { scale(n, -1.) } else { n };
    let world = |p: [f64; 2]| {
        add(
            frame.origin,
            add(scale(frame.u, p[0]), scale(frame.v, p[1])),
        )
    };
    let edges = loop2d
        .into_iter()
        .enumerate()
        .map(|(index, (a, b, straight))| {
            let d = [b[0] - a[0], b[1] - a[1]];
            let length_mm = d[0].hypot(d[1]);
            let along = [d[0] / length_mm.max(1e-12), d[1] / length_mm.max(1e-12)];
            // Outer loops run counterclockwise, so outward is to the right.
            let out = [along[1], -along[0]];
            SheetEdge {
                index,
                start_mm: world(a),
                end_mm: world(b),
                length_mm,
                outward: add(scale(frame.u, out[0]), scale(frame.v, out[1])),
                straight,
            }
        })
        .collect();
    Some(SheetOutline {
        edges,
        material,
        thickness_mm: sheet.thickness_mm,
    })
}

/// The mid-thickness cross-section of a resolved flange. A bend of inside
/// radius r turns the mid-surface on a radius r + t/2 about an axis on the
/// inside of the bend, starting at the base edge.
pub fn flange_section(f: &ResolvedFlange, thickness_mm: f64) -> FlangeGuideSection {
    let t = thickness_mm;
    let mut p = [0., t / 2.];
    let mut heading: f64 = 0.;
    let mut points = vec![p];
    let mut bends = vec![];
    let mut leg = [p, p];
    for s in &f.segments {
        match *s {
            StripSegment::Straight { length_mm } => {
                let start = p;
                p = [
                    p[0] + length_mm * heading.cos(),
                    p[1] + length_mm * heading.sin(),
                ];
                points.push(p);
                leg = [start, p];
            }
            StripSegment::Bend {
                angle_rad,
                radius_mm,
            } => {
                let r = radius_mm + t / 2.;
                let side = angle_rad.signum();
                let c = [
                    p[0] - side * r * heading.sin(),
                    p[1] + side * r * heading.cos(),
                ];
                let rotate = |q: [f64; 2], a: f64| {
                    let (s, co) = a.sin_cos();
                    let d = [q[0] - c[0], q[1] - c[1]];
                    [c[0] + co * d[0] - s * d[1], c[1] + s * d[0] + co * d[1]]
                };
                let steps = ((angle_rad.abs() / (PI / 24.)).ceil() as usize).max(2);
                let start = p;
                for i in 1..=steps {
                    points.push(rotate(start, angle_rad * i as f64 / steps as f64));
                }
                bends.push((rotate(start, angle_rad / 2.), angle_rad.to_degrees()));
                p = rotate(start, angle_rad);
                heading += angle_rad;
            }
        }
    }
    FlangeGuideSection { points, leg, bends }
}
#[derive(Clone, Debug, PartialEq)]
pub struct FlangeGuideSection {
    pub points: Vec<[f64; 2]>,
    pub leg: [[f64; 2]; 2],
    pub bends: Vec<([f64; 2], f64)>,
}

/// Guides for every flange of a resolved sheet whose edge is a straight
/// edge of the outline.
pub fn flange_guides(sheet: &ResolvedSheet, outline: &SheetOutline) -> Vec<FlangeGuide> {
    sheet
        .flanges
        .iter()
        .filter_map(|f| {
            let edge = outline.edges.get(f.edge).filter(|e| e.straight)?;
            let along = edge.along();
            let width = edge.length_mm - f.start_inset_mm - f.end_inset_mm;
            if width <= 0. {
                return None;
            }
            let start = add(edge.start_mm, scale(along, f.start_inset_mm));
            let section = flange_section(f, sheet.thickness_mm);
            Some(FlangeGuide {
                flange: f.index,
                edge: f.edge,
                span_mm: [start, add(start, scale(along, width))],
                section: section.points,
                leg: section.leg,
                bends: section.bends,
                leg_mm: f.leg_mm,
                inside_radius_mm: f.inside_radius_mm,
                direction: f.direction,
                kind: f.kind,
            })
        })
        .collect()
}

fn strip_prefix(message: &str) -> String {
    match message.split_once(": ") {
        Some((head, rest)) if head.starts_with("Flange ") => rest.to_owned(),
        _ => message.to_owned(),
    }
}

/// Check the flanges of a sheet definition before native evaluation. Each
/// flange is resolved on its own so every invalid bend is reported, then
/// edge lengths, insets, reliefs and shared corners are checked against the
/// base outline. Flanges on neighboring edges whose insets are both zero at
/// their shared corner touch there, which the evaluator refuses.
///
/// An empty result does not prove the folded part is valid: collisions
/// with the base and with distant flanges are found by native evaluation.
pub fn check_flanges(
    document: &Document,
    parameters: &EvaluatedParameters,
    sheet: &SheetDefinition,
) -> Result<Vec<FlangeIssue>, DocumentError> {
    Ok(review_flanges(document, parameters, sheet)?.issues)
}

/// The base outline, a guide for every flange that resolves on a straight
/// edge, and the pre-check findings of a sheet definition. Invalid flanges
/// have no guide but keep their findings, so a view can still show the
/// rest of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct FlangeReview {
    /// The bend inputs every guide uses, with their sources.
    pub thickness_mm: f64,
    pub inside_radius_mm: f64,
    pub radius_source: ValueSource,
    pub k_factor: f64,
    pub k_factor_source: ValueSource,
    pub outline: Option<SheetOutline>,
    pub guides: Vec<FlangeGuide>,
    pub issues: Vec<FlangeIssue>,
}

/// Review the flanges of a sheet definition; see [`check_flanges`]. Fails
/// only when the sheet itself (stock, base, radius, K-factor) does not
/// resolve.
pub fn review_flanges(
    document: &Document,
    parameters: &EvaluatedParameters,
    sheet: &SheetDefinition,
) -> Result<FlangeReview, DocumentError> {
    let mut issues = vec![];
    let mut seen = std::collections::BTreeMap::new();
    for (i, f) in sheet.flanges.iter().enumerate() {
        if let Some(other) = seen.insert(f.edge, i) {
            issues.push(FlangeIssue {
                flanges: vec![other, i],
                edges: vec![f.edge as usize],
                message: format!("Base edge {} carries more than one flange", f.edge + 1),
            });
        }
    }
    let bare = SheetDefinition {
        flanges: vec![],
        cutouts: vec![],
        fasteners: vec![],
        ..sheet.clone()
    };
    let resolved = resolve_sheet(document, parameters, &bare)?;
    let outline = sheet_outline(&resolved);
    let mut flanges: Vec<Option<ResolvedFlange>> = vec![];
    for (i, f) in sheet.flanges.iter().enumerate() {
        let one = SheetDefinition {
            flanges: vec![f.clone()],
            ..bare.clone()
        };
        match resolve_sheet(document, parameters, &one) {
            Ok(mut r) => {
                let mut flange = r.flanges.remove(0);
                flange.index = i;
                flanges.push(Some(flange));
            }
            Err(e) => {
                issues.push(FlangeIssue {
                    flanges: vec![i],
                    edges: vec![f.edge as usize],
                    message: format!(
                        "Flange on edge {}: {}",
                        f.edge + 1,
                        strip_prefix(&e.to_string())
                    ),
                });
                flanges.push(None);
            }
        }
    }
    let Some(outline) = outline else {
        return Ok(FlangeReview {
            thickness_mm: resolved.thickness_mm,
            inside_radius_mm: resolved.inside_radius_mm,
            radius_source: resolved.radius_source,
            k_factor: resolved.k_factor,
            k_factor_source: resolved.k_factor_source,
            outline: None,
            guides: vec![],
            issues,
        });
    };
    let n = outline.edges.len();
    for f in flanges.iter().flatten() {
        let what = |m: String| FlangeIssue {
            flanges: vec![f.index],
            edges: vec![f.edge],
            message: format!("Flange on edge {}: {m}", f.edge + 1),
        };
        let Some(edge) = outline.edges.get(f.edge) else {
            issues.push(what(format!(
                "the base has no edge {} (it has {n})",
                f.edge + 1
            )));
            continue;
        };
        if !edge.straight {
            issues.push(what(
                "its base edge is curved; flanges need straight edges".into(),
            ));
            continue;
        }
        if edge.length_mm - f.start_inset_mm - f.end_inset_mm <= 1e-6 {
            issues.push(what(format!(
                "its insets leave no flange on the {:.3} mm edge",
                edge.length_mm
            )));
        }
        if let Some([w, _]) = f.relief_mm {
            for inset in [f.start_inset_mm, f.end_inset_mm] {
                if inset > 1e-9 && w > inset + 1e-9 {
                    issues.push(what(format!(
                        "its {w} mm relief is wider than its {inset} mm inset"
                    )));
                }
            }
        }
    }
    let on_edge = |e: usize| flanges.iter().flatten().find(|f| f.edge == e);
    for e in 0..n {
        let next = (e + 1) % n;
        if next == e {
            continue;
        }
        if let (Some(a), Some(b)) = (on_edge(e), on_edge(next))
            && a.end_inset_mm <= 1e-9
            && b.start_inset_mm <= 1e-9
        {
            issues.push(FlangeIssue {
                flanges: vec![a.index, b.index],
                edges: vec![e, next],
                message: format!(
                    "Flanges on edges {} and {} touch at their shared corner; give either one a corner inset",
                    e + 1,
                    next + 1
                ),
            });
        }
    }
    let values = (
        resolved.thickness_mm,
        resolved.inside_radius_mm,
        resolved.radius_source,
        resolved.k_factor,
        resolved.k_factor_source,
    );
    let guides = flange_guides(
        &ResolvedSheet {
            flanges: flanges.into_iter().flatten().collect(),
            ..resolved
        },
        &outline,
    );
    Ok(FlangeReview {
        thickness_mm: values.0,
        inside_radius_mm: values.1,
        radius_source: values.2,
        k_factor: values.3,
        k_factor_source: values.4,
        outline: Some(outline),
        guides,
        issues,
    })
}
