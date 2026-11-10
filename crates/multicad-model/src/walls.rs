//! Native wall evaluation: the planned outline is extruded to the wall
//! height and each opening is subtracted as an exact prism with stable face
//! roles. Downward drawing views draw walls as a plan cut: the solid is
//! sectioned at a plan cut height and hatched, and receive door swings,
//! window panes and dashed outlines of openings the cut does not pass.
use crate::{DrawingBody, GeometryBackend, Model, ModelError, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::{DrawingVector, VectorCurve, VectorRole, ViewKind},
    model::{self, Loop},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, DrawingProjection, DrawingSolid, GeometryError,
    GeometryPolicy, PlaneFrame, ProjectedVector, Solid,
};

/// Plan cut above a wall's base in a downward projection view; the same as
/// the floor-plan guide default. Guided plans and section views cut at
/// their own plane instead.
pub(crate) const DEFAULT_PLAN_CUT_MM: f64 = 1000.;
/// Section hatch of cut walls whose material has no hatch style: 45° lines
/// at this paper spacing and line width, at any view scale. A material hatch
/// style (including an explicit "none") replaces it.
pub(crate) const WALL_HATCH_ANGLE_DEG: f64 = 45.;
pub(crate) const WALL_HATCH_SPACING_MM: f64 = 1.5;
pub(crate) const WALL_HATCH_LINE_MM: f64 = 0.13;
/// Paper line widths of the plan hierarchy: cut walls are drawn by the
/// section role (0.5 mm); geometry seen below the cut, door leaves and
/// window panes are thinner; swings and overhead outlines are thinnest.
pub(crate) const PLAN_PROJECTED_MM: f64 = 0.18;
const PLAN_SYMBOL_MM: f64 = 0.18;
const PLAN_THIN_MM: f64 = 0.13;

/// A wall body in a downward view and where the plan cuts it, in world Z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WallCut {
    pub index: usize,
    pub feature: EntityId,
    pub base_z: f64,
    pub cut_z: f64,
}

fn view_normal(view: &multicad_core::drawings::DrawingView) -> [f64; 3] {
    let f = &view.frame;
    [
        f.u[1] * f.v[2] - f.u[2] * f.v[1],
        f.u[2] * f.v[0] - f.u[0] * f.v[2],
        f.u[0] * f.v[1] - f.u[1] * f.v[0],
    ]
}

/// The wall bodies of a view looking straight down, with their plan cut.
/// A projection cuts each wall [`DEFAULT_PLAN_CUT_MM`] above its base; a
/// section (including a guided floor plan) cuts at its own plane. Other
/// views, details and views looking up return nothing.
pub(crate) fn wall_cuts(
    document: &Document,
    view: &multicad_core::drawings::DrawingView,
    bodies: &[DrawingBody<'_>],
) -> Result<Vec<WallCut>, ModelError> {
    let n = view_normal(view);
    if matches!(view.kind, ViewKind::Detail { .. }) || n[2] < 1. - 1e-9 {
        return Ok(vec![]);
    }
    let walls: Vec<_> = bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| wall_feature(document, b.identity).map(|w| (i, w)))
        .collect();
    if walls.is_empty() {
        return Ok(vec![]);
    }
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
    walls
        .into_iter()
        .map(|(index, feature)| {
            let e = |e: multicad_core::document::DocumentError| error(Some(feature), e);
            let source = model::read_wall(document, feature).map_err(e)?;
            let wall = model::resolve_wall(document, &parameters, &source).map_err(e)?;
            let cut_z = match view.kind {
                ViewKind::Section { offset_mm, .. } => view.frame.origin_mm[2] + offset_mm,
                _ => wall.base_z_mm + DEFAULT_PLAN_CUT_MM,
            };
            Ok(WallCut {
                index,
                feature,
                base_z: wall.base_z_mm,
                cut_z,
            })
        })
        .collect()
}

/// Project a downward projection view with its walls drawn as plan cuts:
/// the other bodies keep the view's projection, and each group of walls
/// sharing a cut is sectioned there, looking down to its base. Returned
/// source indices refer to `solids`.
pub(crate) fn project_plan(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
    origin_z: f64,
    cuts: &[WallCut],
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut groups: std::collections::BTreeMap<(u64, u64), Vec<usize>> = Default::default();
    for c in cuts {
        groups
            .entry((c.cut_z.to_bits(), c.base_z.to_bits()))
            .or_default()
            .push(c.index);
    }
    let walls: std::collections::BTreeSet<usize> = cuts.iter().map(|c| c.index).collect();
    let mut out = vec![];
    let mut run = |indices: Vec<usize>, options: DrawingProjection| {
        if indices.is_empty() {
            return Ok::<(), GeometryError>(());
        }
        let subset: Vec<_> = indices
            .iter()
            .map(|&i| DrawingSolid {
                solid: solids[i].solid,
                hatch: solids[i].hatch,
            })
            .collect();
        for mut v in multicad_kernel::project_drawing(&subset, options)? {
            v.source_index = indices[v.source_index];
            out.push(v);
        }
        Ok(())
    };
    run(
        (0..solids.len()).filter(|i| !walls.contains(i)).collect(),
        options,
    )?;
    for ((cut, base), indices) in groups {
        let (cut, base) = (f64::from_bits(cut), f64::from_bits(base));
        run(
            indices,
            DrawingProjection {
                section_offset_mm: Some(cut - origin_z),
                section_only: false,
                section_depth_mm: Some(cut - base + 1.),
                section_u_mm: None,
                section_v_mm: None,
                ..options
            },
        )?;
    }
    Ok(out)
}

fn edges(l: &Loop) -> Vec<CurvedEdge2d> {
    let n = l.points.len();
    (0..n)
        .map(|i| CurvedEdge2d::Line {
            a: l.points[i],
            b: l.points[(i + 1) % n],
        })
        .collect()
}
fn origins(id: EntityId, roles: impl IntoIterator<Item = String>) -> Vec<Origin> {
    roles
        .into_iter()
        .map(|role| Origin { source: id, role })
        .collect()
}

fn ensure_disjoint_members(
    members: &[Solid],
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), ModelError> {
    // A compound intentionally retains overlaps. Prove disjoint material
    // volumes before constructing it; do not use a union to hide bad offsets.
    let bounds = members
        .iter()
        .map(|solid| solid.bounds().map_err(|e| error(Some(id), e)))
        .collect::<Result<Vec<_>, _>>()?;
    for i in 0..members.len() {
        for j in i + 1..members.len() {
            if interrupt.is_some_and(|f| f()) {
                return Err(error(Some(id), "Wall evaluation superseded"));
            }
            if (0..3).any(|axis| {
                bounds[i].max[axis] <= bounds[j].min[axis]
                    || bounds[j].max[axis] <= bounds[i].min[axis]
            }) {
                continue;
            }
            if members[i]
                .boolean(&members[j], BooleanOperation::Intersect, policy)
                .map_err(|e| error(Some(id), e))?
                .is_some()
            {
                return Err(error(
                    Some(id),
                    "Material bands overlap for this wall path; adjust the construction or repair the corner geometry",
                ));
            }
        }
    }
    Ok(())
}

fn extrude_region(
    wall: &model::ResolvedWall,
    plan: &model::WallPlan,
    id: EntityId,
    policy: GeometryPolicy,
    band: Option<&model::WallLayerBand>,
    backend: GeometryBackend,
) -> Result<Solid, ModelError> {
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., wall.base_z_mm],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        outer: edges(&plan.outer),
        holes: plan.holes.iter().map(edges).collect(),
    };
    let roles = plan
        .outer
        .roles
        .iter()
        .chain(plan.holes.iter().flat_map(|h| &h.roles))
        .map(|role| {
            if let Some(band) = band {
                let outside_left = band.outside_mm > band.inside_mm;
                if let Some(segment) = role.strip_prefix("wall.seg.") {
                    if let Some(segment) = segment.strip_suffix(".left") {
                        return format!(
                            "wall.layer.seg.{segment}.{}",
                            if outside_left { "outside" } else { "inside" }
                        );
                    }
                    if let Some(segment) = segment.strip_suffix(".right") {
                        return format!(
                            "wall.layer.seg.{segment}.{}",
                            if outside_left { "inside" } else { "outside" }
                        );
                    }
                }
            }
            role.clone()
        });
    let roles = origins(id, roles);
    if backend == GeometryBackend::Rust && has_diagonal_segments(plan) {
        return Solid::rust_extrude_linear_named(
            &profile,
            [0., 0., wall.height_mm],
            0.,
            id,
            &roles,
            policy,
        )
        .map_err(|e| error(Some(id), e));
    }
    sweep_wall_profile(
        &profile,
        [0., 0., wall.height_mm],
        id,
        &roles,
        policy,
        backend,
    )
}

fn has_diagonal_segments(plan: &model::WallPlan) -> bool {
    plan.segments
        .iter()
        .any(|segment| segment.direction[0] != 0. && segment.direction[1] != 0.)
}

fn sweep_wall_profile(
    profile: &CurvedProfile,
    vector: [f64; 3],
    id: EntityId,
    origins: &[Origin],
    policy: GeometryPolicy,
    backend: GeometryBackend,
) -> Result<Solid, ModelError> {
    if backend == GeometryBackend::Rust {
        // Wall outlines and rectangular cutting tools always sweep normal to
        // their authored planes. The orientation determines the sweep sign.
        let normal = profile.frame.normal();
        let height = normal.iter().zip(vector).map(|(a, b)| a * b).sum();
        return crate::rust_backend::stock_profile(id, profile, origins, height, policy);
    }
    Solid::extrude_curved_profile_vector_named(profile, vector, id, origins, policy)
        .map_err(|e| error(Some(id), e))
}

pub(crate) fn cut_tools(
    wall: &model::ResolvedWall,
    plan: &model::WallPlan,
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<Solid>, ModelError> {
    cut_tools_with_backend(
        wall,
        plan,
        id,
        policy,
        interrupt,
        GeometryBackend::OpenCascade,
    )
}

pub(crate) fn cut_tools_with_backend(
    wall: &model::ResolvedWall,
    plan: &model::WallPlan,
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
    backend: GeometryBackend,
) -> Result<Vec<Solid>, ModelError> {
    let mut tools = vec![];
    let depth = plan.left_mm - plan.right_mm + 2.;
    for opening in &wall.openings {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        let seg = plan.segments[opening.segment];
        let start = [
            seg.start[0]
                + seg.direction[0] * opening.offset_mm
                + seg.normal[0] * (plan.right_mm - 1.),
            seg.start[1]
                + seg.direction[1] * opening.offset_mm
                + seg.normal[1] * (plan.right_mm - 1.),
        ];
        let (w, h) = (opening.width_mm, opening.height_mm);
        let corners = [[0., 0.], [w, 0.], [w, h], [0., h]];
        let tool_profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [start[0], start[1], wall.base_z_mm + opening.sill_mm],
                u: [seg.direction[0], seg.direction[1], 0.],
                v: [0., 0., 1.],
            },
            outer: (0..4)
                .map(|i| CurvedEdge2d::Line {
                    a: corners[i],
                    b: corners[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        };
        let roles = ["sill", "jamb.end", "head", "jamb.start"]
            .map(|r| format!("opening.{}.{r}", opening.id));
        let tool = sweep_wall_profile(
            &tool_profile,
            [seg.normal[0] * depth, seg.normal[1] * depth, 0.],
            id,
            &origins(id, roles),
            policy,
            backend,
        )?;
        tools.push(tool);
    }
    for cutout in &wall.cutouts {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        let seg = plan.segments[cutout.segment];
        let ([a, b], [low, high]) = wall.cutout_span(plan, cutout);
        // Reach 1 mm past every free wall face so no sliver skin remains;
        // a chase's back face stays exactly at its depth.
        let (low, high) = match (cutout.through, cutout.face) {
            (true, _) => (low - 1., high + 1.),
            (false, model::WallFace::Left) => (low, high + 1.),
            (false, model::WallFace::Right) => (low - 1., high),
        };
        // The low edge lies toward the right face, the high edge toward the
        // left face; a chase names the edge at its depth `back`.
        let back = |left: bool| match (cutout.through, cutout.face, left) {
            (false, model::WallFace::Left, false) | (false, model::WallFace::Right, true) => "back",
            (_, _, false) => "outside.right",
            (_, _, true) => "outside.left",
        };
        let corners = [[a, low], [b, low], [b, high], [a, high]];
        let tool_profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [seg.start[0], seg.start[1], wall.base_z_mm - 1.],
                u: [seg.direction[0], seg.direction[1], 0.],
                v: [seg.normal[0], seg.normal[1], 0.],
            },
            outer: (0..4)
                .map(|i| CurvedEdge2d::Line {
                    a: corners[i],
                    b: corners[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        };
        let roles = [back(false), "jamb.end", back(true), "jamb.start"]
            .map(|r| format!("cutout.{}.{r}", cutout.id));
        let tool = sweep_wall_profile(
            &tool_profile,
            [0., 0., wall.height_mm + 2.],
            id,
            &origins(id, roles),
            policy,
            backend,
        )?;
        tools.push(tool);
    }
    Ok(tools)
}
fn cut_region(
    mut solid: Solid,
    tools: &[Solid],
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Option<Solid>, ModelError> {
    if solid.uses_rust_geometry() && !tools.is_empty() {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        return solid
            .rust_subtract_orthogonal_tools(tools, policy)
            .map_err(|e| error(Some(id), e));
    }
    for tool in tools {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        let Some(next) = solid
            .boolean(tool, BooleanOperation::Subtract, policy)
            .map_err(|e| error(Some(id), e))?
        else {
            return Ok(None);
        };
        solid = next;
    }
    Ok(Some(solid))
}
pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    backend: GeometryBackend,
    host_solids: &std::collections::BTreeMap<EntityId, Solid>,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    let e = |e: multicad_core::document::DocumentError| error(Some(id), e);
    let source = model::read_wall(document, id).map_err(e)?;
    let wall = model::resolve_wall(document, parameters, &source).map_err(e)?;
    let plan = wall.plan().map_err(e)?;
    wall.check_openings(&plan).map_err(e)?;
    if backend == GeometryBackend::Rust
        && (!wall.junctions.is_empty()
            || (has_diagonal_segments(&plan)
                && (wall.layers.is_some()
                    || !wall.openings.is_empty()
                    || !wall.cutouts.is_empty())))
    {
        return Err(error(
            Some(id),
            "Rust wall geometry requires walls without junctions. Layers, openings and chases require axis-aligned segments",
        ));
    }
    if let Some(layers) = &wall.layers {
        for layer in &layers.stack.layers {
            if layer.thickness_mm > 0. && layer.thickness_mm < policy.minimum_feature_mm {
                let name =
                    match model::read_wall_construction(document, source.wall_type).map_err(e)? {
                        model::WallTypeConstruction::Layered(ty) => ty
                            .layers
                            .layers
                            .into_iter()
                            .find(|item| item.id == layer.id)
                            .map(|item| item.name)
                            .unwrap_or_else(|| "Wall layer".into()),
                        _ => "Wall layer".into(),
                    };
                return Err(error(
                    Some(id),
                    format!(
                        "{name}: thickness {} mm is below the native minimum {} mm",
                        layer.thickness_mm, policy.minimum_feature_mm
                    ),
                ));
            }
        }
        let (left, right) = wall.physical_face_offsets().map_err(e)?;
        if wall.cutouts.iter().any(|cutout| {
            !cutout.through
                && ((cutout.face == model::WallFace::Left && (left - plan.left_mm).abs() > 1e-6)
                    || (cutout.face == model::WallFace::Right
                        && (right - plan.right_mm).abs() > 1e-6)
                    || cutout.depth_mm >= left - right - 1e-6)
        }) {
            return Err(error(
                Some(id),
                "Partial-depth cuts from exterior cavity datums or through all physical material are unsupported; choose an intact physical face and a shallower cut",
            ));
        }
    }
    let tools = cut_tools_with_backend(&wall, &plan, id, policy, interrupt, backend)?;
    let Some(layers) = &wall.layers else {
        let solid = cut_region(
            extrude_region(&wall, &plan, id, policy, None, backend)?,
            &tools,
            id,
            policy,
            interrupt,
        )?
        .ok_or_else(|| error(Some(id), "Openings removed all wall material"))?;
        if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
            return Err(error(
                Some(id),
                "Openings split the wall into separate pieces; keep wall above and beside each opening",
            ));
        }
        return Ok(solid);
    };
    let mut members = vec![];
    for (layer, band) in layers.stack.layers.iter().zip(&layers.bands) {
        if !layer.solid {
            continue;
        }
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        let layer_plan = wall
            .plan_region(
                band.outside_mm.max(band.inside_mm),
                band.outside_mm.min(band.inside_mm),
            )
            .map_err(e)?;
        let solid = extrude_region(&wall, &layer_plan, id, policy, Some(band), backend)?;
        if let Some(solid) = cut_region(solid, &tools, id, policy, interrupt)? {
            members.push(
                solid
                    .with_component_origin(id, layer.id)
                    .map_err(|e| error(Some(id), e))?,
            );
        }
    }
    if members.is_empty() {
        return Err(error(Some(id), "Openings removed all wall material"));
    }
    let solid = if members.len() == 1 {
        members.pop().expect("one retained material member")
    } else {
        ensure_disjoint_members(&members, id, policy, interrupt)?;
        let members = members.iter().collect::<Vec<_>>();
        Solid::compound_named_interruptible(&members, id, policy, interrupt)
            .map_err(|e| error(Some(id), e))?
    };
    for junction in &wall.junctions {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        let host = host_solids.get(&junction.host_wall).ok_or_else(|| {
            error(
                Some(id),
                "Joined host has no active native solid; repair its producer",
            )
        })?;
        let pair = solid
            .inspect_pair(host, policy.linear_tolerance_mm, policy)
            .map_err(|e| error(Some(id), e))?;
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        if pair.relation != multicad_kernel::SolidPairRelation::ContactWithinTolerance {
            return Err(error(
                Some(id),
                "Flush-butt T junction must contact its host without material overlap; simplify the path or repair the join",
            ));
        }
    }
    Ok(solid)
}

/// The wall feature producing a Part, if it is one.
fn wall_feature(document: &Document, part: EntityId) -> Option<EntityId> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return None;
    };
    features.iter().copied().find(|f| {
        matches!(document.get(*f).map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == model::WALL_TYPE)
    })
}

/// Door swings, window panes and opening outlines of walls in a view looking
/// straight down or up, in the view's model-plane coordinates. A downward
/// view draws them for its plan cut; a view looking up draws every opening.
pub(crate) fn plan_vectors(
    document: &Document,
    view: &multicad_core::drawings::DrawingView,
    bodies: &[DrawingBody<'_>],
    vectors: &mut Vec<DrawingVector>,
) -> Result<(), ModelError> {
    let f = &view.frame;
    let n = view_normal(view);
    if matches!(view.kind, ViewKind::Detail { .. }) || n[2].abs() < 1. - 1e-9 {
        return Ok(());
    }
    let cuts = wall_cuts(document, view, bodies)?;
    let walls: Vec<_> = bodies
        .iter()
        .filter_map(|b| wall_feature(document, b.identity).map(|w| (b.identity, w)))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if walls.is_empty() {
        return Ok(());
    }
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
    let project = |p: [f64; 2]| {
        let d = [
            p[0] - f.origin_mm[0],
            p[1] - f.origin_mm[1],
            -f.origin_mm[2],
        ];
        [
            d[0] * f.u[0] + d[1] * f.u[1] + d[2] * f.u[2],
            d[0] * f.v[0] + d[1] * f.v[1] + d[2] * f.v[2],
        ]
    };
    let axis = |a: [f64; 2]| [a[0] * f.u[0] + a[1] * f.u[1], a[0] * f.v[0] + a[1] * f.v[1]];
    for (part, feature) in walls {
        let e = |e: multicad_core::document::DocumentError| error(Some(feature), e);
        let source = model::read_wall(document, feature).map_err(e)?;
        let wall = model::resolve_wall(document, &parameters, &source).map_err(e)?;
        let plan = wall.plan().map_err(e)?;
        let cut = cuts
            .iter()
            .find(|c| c.feature == feature)
            .map(|c| c.cut_z - c.base_z);
        let symbols = wall.plan_symbols_at(&plan, cut);
        let vector = |role: VectorRole,
                      width: f64,
                      curve: VectorCurve,
                      preview_mm: Vec<[f64; 2]>| DrawingVector {
            color_rgb: None,
            source: part,
            material_part: part,
            role,
            curve,
            preview_mm,
            line_width_mm: width,
        };
        for [a, b] in symbols.lines {
            let (a, b) = (project(a), project(b));
            vectors.push(vector(
                VectorRole::Visible,
                PLAN_SYMBOL_MM,
                VectorCurve::Line { a, b },
                vec![a, b],
            ));
        }
        for [a, b] in symbols.overhead {
            let (a, b) = (project(a), project(b));
            vectors.push(vector(
                VectorRole::Hidden,
                PLAN_THIN_MM,
                VectorCurve::Line { a, b },
                vec![a, b],
            ));
        }
        for (center, radius, start, end) in symbols.arcs {
            let steps = 24;
            let preview = (0..=steps)
                .map(|i| {
                    let t = start + (end - start) * i as f64 / steps as f64;
                    project([center[0] + radius * t.cos(), center[1] + radius * t.sin()])
                })
                .collect();
            vectors.push(vector(
                VectorRole::Visible,
                PLAN_THIN_MM,
                VectorCurve::Ellipse {
                    center: project(center),
                    u: axis([1., 0.]),
                    v: axis([0., 1.]),
                    major_mm: radius,
                    minor_mm: radius,
                    start_rad: start,
                    end_rad: end,
                },
                preview,
            ));
        }
    }
    Ok(())
}

impl Model {
    /// The resolved plan of a compiled wall, for inspection and UI overlays.
    pub fn wall_plan(
        &self,
        document: &Document,
        feature: EntityId,
    ) -> Result<(model::ResolvedWall, model::WallPlan), ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(Some(feature), e))?;
        let e = |e: multicad_core::document::DocumentError| error(Some(feature), e);
        let wall = model::resolve_wall(
            document,
            &parameters,
            &model::read_wall(document, feature).map_err(e)?,
        )
        .map_err(e)?;
        let plan = wall.plan().map_err(e)?;
        Ok((wall, plan))
    }
}

#[cfg(test)]
mod layer_guard_tests {
    use super::*;
    #[test]
    fn exact_overlap_guard_allows_contact_rejects_material_overlap_and_observes_cancel() {
        overlap_guard(GeometryBackend::Rust);
        #[cfg(feature = "occt")]
        overlap_guard(GeometryBackend::OpenCascade);
    }
    fn overlap_guard(backend: GeometryBackend) {
        let id = EntityId::new();
        let policy = GeometryPolicy::default();
        let a = match backend {
            GeometryBackend::Rust => Solid::rust_box(
                id.uuid_bits(),
                [multicad_kernel::rust_kernel::Rational::new(10, 1).unwrap(); 3],
                Default::default(),
                policy,
            ),
            GeometryBackend::OpenCascade => Solid::box_named([10.; 3], id, policy),
        }
        .unwrap();
        let move_by = |x| {
            a.transformed(multicad_kernel::RigidTransform {
                translation_mm: [x, 0., 0.],
                ..Default::default()
            })
            .unwrap()
        };
        ensure_disjoint_members(&[move_by(0.), move_by(10.)], id, policy, None).unwrap();
        assert!(
            ensure_disjoint_members(&[move_by(0.), move_by(9.5)], id, policy, None)
                .unwrap_err()
                .message
                .contains("overlap")
        );
        assert!(crate::is_superseded(
            &ensure_disjoint_members(&[move_by(0.), move_by(10.)], id, policy, Some(&|| true))
                .unwrap_err()
        ));
        assert!((a.volume_mm3().unwrap() - 1000.).abs() < 1e-6);
    }
}
