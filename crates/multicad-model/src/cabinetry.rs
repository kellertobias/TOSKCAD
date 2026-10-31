//! Native cabinetry: panel solids from stock, and cut records and stack
//! checks measured from the evaluated solids. A panel's final Part solid is
//! measured in its panel frame, so later features that change the Part are
//! reflected in its finished size.
use crate::{Model, ModelError, error};
use multicad_core::{
    document::{Document, DocumentError, EntityData, EntityId},
    materials,
    model::{self, CutDimensions, GrainAxis, PanelRole, PanelShape, PanelSource, ResolvedBand},
    parameters::EvaluatedParameters,
    references::Origin,
    sketch_regions::RegionIntent,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
};
use std::collections::BTreeMap;

/// Face roles of a rectangular panel's edge faces, in profile edge order.
pub const PANEL_EDGE_ROLES: [&str; 4] = [
    "panel.v_start",
    "panel.u_end",
    "panel.v_end",
    "panel.u_start",
];
const TOLERANCE_MM: f64 = 1e-3;

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Build a panel solid and return it with its panel frame. `profile` supplies
/// solved sketch regions for profile panels.
pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    profile: impl FnOnce(EntityId, &RegionIntent) -> Result<(CurvedProfile, Vec<Origin>), ModelError>,
) -> Result<(Solid, PlaneFrame), ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let source = model::read_panel(document, id).map_err(e)?;
    let panel = model::resolve_panel(document, parameters, &source).map_err(e)?;
    let (profile, origins, thickness) = match &panel.shape {
        PanelShape::Box(b) => {
            let (l, w) = (b.length_mm, b.width_mm);
            let corners = [[0., 0.], [l, 0.], [l, w], [0., w]];
            let profile = CurvedProfile {
                frame: PlaneFrame {
                    origin: b.origin,
                    u: b.u,
                    v: b.v,
                },
                outer: (0..4)
                    .map(|i| CurvedEdge2d::Line {
                        a: corners[i],
                        b: corners[(i + 1) % 4],
                    })
                    .collect(),
                holes: vec![],
            };
            let origins = PANEL_EDGE_ROLES
                .iter()
                .map(|role| Origin {
                    source: id,
                    role: (*role).into(),
                })
                .collect();
            (profile, origins, b.thickness_mm)
        }
        PanelShape::Profile {
            profile: source,
            thickness_mm,
        } => {
            let (profile, origins) = profile(source.sketch, &source.region)?;
            (profile, origins, *thickness_mm)
        }
    };
    if thickness <= 0. {
        return Err(error(Some(id), "Panel stock has no thickness"));
    }
    let n = cross(profile.frame.u, profile.frame.v);
    let solid = Solid::extrude_curved_profile_vector_named(
        &profile,
        n.map(|c| c * thickness),
        id,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(id), e))?;
    Ok((solid, profile.frame))
}

/// Subtract cut prisms from the solid before a cut feature. A cut with no
/// prisms still returns its own copy of the input.
fn subtract(
    base: &Solid,
    prisms: &[model::CutPrism],
    id: EntityId,
    role: &str,
    policy: GeometryPolicy,
    what: &str,
) -> Result<Solid, ModelError> {
    let batch_circles = prisms.len() > 1 && prisms.iter().all(|p| p.circle.is_some());
    let mut tools = Vec::with_capacity(prisms.len());
    let mut current: Option<Solid> = None;
    for (k, prism) in prisms.iter().enumerate() {
        let outer: Vec<_> = match (prism.circle, prism.slot) {
            (Some((center, radius_mm)), _) => vec![CurvedEdge2d::Circle { center, radius_mm }],
            (None, Some((a, b, r))) => slot_edges(a, b, r),
            (None, None) => {
                let n = prism.polygon.len();
                (0..n)
                    .map(|i| CurvedEdge2d::Line {
                        a: prism.polygon[i],
                        b: prism.polygon[(i + 1) % n],
                    })
                    .collect()
            }
        };
        let origins: Vec<_> = (0..outer.len())
            .map(|i| Origin {
                source: id,
                role: format!("{role}.{k}.{i}"),
            })
            .collect();
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: prism.origin,
                u: prism.u,
                v: prism.v,
            },
            outer,
            holes: vec![],
        };
        let tool =
            Solid::extrude_curved_profile_vector_named(&profile, prism.sweep, id, &origins, policy)
                .map_err(|e| error(Some(id), e))?;
        tools.push(tool);
    }
    // Circular drilling is bounded by the shared hole budget. Combine at
    // most 128 exact tool solids per native Boolean, preserving each tool's
    // named faces. The profile and removed-volume contracts are unchanged.
    for chunk in tools.chunks(if batch_circles { 128 } else { 1 }) {
        let compound;
        let tool = if chunk.len() == 1 {
            &chunk[0]
        } else {
            compound = Solid::compound_named(&chunk.iter().collect::<Vec<_>>(), id, policy)
                .map_err(|e| error(Some(id), e))?;
            &compound
        };
        let source = current.as_ref().unwrap_or(base);
        current = source
            .boolean(tool, BooleanOperation::Subtract, policy)
            .map_err(|e| error(Some(id), e))?
            .map(Some)
            .ok_or_else(|| error(Some(id), format!("The {what} removed the whole panel")))?;
    }
    let solid = match current {
        Some(solid) => solid,
        None => base
            .transformed(RigidTransform::default())
            .map_err(|e| error(Some(id), e))?,
    };
    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
        return Err(error(
            Some(id),
            format!("The {what} splits the panel into pieces"),
        ));
    }
    Ok(solid)
}

/// A rounded slot, counterclockwise: the end-circle centers `a` and `b`
/// with radius `r`. Coincident centers give a circle.
fn slot_edges(a: [f64; 2], b: [f64; 2], r: f64) -> Vec<CurvedEdge2d> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len <= 1e-9 {
        return vec![CurvedEdge2d::Circle {
            center: a,
            radius_mm: r,
        }];
    }
    let n = [-d[1] / len * r, d[0] / len * r];
    let p = |c: [f64; 2], s: f64| [c[0] + n[0] * s, c[1] + n[1] * s];
    vec![
        CurvedEdge2d::Line {
            a: p(a, -1.),
            b: p(b, -1.),
        },
        CurvedEdge2d::Arc {
            center: b,
            a: p(b, -1.),
            b: p(b, 1.),
            clockwise: false,
        },
        CurvedEdge2d::Line {
            a: p(b, 1.),
            b: p(a, 1.),
        },
        CurvedEdge2d::Arc {
            center: a,
            a: p(a, 1.),
            b: p(a, -1.),
            clockwise: false,
        },
    ]
}
/// The tenons of one loose-tenon joint pair as one solid.
pub(crate) fn build_loose_tenon(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let tenons = model::resolve_loose_tenon_feature(document, parameters, id)
        .map_err(|e| error(Some(id), e))?;
    let mut out: Option<Solid> = None;
    for (k, t) in tenons.iter().enumerate() {
        let v = cross(t.axis, t.long);
        let r = t.thickness_mm / 2.;
        let half = (t.width_mm / 2. - r).max(0.);
        let start = [0, 1, 2].map(|i| t.center[i] - t.axis[i] * t.into_face_mm);
        let edges = slot_edges([-half, 0.], [half, 0.], r);
        let origins: Vec<_> = (0..edges.len())
            .map(|i| Origin {
                source: id,
                role: format!("tenon.{k}.{i}"),
            })
            .collect();
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: start,
                u: t.long,
                v,
            },
            outer: edges,
            holes: vec![],
        };
        let length = t.into_face_mm + t.into_edge_mm;
        let solid = Solid::extrude_curved_profile_vector_named(
            &profile,
            t.axis.map(|c| c * length),
            id,
            &origins,
            policy,
        )
        .map_err(|e| error(Some(id), e))?;
        out = Some(match out {
            None => solid,
            Some(prev) => prev
                .boolean(&solid, BooleanOperation::Union, policy)
                .map_err(|e| error(Some(id), e))?
                .ok_or_else(|| error(Some(id), "The tenons could not be combined"))?,
        });
    }
    out.ok_or_else(|| error(Some(id), "The joint has no tenons"))
}
/// Remove the other panel's share of each joint corner from a panel solid.
pub(crate) fn build_joint_cut(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    solids: &BTreeMap<EntityId, Solid>,
) -> Result<Solid, ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let cut = model::read_panel_joint_cut(document, id).map_err(e)?;
    let joint = model::read_panel_joint(document, cut.joint).map_err(e)?;
    let corner = model::resolve_panel_joint_pair(document, parameters, &joint, cut.pair as usize)
        .map_err(e)?;
    let base = solids
        .get(&cut.input.producer)
        .ok_or_else(|| error(Some(id), "The panel before this joint has no solid"))?;
    let prisms = match cut.side {
        model::PanelJointSide::A => &corner.cut_a,
        model::PanelJointSide::B => &corner.cut_b,
    };
    subtract(
        base,
        prisms,
        id,
        &format!("joint.{}", cut.joint),
        policy,
        "joint",
    )
}
/// Cut one machining operation into one target panel.
pub(crate) fn build_machining_cut(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    solids: &BTreeMap<EntityId, Solid>,
) -> Result<Solid, ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let cut = model::read_machining_cut(document, id).map_err(e)?;
    let op = model::read_machining(document, cut.operation).map_err(e)?;
    let resolved =
        model::resolve_machining(document, parameters, &op, cut.output.body).map_err(e)?;
    let base = solids
        .get(&cut.input.producer)
        .ok_or_else(|| error(Some(id), "The panel before this operation has no solid"))?;
    subtract(
        base,
        &resolved.prisms,
        id,
        &format!("machining.{}", cut.operation),
        policy,
        op.label(),
    )
}

/// The world-to-frame transform: frame u, v and u × v become X, Y and Z.
fn into_frame(frame: &PlaneFrame) -> RigidTransform {
    let n = cross(frame.u, frame.v);
    RigidTransform {
        rotation: [frame.u, frame.v, n],
        translation_mm: [
            -dot(frame.origin, frame.u),
            -dot(frame.origin, frame.v),
            -dot(frame.origin, n),
        ],
    }
}

/// One panel's cut-list record, measured from its current Part solid.
#[derive(Clone, Debug, PartialEq)]
pub struct CutRecord {
    pub part: EntityId,
    pub feature: EntityId,
    pub name: String,
    pub carcass: Option<EntityId>,
    pub role: Option<PanelRole>,
    pub stack: Option<EntityId>,
    /// Cabinetry joints this panel takes part in.
    pub joints: Vec<EntityId>,
    /// Machining operations cut into this panel.
    pub operations: Vec<EntityId>,
    pub material: Option<EntityId>,
    pub stock: EntityId,
    pub stock_name: String,
    pub grain: GrainAxis,
    pub cut: CutDimensions,
    /// Banded edges by name ("u_start", ...), with thickness and allowance.
    pub bands: Vec<(&'static str, ResolvedBand)>,
    pub oversize_mm: f64,
    pub volume_mm3: f64,
    pub fits_stock: bool,
    /// Review findings: thickness or stock assignment that disagree.
    pub issues: Vec<String>,
}
/// A stack's layers as evaluated, and whether consecutive layers touch face
/// to face.
#[derive(Clone, Debug, PartialEq)]
pub struct StackEvaluation {
    pub stack: EntityId,
    pub name: String,
    pub adhesive: Option<String>,
    pub layers: Vec<StackLayer>,
    pub total_thickness_mm: f64,
    pub face_to_face: bool,
    pub issues: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StackLayer {
    pub part: EntityId,
    pub material: Option<EntityId>,
    pub stock: EntityId,
    pub grain: GrainAxis,
    pub thickness_mm: f64,
    /// Range along the first layer's thickness direction.
    pub span_mm: [f64; 2],
}

/// One machining operation as cut into one panel, with the volume it
/// measurably removed.
#[derive(Clone, Debug, PartialEq)]
pub struct MachiningRecord {
    pub part: EntityId,
    pub feature: EntityId,
    pub operation: EntityId,
    pub name: String,
    pub kind: &'static str,
    pub face: model::MachiningFace,
    pub material: Option<EntityId>,
    pub stock: EntityId,
    pub holes: Vec<model::ResolvedHole>,
    pub recesses: Vec<model::ResolvedRecess>,
    pub removed_mm3: f64,
    /// The loose-tenon Part a joint mortise receives.
    pub tenon: Option<EntityId>,
}

impl Model {
    /// Every machining cut, measured from its native solids.
    pub fn machining(&self, document: &Document) -> Result<Vec<MachiningRecord>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut out = vec![];
        for entity in model::machining_cuts(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let cut = model::read_machining_cut(document, id).map_err(e)?;
            let (Some(before), Some(after)) = (self.solid(cut.input.producer), self.solid(id))
            else {
                continue;
            };
            let op = model::read_machining(document, cut.operation).map_err(e)?;
            let part = cut.output.body;
            let resolved = model::resolve_machining(document, &parameters, &op, part).map_err(e)?;
            let panel = model::resolve_panel(
                document,
                &parameters,
                &model::read_panel(
                    document,
                    model::panel_feature(document, part)
                        .ok_or_else(|| error(Some(id), "The machined Part is not a panel"))?,
                )
                .map_err(e)?,
            )
            .map_err(e)?;
            let volume = |s: &Solid| s.volume_mm3().map_err(|e| error(Some(id), e));
            out.push(MachiningRecord {
                part,
                feature: id,
                operation: cut.operation,
                name: document
                    .get(cut.operation)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                kind: op.label(),
                face: resolved.face,
                material: Some(panel.stock_spec.material),
                stock: panel.stock,
                holes: resolved.holes,
                recesses: resolved.recesses,
                removed_mm3: volume(before)? - volume(after)?,
                tenon: None,
            });
        }
        // Paired face-to-edge joinery: mortises or circular dowel holes.
        for entity in model::panel_joint_cuts(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let cut = model::read_panel_joint_cut(document, id).map_err(e)?;
            let joint = model::read_panel_joint(document, cut.joint).map_err(e)?;
            let pair = joint.pairs[cut.pair as usize];
            let dowels = matches!(joint.kind, model::PanelJointKind::Dowels(_));
            let (_, layout) = match &joint.kind {
                model::PanelJointKind::LooseTenon(spec) => {
                    model::resolve_loose_tenon(document, &parameters, spec, &pair)
                }
                model::PanelJointKind::Dowels(spec) => {
                    model::resolve_dowels(document, &parameters, spec, &pair)
                }
                _ => continue,
            }
            .map_err(e)?;
            let (Some(before), Some(after)) = (self.solid(cut.input.producer), self.solid(id))
            else {
                continue;
            };
            let face_side = cut.side == layout.face_side;
            let frame = if face_side {
                layout.face_frame
            } else {
                layout.edge_frame
            };
            let recesses = if face_side {
                &layout.face_mortises
            } else {
                &layout.edge_mortises
            };
            let holes = if dowels {
                layout
                    .tenons
                    .iter()
                    .zip(recesses)
                    .map(|(hole, recess)| model::ResolvedHole {
                        at: frame.local(hole.center).0,
                        center: hole.center,
                        axis: frame.inward,
                        diameter_mm: recess.size_mm[0],
                        depth_mm: recess.depth_mm,
                        through: false,
                    })
                    .collect()
            } else {
                vec![]
            };
            let part = cut.output.body;
            let panel = model::resolve_panel(
                document,
                &parameters,
                &model::read_panel(
                    document,
                    model::panel_feature(document, part)
                        .ok_or_else(|| error(Some(id), "The joined Part is not a panel"))?,
                )
                .map_err(e)?,
            )
            .map_err(e)?;
            let volume = |s: &Solid| s.volume_mm3().map_err(|e| error(Some(id), e));
            out.push(MachiningRecord {
                part,
                feature: id,
                operation: cut.joint,
                name: document
                    .get(cut.joint)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                kind: if dowels {
                    "dowel drilling"
                } else {
                    "loose-tenon mortise"
                },
                face: if face_side {
                    layout.face
                } else {
                    model::MachiningFace::Edge { edge: layout.edge }
                },
                material: Some(panel.stock_spec.material),
                stock: panel.stock,
                holes,
                recesses: if dowels {
                    vec![]
                } else if face_side {
                    layout.face_mortises
                } else {
                    layout.edge_mortises
                },
                removed_mm3: volume(before)? - volume(after)?,
                tenon: model::joint_tenons(document, cut.joint)
                    .into_iter()
                    .find(|(p, ..)| *p == cut.pair)
                    .map(|(.., body)| body),
            });
        }
        Ok(out)
    }
    /// The frame of an evaluated panel and its Part's final solid.
    fn panel_solid(&self, feature: EntityId, part: EntityId) -> Option<(PlaneFrame, &Solid)> {
        let frame = *self.snapshot().panel_frames.get(&feature)?;
        // A Part used as a component definition is not a final body itself
        // but is still a panel to cut; a Part consumed by a Boolean is not.
        if self.snapshot().consumed_bodies.contains(&part) {
            return None;
        }
        let head = self
            .snapshot()
            .bodies
            .get(&part)
            .or_else(|| self.snapshot().body_sources.get(&part))?;
        Some((frame, self.solid(*head)?))
    }
    /// Cut records for every evaluated panel Part, in document order. Parts
    /// consumed by a Boolean are not listed.
    pub fn cut_list(&self, document: &Document) -> Result<Vec<CutRecord>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut stacks = std::collections::BTreeMap::new();
        for s in model::stacks(document) {
            for layer in model::read_stack(document, s.id)
                .map_err(|e| error(Some(s.id), e))?
                .layers
            {
                stacks.insert(layer, s.id);
            }
        }
        let mut out = vec![];
        for entity in model::panels(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let source = model::read_panel(document, id).map_err(e)?;
            let part = source.output.body;
            let Some((frame, solid)) = self.panel_solid(id, part) else {
                continue;
            };
            let panel = model::resolve_panel(document, &parameters, &source).map_err(e)?;
            let local = solid
                .transformed(into_frame(&frame))
                .map_err(|e| error(Some(id), e))?;
            let b = local.bounds().map_err(|e| error(Some(id), e))?;
            let size = [0, 1, 2].map(|i| b.max[i] - b.min[i]);
            let cut = panel.cut(size[0], size[1], size[2]).map_err(e)?;
            let mut issues = vec![];
            let stock_thickness = model::stock_thickness(&panel.stock_spec);
            if (size[2] - stock_thickness).abs() > TOLERANCE_MM {
                issues.push(format!(
                    "Measured thickness {:.3} mm differs from the {stock_thickness} mm stock",
                    size[2]
                ));
            }
            if let Some((_, properties)) = materials::part_properties(document, part).map_err(e)?
                && properties.stock.as_ref().map(|s| s.id) != Some(panel.stock)
            {
                issues.push("The Part's stock assignment differs from its panel stock".into());
            }
            let (carcass, role) = match source.source {
                PanelSource::Carcass { carcass, role } => (Some(carcass), Some(role)),
                _ => (None, None),
            };
            let Some(EntityData::Part { material, .. }) = document.get(part).map(|e| &e.data)
            else {
                continue;
            };
            out.push(CutRecord {
                part,
                feature: id,
                name: document
                    .get(part)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                carcass,
                role,
                stack: stacks.get(&part).copied(),
                joints: model::panel_joints_of(document, part),
                operations: model::machinings_of(document, part),
                material: *material,
                stock: panel.stock,
                stock_name: document
                    .get(panel.stock)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                grain: panel.grain,
                cut,
                bands: ["u_start", "u_end", "v_start", "v_end"]
                    .into_iter()
                    .zip(panel.edges)
                    .filter_map(|(name, b)| b.map(|b| (name, b)))
                    .collect(),
                oversize_mm: panel.oversize_mm,
                volume_mm3: solid.volume_mm3().map_err(|e| error(Some(id), e))?,
                fits_stock: model::fits_stock(&panel.stock_spec, &cut),
                issues,
            });
        }
        Ok(out)
    }
    /// Evaluate every stack: layers must have parallel faces, touch in the
    /// listed order and overlap in plan.
    pub fn stacks(&self, document: &Document) -> Result<Vec<StackEvaluation>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut out = vec![];
        for entity in model::stacks(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let stack = model::read_stack(document, id).map_err(e)?;
            let mut issues = vec![];
            let mut layers = vec![];
            let mut reference: Option<PlaneFrame> = None;
            let mut footprints = vec![];
            for part in &stack.layers {
                let name = document
                    .get(*part)
                    .map_or("A layer".into(), |e| e.name.clone());
                let Some(feature) = model::panel_feature(document, *part) else {
                    issues.push(format!("{name} is no longer a panel"));
                    continue;
                };
                let source = model::read_panel(document, feature).map_err(e)?;
                let panel = model::resolve_panel(document, &parameters, &source).map_err(e)?;
                let Some((frame, solid)) = self.panel_solid(feature, *part) else {
                    issues.push(format!("{name} has no evaluated solid"));
                    continue;
                };
                let frame0 = *reference.get_or_insert(frame);
                let n0 = cross(frame0.u, frame0.v);
                if dot(n0, cross(frame.u, frame.v)).abs() < 1. - 1e-9 {
                    issues.push(format!("{name} is not parallel to the first layer"));
                }
                let b = solid
                    .transformed(into_frame(&frame0))
                    .map_err(|e| error(Some(id), e))?
                    .bounds()
                    .map_err(|e| error(Some(id), e))?;
                footprints.push(b);
                let Some(EntityData::Part { material, .. }) = document.get(*part).map(|e| &e.data)
                else {
                    continue;
                };
                layers.push(StackLayer {
                    part: *part,
                    material: *material,
                    stock: panel.stock,
                    grain: panel.grain,
                    thickness_mm: b.max[2] - b.min[2],
                    span_mm: [b.min[2], b.max[2]],
                });
            }
            let ascending = layers.len() < 2 || layers[1].span_mm[0] >= layers[0].span_mm[0];
            for (i, pair) in layers.windows(2).enumerate() {
                let (a, b) = (&pair[0], &pair[1]);
                let gap = if ascending {
                    b.span_mm[0] - a.span_mm[1]
                } else {
                    a.span_mm[0] - b.span_mm[1]
                };
                if gap.abs() > TOLERANCE_MM {
                    issues.push(format!(
                        "Layers {} and {} are {:.3} mm apart instead of face to face",
                        i + 1,
                        i + 2,
                        gap
                    ));
                }
                let (fa, fb) = (&footprints[i], &footprints[i + 1]);
                let overlap = (0..2)
                    .all(|k| fa.max[k].min(fb.max[k]) - fa.min[k].max(fb.min[k]) > TOLERANCE_MM);
                if !overlap {
                    issues.push(format!(
                        "Layers {} and {} do not overlap in plan",
                        i + 1,
                        i + 2
                    ));
                }
            }
            let total = match (layers.first(), layers.last()) {
                (Some(a), Some(b)) => {
                    a.span_mm[1].max(b.span_mm[1]) - a.span_mm[0].min(b.span_mm[0])
                }
                _ => 0.,
            };
            out.push(StackEvaluation {
                stack: id,
                name: entity.name.clone(),
                adhesive: stack.adhesive,
                face_to_face: issues.is_empty(),
                layers,
                total_thickness_mm: total,
                issues,
            });
        }
        Ok(out)
    }
}
