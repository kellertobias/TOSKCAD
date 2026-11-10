//! Weldment members: the profile section swept along the member axis
//! as an exact prism (hollow sections keep their wall), and member cut
//! records measured from each member Part's final solid in its member frame.
use crate::{GeometryBackend, Model, ModelError, error};
use multicad_core::{
    document::{Document, DocumentError, EntityData, EntityId},
    materials::{self, ProfileShape},
    model::{
        self, ConnectionKind, EndCut, EndTreatment, MemberDefinition, MemberEnd, MemberFrame,
        MemberPath, PathSegment, ResolvedConnection, ResolvedMember, SectionEdge, SectionLoop,
    },
    parameters::EvaluatedParameters,
    references::Origin,
    sketch::{Frame, Geometry},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
};
use multicad_solver::SketchEvaluationContext;
use std::collections::BTreeMap;

const TOLERANCE_MM: f64 = 1e-3;

/// Solved sketch-line paths, evaluated once per sketch per compile.
pub(crate) struct Paths<'a> {
    context: &'a SketchEvaluationContext<'a>,
    sketches: BTreeMap<EntityId, (Frame, Vec<Geometry>)>,
}
impl<'a> Paths<'a> {
    pub(crate) fn new(context: &'a SketchEvaluationContext<'a>) -> Self {
        Self {
            context,
            sketches: BTreeMap::new(),
        }
    }
    /// The solved world segment of a member path.
    pub(crate) fn segment(
        &mut self,
        member: &MemberDefinition,
        owner: EntityId,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PathSegment, ModelError> {
        let (sketch, line) = match &member.path {
            MemberPath::SketchLine { sketch, line } => (*sketch, *line),
            path => {
                return model::point_path(path, self.context.parameters())
                    .map_err(|e| error(Some(owner), e))?
                    .ok_or_else(|| error(Some(owner), "Member path is not a point pair"));
            }
        };
        if let std::collections::btree_map::Entry::Vacant(entry) = self.sketches.entry(sketch) {
            let evaluated = self
                .context
                .evaluate(sketch, None, interrupt)
                .map_err(|e| error(Some(owner), format!("Member path sketch: {e}")))?;
            let geometry = evaluated
                .geometry()
                .map_err(|e| error(Some(owner), format!("Member path sketch: {e}")))?;
            entry.insert((evaluated.frame, geometry));
        }
        let (frame, geometry) = &self.sketches[&sketch];
        let found = geometry.iter().find_map(|g| match g {
            Geometry::Line { id, a, b } if *id == line => Some((*a, *b)),
            _ => None,
        });
        let (a, b) = found.ok_or_else(|| {
            error(
                Some(owner),
                "The member's sketch line no longer exists or is not a line; choose another path",
            )
        })?;
        Ok(PathSegment {
            start: frame.point(a),
            end: frame.point(b),
            reference: Some(frame.n),
        })
    }
}

fn edges(section_loop: &SectionLoop) -> Vec<CurvedEdge2d> {
    match section_loop {
        SectionLoop::Polygon(points) => (0..points.len())
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % points.len()],
            })
            .collect(),
        SectionLoop::Curved(edges) => edges
            .iter()
            .map(|edge| match *edge {
                SectionEdge::Line { a, b } => CurvedEdge2d::Line { a, b },
                SectionEdge::Arc { center, a, b } => CurvedEdge2d::Arc {
                    center,
                    a,
                    b,
                    clockwise: false,
                },
            })
            .collect(),
        SectionLoop::Circle { center, radius_mm } => vec![CurvedEdge2d::Circle {
            center: *center,
            radius_mm: *radius_mm,
        }],
    }
}
/// The member section placed at axial distance `s` from the path start.
/// With `envelope`, the solid outside grown by that clearance, without the
/// bore; otherwise the real section with its holes.
pub(crate) fn section_profile(
    member: &ResolvedMember,
    s: f64,
    envelope: Option<f64>,
) -> CurvedProfile {
    let holes = match envelope {
        None => member.section.holes().iter().map(edges).collect(),
        Some(_) => vec![],
    };
    CurvedProfile {
        frame: PlaneFrame {
            origin: member.section_origin(s),
            u: member.frame.x,
            v: member.frame.y,
        },
        outer: edges(&member.section.outer(envelope.unwrap_or(0.))),
        holes,
    }
}
fn origins(
    owner: EntityId,
    member: &ResolvedMember,
    profile: &CurvedProfile,
    prefix: &str,
) -> Vec<Origin> {
    let outer = (0..profile.outer.len()).map(|i| {
        member.section.slotted_outer_role(i).map_or_else(
            || format!("{prefix}.outer.{i}"),
            |role| format!("{prefix}.{role}"),
        )
    });
    let inner = profile
        .holes
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, _)| format!("{prefix}.inner.{i}"));
    outer
        .chain(inner)
        .map(|role| Origin {
            source: owner,
            role,
        })
        .collect()
}
/// An exact prism of the member section from axial distance `from` over
/// `length` along the axis.
pub(crate) fn prism(
    member: &ResolvedMember,
    from: f64,
    length: f64,
    envelope: Option<f64>,
    owner: EntityId,
    prefix: &str,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let profile = section_profile(member, from, envelope);
    Solid::extrude_curved_profile_vector_named(
        &profile,
        member.frame.axis.map(|a| a * length),
        owner,
        &origins(owner, member, &profile, prefix),
        policy,
    )
    .map_err(|e| error(Some(owner), e))
}

/// Resolve a member feature with its solved path.
pub(crate) fn resolve(
    document: &Document,
    parameters: &EvaluatedParameters,
    feature: EntityId,
    paths: &mut Paths,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ResolvedMember, ModelError> {
    let e = |e: DocumentError| error(Some(feature), e);
    let source = model::read_member(document, feature).map_err(e)?;
    let segment = paths.segment(&source, feature, interrupt)?;
    model::resolve_member(document, parameters, &source, segment).map_err(e)
}

/// One connection treating an end of a member.
#[derive(Clone, Debug)]
pub(crate) struct Joint {
    pub connection: EntityId,
    pub partner: EntityId,
    pub partner_member: ResolvedMember,
    pub resolved: ResolvedConnection,
    pub treatment: EndTreatment,
}
/// Every connection that cuts an end of the member Part `part`. Two
/// connections cutting the same end are refused.
pub(crate) fn joints(
    document: &Document,
    parameters: &EvaluatedParameters,
    part: EntityId,
    member: &ResolvedMember,
    other: &mut dyn FnMut(EntityId) -> Result<ResolvedMember, ModelError>,
) -> Result<Vec<Joint>, ModelError> {
    let mut out: Vec<Joint> = vec![];
    for (id, connection) in
        model::member_connections(document, part).map_err(|e| error(Some(part), e))?
    {
        let e = |e: DocumentError| error(Some(id), e);
        let index = usize::from(connection.members[1] == part);
        let partner = connection.members[1 - index];
        let feature = model::member_feature(document, partner).ok_or_else(|| {
            error(
                Some(id),
                "The connected Part is no longer a weldment member",
            )
        })?;
        let partner_member = other(feature)?;
        if member.section.is_item_line_8_40x40() || partner_member.section.is_item_line_8_40x40() {
            return Err(error(
                Some(id),
                "Slotted extrusion members use Mechanical profile connectors; welded end cuts are unavailable",
            ));
        }
        let gap = connection.gap_mm(parameters).map_err(e)?;
        let (a, b) = if index == 0 {
            (member, &partner_member)
        } else {
            (&partner_member, member)
        };
        let resolved = model::resolve_connection(connection.kind, a, b, gap).map_err(e)?;
        let Some(treatment) = resolved.treatments[index] else {
            continue;
        };
        if let Some(previous) = out.iter().find(|j| j.treatment.end == treatment.end) {
            let name = |id: EntityId| document.get(id).map_or("".into(), |e| e.name.clone());
            return Err(error(
                Some(id),
                format!(
                    "{} and {} both cut the {:?} end of {}; remove one connection",
                    name(previous.connection),
                    name(id),
                    treatment.end,
                    name(part)
                )
                .replace("Start end", "start")
                .replace("End end", "end"),
            ));
        }
        out.push(Joint {
            connection: id,
            partner,
            partner_member,
            resolved,
            treatment,
        });
    }
    Ok(out)
}
fn diagonal(m: &ResolvedMember) -> f64 {
    m.section.width_mm.hypot(m.section.height_mm)
}
fn plane_frame(point: [f64; 3], normal: [f64; 3]) -> PlaneFrame {
    let seed = if normal[0].abs() < 0.9 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let d = dot(seed, normal);
    let u = [
        seed[0] - normal[0] * d,
        seed[1] - normal[1] * d,
        seed[2] - normal[2] * d,
    ];
    let len = dot(u, u).sqrt();
    let u = u.map(|c| c / len);
    let v = [
        normal[1] * u[2] - normal[2] * u[1],
        normal[2] * u[0] - normal[0] * u[2],
        normal[0] * u[1] - normal[1] * u[0],
    ];
    PlaneFrame {
        origin: point,
        u,
        v,
    }
}

/// Remove the half-space behind a plane: material on the side `normal`
/// points to is kept.
fn cut_plane(
    solid: Solid,
    point: [f64; 3],
    normal: [f64; 3],
    size: f64,
    owner: EntityId,
    role: &str,
    policy: GeometryPolicy,
) -> Result<Option<Solid>, ModelError> {
    let corners = [[-size, -size], [size, -size], [size, size], [-size, size]];
    let profile = CurvedProfile {
        frame: plane_frame(point, normal),
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: corners[i],
                b: corners[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    };
    let origins: Vec<_> = (0..4)
        .map(|i| Origin {
            source: owner,
            role: format!("{role}.side.{i}"),
        })
        .collect();
    let tool = Solid::extrude_curved_profile_vector_named(
        &profile,
        normal.map(|n| -n * size),
        owner,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(owner), e))?;
    solid
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .map_err(|e| error(Some(owner), e))
}

fn rust_member(
    member: &ResolvedMember,
    joints: &[Joint],
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let from = member.start_offset_mm();
    let mut stations = [from, from + member.length_mm()];
    let mut miters = [None; 2];
    for joint in joints {
        let EndCut::Plane { point, normal } = joint.treatment.cut else {
            return Err(error(
                Some(joint.connection),
                "Rust weldment members do not yet support coped ends",
            ));
        };
        let end = usize::from(joint.treatment.end == MemberEnd::End);
        if joint.treatment.kind == ConnectionKind::Miter {
            miters[end] = Some((joint.connection, plane_frame(point, normal)));
        } else {
            stations[end] = member.axial(point);
        }
    }
    let profile = section_profile(member, stations[0], None);
    let origins = origins(id, member, &profile, "member");
    let straight = std::iter::once(&profile.outer)
        .chain(&profile.holes)
        .flatten()
        .all(|edge| matches!(edge, CurvedEdge2d::Line { .. }));
    let Some((connection, _)) = miters.iter().flatten().next().copied() else {
        let height = stations[1] - stations[0];
        if height <= 0. {
            return Err(error(
                Some(joints.first().map_or(id, |joint| joint.connection)),
                "Connection cuts leave no member between the end planes",
            ));
        }
        if straight {
            return Solid::rust_extrude_linear_named(
                &profile,
                member.frame.axis.map(|component| component * height),
                0.,
                id,
                &origins,
                policy,
            )
            .map_err(|cause| error(Some(id), cause));
        }
        return crate::rust_backend::stock_profile(id, &profile, &origins, height, policy);
    };
    if !straight {
        return Err(error(
            Some(connection),
            "Rust weldment miters require a straight-sided section",
        ));
    }
    let planes = std::array::from_fn(|end| {
        miters[end].map_or_else(
            || PlaneFrame {
                origin: member.section_origin(stations[end]),
                ..profile.frame
            },
            |(_, plane)| plane,
        )
    });
    Solid::rust_extrude_between_planes_named(
        &profile,
        member.frame.axis,
        planes,
        id,
        &origins,
        policy,
    )
    .map_err(|cause| error(Some(connection), cause))
}

/// Build a member solid with its connection end cuts; returns it with its
/// resolved member.
pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    backend: GeometryBackend,
    paths: &mut Paths,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(Solid, ResolvedMember), ModelError> {
    let member = resolve(document, parameters, id, paths, interrupt)?;
    let source = model::read_member(document, id).map_err(|e| error(Some(id), e))?;
    let joints = joints(
        document,
        parameters,
        source.output.body,
        &member,
        &mut |feature| resolve(document, parameters, feature, paths, interrupt),
    )?;
    if backend == GeometryBackend::Rust {
        return Ok((rust_member(&member, &joints, id, policy)?, member));
    }
    // Connected ends start long enough to be cut back to their joint.
    let mut from = member.start_offset_mm();
    let mut to = from + member.length_mm();
    let mut size = member.length_mm() + diagonal(&member);
    for joint in &joints {
        let sin = (joint.resolved.angle_deg.to_radians().sin()).max(0.25);
        let margin = 2. * (diagonal(&member) + diagonal(&joint.partner_member)) / sin
            + joint.resolved.gap_mm
            + 1.;
        match joint.treatment.end {
            model::MemberEnd::Start => from = from.min(-margin),
            model::MemberEnd::End => to = to.max(member.path_length_mm + margin),
        }
        size = size.max(joint.partner_member.length_mm() + 4. * margin);
    }
    let mut solid = prism(&member, from, to - from, None, id, "member", policy)?;
    let size = 2. * size + 100.;
    for joint in &joints {
        let role = format!("connection.{}", joint.connection);
        let removed = || {
            error(
                Some(joint.connection),
                "The connection cut removes the whole member",
            )
        };
        solid = match joint.treatment.cut {
            EndCut::Plane { point, normal } => {
                cut_plane(solid, point, normal, size, id, &role, policy)?.ok_or_else(removed)?
            }
            EndCut::Cope {
                grow_mm,
                point,
                normal,
            } => {
                let partner = &joint.partner_member;
                let envelope = prism(
                    partner,
                    partner.start_offset_mm(),
                    partner.length_mm(),
                    Some(grow_mm),
                    id,
                    &format!("{role}.cope"),
                    policy,
                )?;
                let coped = solid
                    .boolean(&envelope, BooleanOperation::Subtract, policy)
                    .map_err(|e| error(Some(joint.connection), e))?
                    .ok_or_else(removed)?;
                cut_plane(coped, point, normal, size, id, &role, policy)?.ok_or_else(removed)?
            }
        };
    }
    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
        return Err(error(
            Some(id),
            "Connection cuts split the member into separate pieces; check the joint geometry",
        ));
    }
    Ok((solid, member))
}

/// The world-to-member transform: x, y and the axis become X, Y and Z, with
/// the path start at the origin.
pub(crate) fn into_member(frame: &MemberFrame, start: [f64; 3]) -> RigidTransform {
    let d = |a: [f64; 3]| -(a[0] * start[0] + a[1] * start[1] + a[2] * start[2]);
    RigidTransform {
        rotation: [frame.x, frame.y, frame.axis],
        translation_mm: [d(frame.x), d(frame.y), d(frame.axis)],
    }
}

/// How one end of a member is finished.
#[derive(Clone, Debug, PartialEq)]
pub struct EndRecord {
    /// None for a square end without a connection.
    pub kind: Option<model::ConnectionKind>,
    pub connection: Option<EntityId>,
    /// The member Part at the other side of the connection.
    pub partner: Option<EntityId>,
    /// Plane cuts: the cut face angle from square (0 = square, 45 = a
    /// right-angle miter). Copes: the branch angle to the receiving axis.
    pub angle_deg: f64,
    /// False for a continuing member cut square to cover its partner.
    pub terminates: bool,
    /// Plane cuts more than 0.01° from square: where the cut end reaches
    /// farthest along the axis. None for square ends and copes.
    pub long_point: Option<LongPoint>,
}
impl EndRecord {
    fn square() -> Self {
        Self {
            kind: None,
            connection: None,
            partner: None,
            angle_deg: 0.,
            terminates: true,
            long_point: None,
        }
    }
}
/// The long point of an angled plane cut: the side of the section where the
/// cut end reaches farthest along the member axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongPoint {
    /// World unit direction from the axis toward the long point, square to
    /// the axis.
    pub direction: [f64; 3],
    /// The section face it lies on, in section coordinates (width x, height
    /// y): "+x", "-x", "+y" or "-y", or a corner such as "+x-y" when the cut
    /// runs diagonally across the section.
    pub face: &'static str,
}
impl LongPoint {
    /// The long point of a plane cut that keeps the side `normal` points to.
    /// For a section offset q the cut face lies at s(q) = -(n_q · q) / n_a
    /// along the axis, so at either end the material reaches farthest on the
    /// side of the normal's component square to the axis.
    pub(crate) fn of_plane(frame: &MemberFrame, normal: [f64; 3]) -> Option<Self> {
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let along = dot(normal, frame.axis);
        let q = [0, 1, 2].map(|i| normal[i] - along * frame.axis[i]);
        let length = dot(q, q).sqrt();
        if length < 0.01_f64.to_radians().sin() {
            return None;
        }
        let direction = q.map(|c| c / length);
        let (x, y) = (dot(direction, frame.x), dot(direction, frame.y));
        let face = if (x.abs() - y.abs()).abs() < 1e-6 {
            match (x > 0., y > 0.) {
                (true, true) => "+x+y",
                (true, false) => "+x-y",
                (false, true) => "-x+y",
                (false, false) => "-x-y",
            }
        } else if x.abs() > y.abs() {
            if x > 0. { "+x" } else { "-x" }
        } else if y > 0. {
            "+y"
        } else {
            "-y"
        };
        Some(Self { direction, face })
    }
}

/// One member's cut record, measured from its current Part solid.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberRecord {
    pub part: EntityId,
    pub feature: EntityId,
    pub name: String,
    pub material: Option<EntityId>,
    pub stock: EntityId,
    pub stock_name: String,
    pub designation: String,
    pub shape: ProfileShape,
    /// Section bounding box: width (x) and height (y).
    pub section_mm: [f64; 2],
    pub wall_mm: Option<f64>,
    /// Sketch line or point pair the member follows.
    pub path: MemberPath,
    /// Path length plus both end extensions.
    pub nominal_length_mm: f64,
    /// Overall length along the axis, measured from the solid: the cut
    /// length from the farthest point of one end to the farthest point of
    /// the other.
    pub length_mm: f64,
    /// Measured axial range of the solid from the path start.
    pub span_mm: [f64; 2],
    /// The start and end treatments.
    pub ends: [EndRecord; 2],
    /// Measured section size in the member frame.
    pub measured_section_mm: [f64; 2],
    /// Section rotation about the axis, and the anchor on the path.
    pub rotation_deg: f64,
    pub anchor: [model::AxisAnchor; 2],
    /// World directions of the member axis and the section width and height.
    pub frame: MemberFrame,
    pub path_start: [f64; 3],
    pub volume_mm3: f64,
    /// Whether the overall length fits one stock bar.
    pub fits_stock: bool,
    /// Review findings: section or stock assignment that disagree.
    pub issues: Vec<String>,
}

impl Model {
    /// Cut records for every evaluated member Part, in document order. The
    /// length convention is the overall length along the member axis.
    pub fn member_cut_list(&self, document: &Document) -> Result<Vec<MemberRecord>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut out = vec![];
        for entity in model::members(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let source = model::read_member(document, id).map_err(e)?;
            let part = source.output.body;
            let Some(segment) = self.snapshot().member_paths.get(&id) else {
                continue;
            };
            let Some(solid) = self
                .snapshot()
                .bodies
                .get(&part)
                .and_then(|head| self.solid(*head))
            else {
                continue;
            };
            let member =
                model::resolve_member(document, &parameters, &source, *segment).map_err(e)?;
            let b = solid
                .transformed(into_member(&member.frame, member.path.start))
                .map_err(|e| error(Some(id), e))?
                .bounds()
                .map_err(|e| error(Some(id), e))?;
            let length = b.max[2] - b.min[2];
            let snapshot = self.snapshot();
            let joints = joints(document, &parameters, part, &member, &mut |feature| {
                let e = |e: DocumentError| error(Some(feature), e);
                let other = model::read_member(document, feature).map_err(e)?;
                let segment = snapshot
                    .member_paths
                    .get(&feature)
                    .ok_or_else(|| error(Some(feature), "Connected member was not evaluated"))?;
                model::resolve_member(document, &parameters, &other, *segment).map_err(e)
            })?;
            let mut ends = [EndRecord::square(), EndRecord::square()];
            for j in &joints {
                ends[usize::from(j.treatment.end == model::MemberEnd::End)] = EndRecord {
                    kind: Some(j.treatment.kind),
                    connection: Some(j.connection),
                    partner: Some(j.partner),
                    angle_deg: j.treatment.angle_deg,
                    terminates: j.treatment.terminates,
                    long_point: match j.treatment.cut {
                        EndCut::Plane { normal, .. } if j.treatment.angle_deg > 0.01 => {
                            LongPoint::of_plane(&member.frame, normal)
                        }
                        _ => None,
                    },
                };
            }
            let measured = [b.max[0] - b.min[0], b.max[1] - b.min[1]];
            let section = &member.section;
            let mut issues = vec![];
            if (measured[0] - section.width_mm).abs() > TOLERANCE_MM
                || (measured[1] - section.height_mm).abs() > TOLERANCE_MM
            {
                issues.push(format!(
                    "Measured section {:.3} × {:.3} mm differs from the {} × {} mm stock",
                    measured[0], measured[1], section.width_mm, section.height_mm
                ));
            }
            if let Some((_, properties)) = materials::part_properties(document, part).map_err(e)?
                && properties.stock.as_ref().map(|s| s.id) != Some(member.stock)
            {
                issues.push("The Part's stock assignment differs from its member stock".into());
            }
            let Some(EntityData::Part { material, .. }) = document.get(part).map(|e| &e.data)
            else {
                continue;
            };
            out.push(MemberRecord {
                part,
                feature: id,
                name: document
                    .get(part)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                material: *material,
                stock: member.stock,
                stock_name: document
                    .get(member.stock)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                designation: section.designation.clone(),
                shape: section.shape,
                section_mm: [section.width_mm, section.height_mm],
                wall_mm: section.wall_mm,
                path: source.path.clone(),
                nominal_length_mm: member.length_mm(),
                length_mm: length,
                span_mm: [b.min[2], b.max[2]],
                ends,
                measured_section_mm: measured,
                rotation_deg: member.rotation_rad.to_degrees(),
                anchor: source.anchor,
                frame: member.frame,
                path_start: member.path.start,
                volume_mm3: solid.volume_mm3().map_err(|e| error(Some(id), e))?,
                fits_stock: length <= section.stock_length_mm + TOLERANCE_MM,
                issues,
            });
        }
        Ok(out)
    }
}
