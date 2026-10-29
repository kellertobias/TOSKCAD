//! Native weldment members: the profile section swept along the member axis
//! as an exact prism (hollow sections keep their wall), and member cut
//! records measured from each member Part's final solid in its member frame.
use crate::{Model, ModelError, error};
use multicad_core::{
    document::{Document, DocumentError, EntityData, EntityId},
    materials::{self, ProfileShape},
    model::{
        self, MemberDefinition, MemberFrame, MemberPath, PathSegment, ResolvedMember, SectionLoop,
    },
    parameters::EvaluatedParameters,
    references::Origin,
    sketch::{Frame, Geometry},
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
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
        SectionLoop::Circle { center, radius_mm } => vec![CurvedEdge2d::Circle {
            center: *center,
            radius_mm: *radius_mm,
        }],
    }
}
/// The member section placed at axial distance `s` from the path start.
/// `grow_mm` enlarges the outside for clearance envelopes, which never have
/// holes.
pub(crate) fn section_profile(member: &ResolvedMember, s: f64, grow_mm: f64) -> CurvedProfile {
    let holes = if grow_mm == 0. {
        member.section.holes().iter().map(edges).collect()
    } else {
        vec![]
    };
    CurvedProfile {
        frame: PlaneFrame {
            origin: member.section_origin(s),
            u: member.frame.x,
            v: member.frame.y,
        },
        outer: edges(&member.section.outer(grow_mm)),
        holes,
    }
}
fn origins(owner: EntityId, profile: &CurvedProfile, prefix: &str) -> Vec<Origin> {
    let outer = (0..profile.outer.len()).map(|i| format!("{prefix}.outer.{i}"));
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
    grow_mm: f64,
    owner: EntityId,
    prefix: &str,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let profile = section_profile(member, from, grow_mm);
    Solid::extrude_curved_profile_vector_named(
        &profile,
        member.frame.axis.map(|a| a * length),
        owner,
        &origins(owner, &profile, prefix),
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

/// Build a member solid; returns it with its resolved member.
pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    paths: &mut Paths,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(Solid, ResolvedMember), ModelError> {
    let member = resolve(document, parameters, id, paths, interrupt)?;
    let solid = prism(
        &member,
        member.start_offset_mm(),
        member.length_mm(),
        0.,
        id,
        "member",
        policy,
    )?;
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
