//! Exact independent fence Parts. A provider miss builds and validates the
//! complete run before returning any member, including in source-scoped jobs.
//! Cheap complete-source/material validation also precedes cache hits.
use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    materials::Resolver,
    model::{self, FenceLayout, FenceMemberDefinition, FenceMemberKey, ResolvedFencePostProfile},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, Bounds, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
};
use std::collections::BTreeMap;

struct Run {
    layout: FenceLayout,
    members: BTreeMap<FenceMemberKey, (EntityId, EntityId)>,
    solids: Option<BTreeMap<EntityId, Solid>>,
}

pub(crate) struct Context<'a> {
    document: &'a Document,
    parameters: &'a EvaluatedParameters,
    materials: Option<Resolver<'a>>,
    runs: BTreeMap<EntityId, Run>,
}
fn checkpoint(id: EntityId, interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ModelError> {
    if interrupt.is_some_and(|f| f()) {
        return Err(error(Some(id), "Fence evaluation superseded"));
    }
    Ok(())
}

impl<'a> Context<'a> {
    pub(crate) fn new(document: &'a Document, parameters: &'a EvaluatedParameters) -> Self {
        Self {
            document,
            parameters,
            materials: None,
            runs: BTreeMap::new(),
        }
    }
    pub(crate) fn prepare(
        &mut self,
        feature: EntityId,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<FenceMemberDefinition, ModelError> {
        checkpoint(feature, interrupt)?;
        let source = model::read_fence_member(self.document, feature)
            .map_err(|e| error(Some(feature), e))?;
        if !self.runs.contains_key(&source.run) {
            let value = model::read_fence_run(self.document, source.run)
                .map_err(|e| error(Some(source.run), e))?;
            let layout = model::resolve_fence_run(self.document, self.parameters, &value)
                .map_err(|e| error(Some(source.run), e))?;
            let members: BTreeMap<_, _> = model::fence_members(self.document, source.run)
                .map_err(|e| error(Some(source.run), e))?
                .into_iter()
                .map(|(key, feature, part)| (key, (feature, part)))
                .collect();
            let specs = layout.member_specs();
            if members.len() != specs.len() {
                return Err(error(
                    Some(source.run),
                    "Fence member set is incomplete or stale; rebuild the run from its construction",
                ));
            }
            if self.materials.is_none() {
                self.materials =
                    Some(Resolver::new(self.document).map_err(|e| error(Some(source.run), e))?);
            }
            for spec in &specs {
                checkpoint(source.run, interrupt)?;
                let Some((feature, part)) = members.get(&spec.key) else {
                    return Err(error(
                        Some(source.run),
                        "Fence layout and retained member identities disagree; repair the run",
                    ));
                };
                if matches!(
                    self.document.get(*feature).map(|e| &e.data),
                    Some(EntityData::Feature {
                        suppressed: true,
                        ..
                    })
                ) {
                    return Err(error(
                        Some(*feature),
                        "A generated fence member is suppressed; edit its construction or run instead",
                    ));
                }
                let effective = self
                    .materials
                    .as_ref()
                    .expect("initialized resolver")
                    .resolve(*part)
                    .map_err(|e| {
                        error(
                            Some(*part),
                            format!("Fence member material assignment: {e}"),
                        )
                    })?;
                if effective.material != Some(spec.material) {
                    return Err(error(
                        Some(*part),
                        "Fence member material differs from its construction; repair the type or assignment",
                    ));
                }
            }
            self.runs.insert(
                source.run,
                Run {
                    layout,
                    members,
                    solids: None,
                },
            );
        }
        Ok(source)
    }
    pub(crate) fn build(
        &mut self,
        feature: EntityId,
        source: &FenceMemberDefinition,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Solid, ModelError> {
        let run = self
            .runs
            .get_mut(&source.run)
            .ok_or_else(|| error(Some(feature), "Fence source was not prepared"))?;
        if run.solids.is_none() {
            let mut solids = BTreeMap::new();
            for post in &run.layout.posts {
                checkpoint(source.run, interrupt)?;
                let id = run.members[&FenceMemberKey::Post { post: post.key }].0;
                let shape = match run.layout.construction.post {
                    ResolvedFencePostProfile::Rectangular { width_mm, depth_mm } => rectangle(
                        id,
                        "fence.post",
                        post.position_mm,
                        post.direction,
                        width_mm,
                        depth_mm,
                        run.layout.construction.height_mm,
                        policy,
                    )?,
                    ResolvedFencePostProfile::Round { diameter_mm } => round_post(
                        id,
                        post.position_mm,
                        post.direction,
                        diameter_mm,
                        run.layout.construction.height_mm,
                        policy,
                    )?,
                };
                solids.insert(id, shape);
            }
            for member in &run.layout.infills {
                checkpoint(source.run, interrupt)?;
                let id = run.members[&FenceMemberKey::Infill { infill: member.key }].0;
                let prefix = match member.kind {
                    model::FenceLayoutInfillKind::Panel { .. } => "fence.panel",
                    model::FenceLayoutInfillKind::Rail => "fence.rail",
                    model::FenceLayoutInfillKind::Picket => "fence.picket",
                };
                let shape = rectangle(
                    id,
                    prefix,
                    member.position_mm,
                    member.direction,
                    member.width_mm,
                    member.depth_mm,
                    member.height_mm,
                    policy,
                )?;
                solids.insert(id, shape);
            }
            disjoint(&solids, self.document, source.run, policy, interrupt)?;
            checkpoint(source.run, interrupt)?;
            run.solids = Some(solids);
        }
        checkpoint(source.run, interrupt)?;
        run.solids
            .as_mut()
            .expect("completed run")
            .remove(&feature)
            .ok_or_else(|| {
                error(
                    Some(feature),
                    "Completed fence member is missing or was already consumed",
                )
            })
    }
}

fn frame(position: [f64; 3], direction: [f64; 2]) -> PlaneFrame {
    PlaneFrame {
        origin: position,
        u: [direction[0], direction[1], 0.],
        v: [-direction[1], direction[0], 0.],
    }
}
fn dimensions(id: EntityId, values: &[f64], policy: GeometryPolicy) -> Result<(), ModelError> {
    if values
        .iter()
        .any(|v| !v.is_finite() || *v < policy.minimum_feature_mm)
    {
        return Err(error(
            Some(id),
            format!(
                "Fence member dimension is below the native minimum {} mm; change the construction without inflating its geometry",
                policy.minimum_feature_mm
            ),
        ));
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn rectangle(
    id: EntityId,
    prefix: &str,
    position: [f64; 3],
    direction: [f64; 2],
    width: f64,
    depth: f64,
    height: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    dimensions(id, &[width, depth, height], policy)?;
    let corners = [
        [-width / 2., -depth / 2.],
        [width / 2., -depth / 2.],
        [width / 2., depth / 2.],
        [-width / 2., depth / 2.],
    ];
    let profile = CurvedProfile {
        frame: frame(position, direction),
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: corners[i],
                b: corners[(i + 1) % 4],
            })
            .collect(),
        holes: Vec::new(),
    };
    let origins = ["back", "end", "front", "start"].map(|role| Origin {
        source: id,
        role: format!("{prefix}.{role}"),
    });
    let solid = Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., height],
        id,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(id), e))?;
    measured(solid, id, width * depth * height)
}
fn round_post(
    id: EntityId,
    position: [f64; 3],
    direction: [f64; 2],
    diameter: f64,
    height: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    dimensions(id, &[diameter, height], policy)?;
    let profile = CurvedProfile {
        frame: frame(position, direction),
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: diameter / 2.,
        }],
        holes: Vec::new(),
    };
    let solid = Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., height],
        id,
        &[Origin {
            source: id,
            role: "fence.post.round".into(),
        }],
        policy,
    )
    .map_err(|e| error(Some(id), e))?;
    measured(
        solid,
        id,
        std::f64::consts::PI * (diameter / 2.).powi(2) * height,
    )
}
fn measured(solid: Solid, id: EntityId, expected: f64) -> Result<Solid, ModelError> {
    let volume = solid.volume_mm3().map_err(|e| error(Some(id), e))?;
    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1
        || !volume.is_finite()
        || volume <= 0.
        || (volume - expected).abs() > (expected.abs() * 1e-8).max(1e-6)
    {
        return Err(error(
            Some(id),
            "Fence member solid count or measured volume disagrees with its source dimensions",
        ));
    }
    Ok(solid)
}

fn disjoint(
    solids: &BTreeMap<EntityId, Solid>,
    document: &Document,
    run: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), ModelError> {
    let mut bounds: Vec<(EntityId, Bounds)> = solids
        .iter()
        .map(|(id, solid)| {
            solid
                .bounds()
                .map(|b| (*id, b))
                .map_err(|e| error(Some(*id), e))
        })
        .collect::<Result<_, _>>()?;
    // Sweep the longest world extent; bounds only prune candidates. Native
    // COMMON with no extra fuzzy gap closing decides occupied material overlap.
    let span = |axis| {
        bounds
            .iter()
            .map(|(_, b)| b.max[axis])
            .fold(f64::NEG_INFINITY, f64::max)
            - bounds
                .iter()
                .map(|(_, b)| b.min[axis])
                .fold(f64::INFINITY, f64::min)
    };
    let axis = (0..3)
        .max_by(|a, b| span(*a).total_cmp(&span(*b)))
        .unwrap_or(0);
    bounds.sort_by(|(_, a), (_, b)| a.min[axis].total_cmp(&b.min[axis]));
    for i in 0..bounds.len() {
        checkpoint(run, interrupt)?;
        let (a, ab) = bounds[i];
        for &(b, bb) in &bounds[i + 1..] {
            if bb.min[axis] >= ab.max[axis] {
                break;
            }
            if (0..3).any(|k| ab.max[k] <= bb.min[k] || bb.max[k] <= ab.min[k]) {
                continue;
            }
            checkpoint(run, interrupt)?;
            let common = solids[&a]
                .boolean(
                    &solids[&b],
                    BooleanOperation::Intersect,
                    GeometryPolicy {
                        boolean_fuzzy_mm: 0.,
                        ..policy
                    },
                )
                .map_err(|e| error(Some(run), e))?;
            checkpoint(run, interrupt)?;
            if common
                .as_ref()
                .map(Solid::volume_mm3)
                .transpose()
                .map_err(|e| error(Some(run), e))?
                .is_some_and(|volume| volume > 0.)
            {
                let name = |id| {
                    model::read_fence_member(document, id)
                        .ok()
                        .and_then(|member| document.get(member.output.body))
                        .map_or("Fence member", |entity| entity.name.as_str())
                };
                return Err(error(
                    Some(run),
                    format!(
                        "{} and {} overlap in native material; repair post positions or infill dimensions/offsets",
                        name(a),
                        name(b)
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        document::{Entity, Mutation},
        model::*,
    };
    use std::cell::Cell;

    #[test]
    fn interrupted_run_generation_returns_no_partial_member_and_retries_completely() {
        let material = Entity::new(
            "Timber",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [150, 100, 70, 255],
                section_hatch: None,
            },
        );
        let document = Document::new("Cancelled fence")
            .unwrap()
            .edited(&[Mutation::Insert(material.clone())])
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let ty = FenceTypeRequest::new(
            "Fence",
            FenceConstruction {
                height: length(1800.),
                post: FencePostProfile::Rectangular {
                    width: length(90.),
                    depth: length(90.),
                },
                post_material: material.id,
                preferred_post_spacing: length(2000.),
                layout: FenceBayLayout::ShortLast,
                infills: vec![FenceInfill {
                    id: EntityId::new(),
                    name: "Panel".into(),
                    material: material.id,
                    bottom: length(100.),
                    height: length(1600.),
                    normal_offset: length(0.),
                    construction: FenceInfillKind::Panel {
                        thickness: length(18.),
                        width: FencePanelWidth::FitBay,
                    },
                }],
            },
        );
        let path = FencePath {
            vertices: [[0., 0.], [4500., 0.]]
                .into_iter()
                .map(|p| FenceVertex {
                    id: EntityId::new(),
                    point: p.map(length),
                })
                .collect(),
            closed: false,
            base_z: length(0.),
        };
        let run = FenceRunRequest::new("Boundary", FenceRunDefinition::new(ty.id, path));
        let prepared = prepare_fence_change(
            &session,
            0,
            Actor::local(Channel::Ui),
            &FenceChange {
                new_type: Some(ty),
                run: Some(FenceRunChange::Create(run.clone())),
                ..Default::default()
            },
        )
        .unwrap();
        session.commit(prepared.model.prepared).unwrap();
        let members = fence_members(session.document(), run.id).unwrap();
        let mut context = Context::new(session.document(), session.parameters());
        let source = context.prepare(members[0].1, None).unwrap();
        let checkpoints = Cell::new(0);
        let cancel = || {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() >= 3
        };
        let result = context.build(
            members[0].1,
            &source,
            GeometryPolicy::default(),
            Some(&cancel),
        );
        assert!(
            result
                .err()
                .expect("Cancellation must refuse the member")
                .message
                .contains("superseded")
        );
        assert!(checkpoints.get() >= 3);
        let mut volume = 0.;
        for (_, feature, _) in members {
            let source = context.prepare(feature, None).unwrap();
            let solid = context
                .build(feature, &source, GeometryPolicy::default(), None)
                .unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            volume += solid.volume_mm3().unwrap();
        }
        assert!((volume - 180_144_000.).abs() < 0.01);
        assert_eq!(session.revision(), 1);
    }
}
