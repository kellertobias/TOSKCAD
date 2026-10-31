//! Native attached hardware: each hardware Part's solids placed at its
//! mounting pattern's points on the evaluated sheet, and the purchased-item
//! records with quantities measured from those solids and compatibility
//! findings. Dimensions are nominal; nothing here is an installation,
//! torque or strength statement.
use crate::{Model, ModelError, SheetLayout, error};
use multicad_core::{
    document::{Document, DocumentError, EntityData, EntityId},
    model::{self, HardwareItem, HardwareSide, ResolvedPreset},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
};
use std::collections::BTreeMap;

fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}
/// A prism of `outer` (and `holes`) drawn in the face plane at `origin`,
/// running `height` along `axis`.
fn prism(
    frame: PlaneFrame,
    origin: [f64; 3],
    outer: Vec<CurvedEdge2d>,
    holes: Vec<Vec<CurvedEdge2d>>,
    axis: [f64; 3],
    height: f64,
    policy: GeometryPolicy,
) -> Result<Solid, String> {
    Solid::extrude_curved_profile_vector(
        &CurvedProfile {
            frame: PlaneFrame { origin, ..frame },
            outer,
            holes,
        },
        axis.map(|c| c * height),
        policy,
    )
    .map_err(|e| e.to_string())
}
fn circle(d: f64) -> Vec<CurvedEdge2d> {
    vec![CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm: d / 2.,
    }]
}
fn hexagon(across_flats: f64) -> Vec<CurvedEdge2d> {
    let r = across_flats / 3f64.sqrt();
    let p: Vec<[f64; 2]> = (0..6)
        .map(|k| {
            let a = k as f64 * std::f64::consts::FRAC_PI_3;
            [r * a.cos(), r * a.sin()]
        })
        .collect();
    (0..6)
        .map(|i| CurvedEdge2d::Line {
            a: p[i],
            b: p[(i + 1) % 6],
        })
        .collect()
}
fn union(acc: Option<Solid>, s: Solid, policy: GeometryPolicy) -> Result<Solid, String> {
    match acc {
        None => Ok(s),
        Some(a) => a
            .boolean(&s, BooleanOperation::Union, policy)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "A hardware union removed all material".into()),
    }
}

/// Build one hardware Part: its item at every point of its pattern.
pub(crate) fn build(
    document: &Document,
    id: EntityId,
    layouts: &BTreeMap<EntityId, SheetLayout>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let m = |m: String| error(Some(id), m);
    let h = model::read_hardware(document, id).map_err(|e: DocumentError| error(Some(id), e))?;
    let layout = layouts
        .get(&h.sheet)
        .ok_or_else(|| m("The hardware's sheet part has not been evaluated".into()))?;
    let pattern = layout.patterns.get(h.pattern as usize).ok_or_else(|| {
        m(format!(
            "Mounting pattern {} no longer exists",
            h.pattern + 1
        ))
    })?;
    if !pattern_items(&pattern.preset).contains(&h.item) {
        return Err(m(format!(
            "Mounting pattern {} no longer has a {}",
            h.pattern + 1,
            h.item.label()
        )));
    }
    let space = layout.space();
    let (frame, n) = space.face_frame(pattern.face, &layout.flanges).map_err(m)?;
    let t = layout.thickness_mm;
    // `out` points away from the face on the hardware side; `surface` is
    // the face on that side, `back` the opposite face.
    let (out, surface_offset, back_offset) = match pattern.side {
        HardwareSide::Material => (n, t, 0.),
        HardwareSide::Base => (n.map(|c| -c), 0., t),
    };
    let back = out.map(|c| -c);
    let mut acc = None;
    for p in &pattern.points {
        let base = frame.point(*p);
        let surface = add(base, n, surface_offset);
        let rear = add(base, n, back_offset);
        let solid = match (&pattern.preset, h.item) {
            (
                ResolvedPreset::Screw {
                    thread,
                    length_mm,
                    washers,
                    ..
                },
                HardwareItem::Screw,
            ) => {
                let d = thread.data();
                let lift = if *washers { d.washer[2] } else { 0. };
                let seat = add(surface, out, lift);
                let head = prism(
                    frame,
                    seat,
                    circle(d.head[0]),
                    vec![],
                    out,
                    d.head[1],
                    policy,
                );
                let shank = prism(
                    frame,
                    seat,
                    circle(d.diameter),
                    vec![],
                    back,
                    *length_mm,
                    policy,
                );
                union(Some(head.map_err(m)?), shank.map_err(m)?, policy).map_err(m)?
            }
            (ResolvedPreset::Screw { thread, nut, .. }, HardwareItem::Washer) => {
                let d = thread.data();
                let ring = |at: [f64; 3], axis: [f64; 3]| {
                    prism(
                        frame,
                        at,
                        circle(d.washer[1]),
                        vec![circle(d.washer[0])],
                        axis,
                        d.washer[2],
                        policy,
                    )
                };
                let mut w = ring(surface, out).map_err(m)?;
                if *nut {
                    w = union(Some(w), ring(rear, back).map_err(m)?, policy).map_err(m)?;
                }
                w
            }
            (
                ResolvedPreset::Screw {
                    thread, washers, ..
                },
                HardwareItem::Nut,
            ) => {
                let d = thread.data();
                let lift = if *washers { d.washer[2] } else { 0. };
                prism(
                    frame,
                    add(rear, back, lift),
                    hexagon(d.nut[0]),
                    vec![circle(d.diameter)],
                    back,
                    d.nut[1],
                    policy,
                )
                .map_err(m)?
            }
            (
                ResolvedPreset::Standoff {
                    thread,
                    length_mm,
                    across_flats_mm,
                },
                _,
            ) => prism(
                frame,
                surface,
                hexagon(*across_flats_mm),
                vec![circle(thread.data().diameter)],
                out,
                *length_mm,
                policy,
            )
            .map_err(m)?,
            (
                ResolvedPreset::Insert {
                    body_mm, height_mm, ..
                },
                _,
            ) => prism(
                frame,
                surface,
                circle(*body_mm),
                vec![],
                out,
                *height_mm,
                policy,
            )
            .map_err(m)?,
            _ => return Err(m("Hardware item does not match its preset".into())),
        };
        acc = Some(union(acc, solid, policy).map_err(m)?);
    }
    acc.ok_or_else(|| m("A mounting pattern needs points".into()))
}
fn pattern_items(preset: &ResolvedPreset) -> Vec<HardwareItem> {
    match preset {
        ResolvedPreset::Screw { washers, nut, .. } => {
            let mut out = vec![HardwareItem::Screw];
            if *washers {
                out.push(HardwareItem::Washer);
            }
            if *nut {
                out.push(HardwareItem::Nut);
            }
            out
        }
        ResolvedPreset::Standoff { .. } => vec![HardwareItem::Standoff],
        ResolvedPreset::Insert { .. } => vec![HardwareItem::Insert],
    }
}

/// One purchased hardware item of a sheet part, for BOM producers.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareRecord {
    pub part: EntityId,
    pub feature: EntityId,
    pub sheet_part: EntityId,
    pub sheet_feature: EntityId,
    pub pattern: usize,
    pub item: HardwareItem,
    pub designation: String,
    /// Where the dimensions come from: a standard's nominal values, the
    /// user, or a named catalogue.
    pub source: String,
    /// Measured: the number of separate solids in the hardware Part.
    pub quantity: usize,
    /// Receiving hole diameter cut into the sheet.
    pub hole_mm: f64,
    /// Compatibility findings: stack length, sheet thickness range, or
    /// hardware that intersects the sheet.
    pub findings: Vec<String>,
}

impl Model {
    /// Records for every evaluated hardware Part, in document order.
    pub fn sheet_hardware(&self, document: &Document) -> Result<Vec<HardwareRecord>, ModelError> {
        let mut out = vec![];
        for entity in model::hardware_features(document) {
            let id = entity.id;
            let h = model::read_hardware(document, id).map_err(|e| error(Some(id), e))?;
            let part = h.output.body;
            let (Some(layout), Some(head)) = (
                self.snapshot().sheet_layouts.get(&h.sheet),
                self.sheet_head(document, part),
            ) else {
                continue;
            };
            let Some(solid) = self.solid(head) else {
                continue;
            };
            let sheet = model::read_sheet(document, h.sheet).map_err(|e| error(Some(id), e))?;
            let pattern = &layout.patterns[h.pattern as usize];
            let (designation, source) = pattern.preset.identity(h.item);
            let quantity = solid.solid_count().map_err(|e| error(Some(id), e))? as usize;
            let t = layout.thickness_mm;
            let mut findings = vec![];
            match &pattern.preset {
                ResolvedPreset::Screw {
                    thread,
                    length_mm,
                    washers,
                    nut,
                } if h.item == HardwareItem::Screw => {
                    let d = thread.data();
                    let stack =
                        t + if *washers {
                            d.washer[2] * (1. + f64::from(u8::from(*nut)))
                        } else {
                            0.
                        } + if *nut { d.nut[1] } else { 0. };
                    if *length_mm + 1e-9 < stack {
                        findings.push(format!(
                            "The {length_mm} mm screw is shorter than the {stack:.3} mm stack of \
                             sheet{}",
                            match (washers, nut) {
                                (true, true) => ", washers and nut",
                                (false, true) => " and nut",
                                (true, false) => " and washer",
                                (false, false) => "",
                            }
                        ));
                    }
                }
                ResolvedPreset::Insert { sheet_range_mm, .. }
                    if t + 1e-9 < sheet_range_mm[0] || t > sheet_range_mm[1] + 1e-9 =>
                {
                    findings.push(format!(
                        "The {t} mm sheet is outside the insert's {}–{} mm range from its source",
                        sheet_range_mm[0], sheet_range_mm[1]
                    ));
                }
                _ => {}
            }
            if let Some(sheet_head) = self.snapshot().bodies.get(&sheet.output.body)
                && let Some(sheet_solid) = self.solid(*sheet_head)
                && let Some(common) = solid
                    .boolean(
                        sheet_solid,
                        BooleanOperation::Intersect,
                        GeometryPolicy {
                            boolean_fuzzy_mm: 0.,
                            ..self.policy
                        },
                    )
                    .map_err(|e| error(Some(id), e))?
                && common.volume_mm3().map_err(|e| error(Some(id), e))? > 1e-6
            {
                findings
                    .push("The hardware intersects the sheet; check its side and points".into());
            }
            let EntityData::Part { .. } = &document
                .get(part)
                .map(|e| &e.data)
                .ok_or_else(|| error(Some(id), "Hardware Part is missing"))?
            else {
                continue;
            };
            out.push(HardwareRecord {
                part,
                feature: id,
                sheet_part: sheet.output.body,
                sheet_feature: h.sheet,
                pattern: h.pattern as usize,
                item: h.item,
                designation,
                source,
                quantity,
                hole_mm: pattern.preset.hole_mm(),
                findings,
            });
        }
        Ok(out)
    }
}
