//! Native wall evaluation: the planned outline is extruded to the wall
//! height and each opening is subtracted as an exact prism with stable face
//! roles. Downward drawing views also receive door swings and window panes.
use crate::{DrawingBody, Model, ModelError, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::{DrawingVector, VectorCurve, VectorRole, ViewKind},
    model::{self, Loop},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
};

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

pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    let e = |e: multicad_core::document::DocumentError| error(Some(id), e);
    let source = model::read_wall(document, id).map_err(e)?;
    let wall = model::resolve_wall(document, parameters, &source).map_err(e)?;
    let plan = wall.plan().map_err(e)?;
    wall.check_openings(&plan).map_err(e)?;
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
        .cloned();
    let mut solid = Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., wall.height_mm],
        id,
        &origins(id, roles),
        policy,
    )
    .map_err(|e| error(Some(id), e))?;
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
        let tool = Solid::extrude_curved_profile_vector_named(
            &tool_profile,
            [seg.normal[0] * depth, seg.normal[1] * depth, 0.],
            id,
            &origins(id, roles),
            policy,
        )
        .map_err(|e| error(Some(id), e))?;
        solid = solid
            .boolean(&tool, BooleanOperation::Subtract, policy)
            .map_err(|e| error(Some(id), e))?
            .ok_or_else(|| error(Some(id), "An opening removed the whole wall"))?;
    }
    for cutout in &wall.cutouts {
        if interrupt.is_some_and(|f| f()) {
            return Err(error(Some(id), "Wall evaluation superseded"));
        }
        let seg = plan.segments[cutout.segment];
        let ([a, b], [low, high]) = wall.cutout_span(&plan, cutout);
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
        let tool = Solid::extrude_curved_profile_vector_named(
            &tool_profile,
            [0., 0., wall.height_mm + 2.],
            id,
            &origins(id, roles),
            policy,
        )
        .map_err(|e| error(Some(id), e))?;
        solid = solid
            .boolean(&tool, BooleanOperation::Subtract, policy)
            .map_err(|e| error(Some(id), e))?
            .ok_or_else(|| error(Some(id), "A vertical cutout removed the whole wall"))?;
    }
    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
        return Err(error(
            Some(id),
            "Openings split the wall into separate pieces; keep wall above and beside each opening",
        ));
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

/// Door swings and window panes of walls in a view looking straight down or
/// up, in the view's model-plane coordinates.
pub(crate) fn plan_vectors(
    document: &Document,
    view: &multicad_core::drawings::DrawingView,
    bodies: &[DrawingBody<'_>],
    vectors: &mut Vec<DrawingVector>,
) -> Result<(), ModelError> {
    let f = &view.frame;
    let n = [
        f.u[1] * f.v[2] - f.u[2] * f.v[1],
        f.u[2] * f.v[0] - f.u[0] * f.v[2],
        f.u[0] * f.v[1] - f.u[1] * f.v[0],
    ];
    if matches!(view.kind, ViewKind::Detail { .. }) || n[2].abs() < 1. - 1e-9 {
        return Ok(());
    }
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
        let symbols = wall.plan_symbols(&plan);
        let vector = |curve: VectorCurve, preview_mm: Vec<[f64; 2]>| DrawingVector {
            source: part,
            material_part: part,
            role: VectorRole::Visible,
            curve,
            preview_mm,
            line_width_mm: 0.18,
        };
        for [a, b] in symbols.lines {
            let (a, b) = (project(a), project(b));
            vectors.push(vector(VectorCurve::Line { a, b }, vec![a, b]));
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
