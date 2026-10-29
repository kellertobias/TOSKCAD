//! Native slabs, pedestals and stairs, service envelopes around placed
//! equipment with model clearance checks, and their plan graphics.
use crate::{DrawingBody, Model, ModelError, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::{DrawingVector, VectorCurve, VectorRole, ViewKind},
    model::{self, RigidPose},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
};

fn closed(points: &[[f64; 2]]) -> Vec<CurvedEdge2d> {
    let n = points.len();
    (0..n)
        .map(|i| CurvedEdge2d::Line {
            a: points[i],
            b: points[(i + 1) % n],
        })
        .collect()
}
fn named(id: EntityId, prefix: &str, n: usize) -> Vec<Origin> {
    (0..n)
        .map(|i| Origin {
            source: id,
            role: format!("{prefix}.{i}"),
        })
        .collect()
}

pub(crate) fn build_slab(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let e = |e: multicad_core::document::DocumentError| error(Some(id), e);
    let slab = model::resolve_slab(
        document,
        parameters,
        &model::read_slab(document, id).map_err(e)?,
    )
    .map_err(e)?;
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., slab.bottom_mm],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        outer: closed(&slab.outline),
        holes: vec![],
    };
    Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., slab.top_mm - slab.bottom_mm],
        id,
        &named(id, "slab.edge", slab.outline.len()),
        policy,
    )
    .map_err(|e| error(Some(id), e))
}

pub(crate) fn build_stair(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let e = |e: multicad_core::document::DocumentError| error(Some(id), e);
    let stair = model::resolve_stair(
        document,
        parameters,
        &model::read_stair(document, id).map_err(e)?,
    )
    .map_err(e)?;
    let mut result: Option<Solid> = None;
    for piece in &stair.pieces {
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [piece.origin[0], piece.origin[1], piece.base_z_mm],
                u: [piece.along[0], piece.along[1], 0.],
                v: [0., 0., 1.],
            },
            outer: closed(&piece.profile),
            holes: vec![],
        };
        let left = [
            -piece.along[1] * piece.width_mm,
            piece.along[0] * piece.width_mm,
            0.,
        ];
        let solid = Solid::extrude_curved_profile_vector_named(
            &profile,
            left,
            id,
            &named(id, &format!("{}.edge", piece.role), piece.profile.len()),
            policy,
        )
        .map_err(|e| error(Some(id), e))?;
        result = Some(match result {
            None => solid,
            Some(r) => r
                .boolean(&solid, BooleanOperation::Union, policy)
                .map_err(|e| error(Some(id), e))?
                .ok_or_else(|| error(Some(id), "Stair pieces produced no material"))?,
        });
    }
    let solid = result.ok_or_else(|| error(Some(id), "Stair has no flights"))?;
    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
        return Err(error(
            Some(id),
            "Stair flights and landing do not connect into one solid",
        ));
    }
    Ok(solid)
}

/// A service envelope: its box corners in world millimeters (bottom four
/// then top four) and the final solids it intersects.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopeEvaluation {
    pub envelope: EntityId,
    pub target: EntityId,
    pub corners: [[f64; 3]; 8],
    pub volume_mm3: f64,
    /// Final Parts or Instances inside the envelope, with overlap volume.
    pub conflicts: Vec<(EntityId, f64)>,
}
fn transform(pose: RigidPose) -> RigidTransform {
    RigidTransform {
        rotation: pose.rotation,
        translation_mm: pose.translation_mm,
    }
}
impl Model {
    fn target_pose_and_solid(&self, target: EntityId) -> Option<(RigidPose, &Solid)> {
        if let Some(o) = self.snapshot().occurrences.get(&target) {
            return Some((o.pose, self.solid(target)?));
        }
        let producer = *self.snapshot().bodies.get(&target)?;
        Some((RigidPose::default(), self.solid(producer)?))
    }
    /// One envelope's box solid and world corners. The box is the target's
    /// own bounds in its local frame plus the clearances; it rotates and
    /// moves with the target.
    fn envelope_box(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        id: EntityId,
    ) -> Result<(model::EnvelopeDefinition, Solid, [[f64; 3]; 8]), ModelError> {
        let mm = |d: &multicad_core::constraint_sketch::Dimension| {
            d.value_in(
                multicad_core::parameters::Unit::Millimeter,
                Some(parameters),
            )
            .map_err(|e| error(Some(id), e))
        };
        let env = model::read_envelope(document, id).map_err(|e| error(Some(id), e))?;
        let (pose, solid) = self
            .target_pose_and_solid(env.target)
            .ok_or_else(|| error(Some(id), "The envelope's target has no current solid"))?;
        let inverse = pose.inverse().map_err(|e| error(Some(id), e))?;
        let local = solid
            .transformed(transform(inverse))
            .map_err(|e| error(Some(id), e))?;
        let b = local.bounds().map_err(|e| error(Some(id), e))?;
        let min = [
            b.min[0] - mm(&env.left)?,
            b.min[1] - mm(&env.front)?,
            b.min[2],
        ];
        let max = [
            b.max[0] + mm(&env.right)?,
            b.max[1] + mm(&env.back)?,
            b.max[2] + mm(&env.top)?,
        ];
        if (0..3).any(|i| max[i] - min[i] <= 1e-3) {
            return Err(error(Some(id), "Service envelope has no volume"));
        }
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [0., 0., min[2]],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            outer: closed(&[
                [min[0], min[1]],
                [max[0], min[1]],
                [max[0], max[1]],
                [min[0], max[1]],
            ]),
            holes: vec![],
        };
        let boxed = Solid::extrude_curved_profile(&profile, max[2] - min[2], self.policy)
            .and_then(|s| s.transformed(transform(pose)))
            .map_err(|e| error(Some(id), e))?;
        let mut corners = [[0.; 3]; 8];
        for (k, c) in corners.iter_mut().enumerate() {
            let p = [
                if matches!(k % 4, 1 | 2) {
                    max[0]
                } else {
                    min[0]
                },
                if matches!(k % 4, 2 | 3) {
                    max[1]
                } else {
                    min[1]
                },
                if k < 4 { min[2] } else { max[2] },
            ];
            *c = pose.point(p);
        }
        Ok((env, boxed, corners))
    }
    /// Viewport outlines of every drawing guide, sized to the final model.
    pub(crate) fn guide_outlines(
        &self,
        document: &Document,
    ) -> std::collections::BTreeMap<EntityId, Vec<[[f64; 3]; 4]>> {
        let snapshot = self.snapshot();
        let mut bounds: Option<[[f64; 3]; 2]> = None;
        let finals = snapshot
            .bodies
            .iter()
            .filter(|(part, _)| !snapshot.consumed_bodies.contains(part))
            .filter_map(|(_, producer)| snapshot.shapes.get(producer).map(|s| s.bounds))
            .chain(snapshot.occurrences.values().map(|o| o.bounds));
        for b in finals {
            let [lo, hi] = bounds.unwrap_or([[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]]);
            bounds = Some([
                std::array::from_fn(|i| lo[i].min(b.min[i])),
                std::array::from_fn(|i| hi[i].max(b.max[i])),
            ]);
        }
        let (Some(bounds), Ok(parameters)) =
            (bounds, multicad_core::parameters::evaluate(document))
        else {
            return Default::default();
        };
        multicad_core::drawings::guides(document)
            .filter_map(|e| {
                let guide = multicad_core::drawings::read_guide(document, e.id).ok()?;
                let loops =
                    multicad_core::drawings::guide_outlines(document, &parameters, &guide, bounds);
                (!loops.is_empty()).then_some((e.id, loops))
            })
            .collect()
    }
    /// Display boxes of every envelope whose target currently has a solid.
    pub(crate) fn envelope_boxes(
        &self,
        document: &Document,
    ) -> std::collections::BTreeMap<EntityId, [[f64; 3]; 8]> {
        let Ok(parameters) = multicad_core::parameters::evaluate(document) else {
            return Default::default();
        };
        model::envelopes(document)
            .filter_map(|e| {
                self.envelope_box(document, &parameters, e.id)
                    .ok()
                    .map(|(_, _, c)| (e.id, c))
            })
            .collect()
    }
    /// Evaluate every service envelope against the current model. Overlaps
    /// below 1 mm³ are ignored.
    pub fn envelopes(&self, document: &Document) -> Result<Vec<EnvelopeEvaluation>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut out = vec![];
        for entity in model::envelopes(document) {
            let id = entity.id;
            let (env, boxed, corners) = self.envelope_box(document, &parameters, id)?;
            let mut conflicts = vec![];
            let others = self
                .snapshot()
                .bodies
                .iter()
                .filter(|(part, _)| !self.snapshot().consumed_bodies.contains(part))
                .map(|(part, producer)| (*part, *producer))
                .chain(self.snapshot().occurrences.keys().map(|i| (*i, *i)));
            for (subject, producer) in others {
                if subject == env.target {
                    continue;
                }
                let Some(other) = self.solid(producer) else {
                    continue;
                };
                let overlap = boxed
                    .boolean(other, BooleanOperation::Intersect, self.policy)
                    .map_err(|e| error(Some(id), e))?
                    .map(|s| s.volume_mm3())
                    .transpose()
                    .map_err(|e| error(Some(id), e))?
                    .unwrap_or(0.);
                if overlap > 1. {
                    conflicts.push((subject, overlap));
                }
            }
            out.push(EnvelopeEvaluation {
                envelope: id,
                target: env.target,
                corners,
                volume_mm3: boxed.volume_mm3().map_err(|e| error(Some(id), e))?,
                conflicts,
            });
        }
        Ok(out)
    }
}

fn feature_of(document: &Document, part: EntityId, type_id: &str) -> Option<EntityId> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return None;
    };
    features.iter().copied().find(|f| {
        matches!(document.get(*f).map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == type_id)
    })
}
/// Stair walking lines with arrows and dashed service envelopes in views
/// looking straight down or up.
pub(crate) fn plan_vectors(
    document: &Document,
    model: &Model,
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
    let mut line = |source: EntityId, a: [f64; 2], b: [f64; 2], role: VectorRole| {
        let (a, b) = (project(a), project(b));
        vectors.push(DrawingVector {
            source,
            material_part: source,
            role,
            curve: VectorCurve::Line { a, b },
            preview_mm: vec![a, b],
            line_width_mm: 0.18,
        });
    };
    let identities: std::collections::BTreeSet<_> = bodies.iter().map(|b| b.identity).collect();
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
    for part in &identities {
        let Some(feature) = feature_of(document, *part, model::STAIR_TYPE) else {
            continue;
        };
        let e = |e: multicad_core::document::DocumentError| error(Some(feature), e);
        let stair = model::resolve_stair(
            document,
            &parameters,
            &model::read_stair(document, feature).map_err(e)?,
        )
        .map_err(e)?;
        let w = &stair.walking_line;
        for pair in w.windows(2) {
            line(*part, pair[0], pair[1], VectorRole::Visible);
        }
        if let [.., a, b] = w.as_slice() {
            let d = [b[0] - a[0], b[1] - a[1]];
            let l = d[0].hypot(d[1]).max(1e-9);
            let (u, v) = ([d[0] / l, d[1] / l], [-d[1] / l, d[0] / l]);
            let size = 150.;
            for side in [1., -1.] {
                let tip = [
                    b[0] - u[0] * size + v[0] * side * size * 0.5,
                    b[1] - u[1] * size + v[1] * side * size * 0.5,
                ];
                line(*part, *b, tip, VectorRole::Visible);
            }
        }
    }
    let envelopes: Vec<_> = model
        .envelopes(document)?
        .into_iter()
        .filter(|e| identities.contains(&e.target))
        .collect();
    for env in envelopes {
        for k in 0..4 {
            let (a, b) = (env.corners[k], env.corners[(k + 1) % 4]);
            line(env.target, [a[0], a[1]], [b[0], b[1]], VectorRole::Hidden);
        }
    }
    Ok(())
}
