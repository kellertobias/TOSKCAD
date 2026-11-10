//! Exact independent fence Parts. A provider miss builds and validates the
//! complete run before returning any member, including in source-scoped jobs.
//! Cheap complete-source/material validation also precedes cache hits.
use crate::{GeometryBackend, ModelError, error};
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
    backend: GeometryBackend,
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
    pub(crate) fn new(
        document: &'a Document,
        parameters: &'a EvaluatedParameters,
        backend: GeometryBackend,
    ) -> Self {
        Self {
            document,
            parameters,
            backend,
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
                let profile = match run.layout.construction.post {
                    ResolvedFencePostProfile::Rectangular { width_mm, depth_mm } => {
                        Footprint::Rectangle([width_mm, depth_mm])
                    }
                    ResolvedFencePostProfile::Round { diameter_mm } => {
                        Footprint::Circle(diameter_mm)
                    }
                };
                let section = Section::new(post.position_mm, post.direction, profile);
                let shape = section.sweep(
                    id,
                    "fence.post",
                    run.layout.construction.height_mm,
                    policy,
                    self.backend,
                )?;
                solids.insert(id, (shape, section));
            }
            for member in &run.layout.infills {
                checkpoint(source.run, interrupt)?;
                let id = run.members[&FenceMemberKey::Infill { infill: member.key }].0;
                let prefix = match member.kind {
                    model::FenceLayoutInfillKind::Panel { .. } => "fence.panel",
                    model::FenceLayoutInfillKind::Rail => "fence.rail",
                    model::FenceLayoutInfillKind::Picket => "fence.picket",
                };
                let section = Section::new(
                    member.position_mm,
                    member.direction,
                    Footprint::Rectangle([member.width_mm, member.depth_mm]),
                );
                let shape = section.sweep(id, prefix, member.height_mm, policy, self.backend)?;
                solids.insert(id, (shape, section));
            }
            disjoint(
                &solids,
                self.document,
                source.run,
                policy,
                interrupt,
                self.backend,
            )?;
            checkpoint(source.run, interrupt)?;
            run.solids = Some(
                solids
                    .into_iter()
                    .map(|(id, (solid, _))| (id, solid))
                    .collect(),
            );
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
#[derive(Clone, Copy)]
enum Footprint {
    Rectangle([f64; 2]),
    Circle(f64),
}

struct Section {
    frame: PlaneFrame,
    footprint: Footprint,
}

impl Section {
    fn new(position: [f64; 3], [x, y]: [f64; 2], footprint: Footprint) -> Self {
        Self {
            frame: PlaneFrame {
                origin: position,
                u: [x, y, 0.],
                v: [-y, x, 0.],
            },
            footprint,
        }
    }

    fn sweep(
        &self,
        id: EntityId,
        prefix: &str,
        height: f64,
        policy: GeometryPolicy,
        backend: GeometryBackend,
    ) -> Result<Solid, ModelError> {
        let (outer, roles, area) = match self.footprint {
            Footprint::Rectangle([width, depth]) => {
                dimensions(id, &[width, depth, height], policy)?;
                let (w, d) = (width / 2., depth / 2.);
                let corners = [[-w, -d], [w, -d], [w, d], [-w, d]];
                let edges = (0..4)
                    .map(|i| CurvedEdge2d::Line {
                        a: corners[i],
                        b: corners[(i + 1) % 4],
                    })
                    .collect();
                (edges, &["back", "end", "front", "start"][..], width * depth)
            }
            Footprint::Circle(diameter) => {
                dimensions(id, &[diameter, height], policy)?;
                let radius = diameter / 2.;
                (
                    vec![CurvedEdge2d::Circle {
                        center: [0.; 2],
                        radius_mm: radius,
                    }],
                    &["round"][..],
                    std::f64::consts::PI * radius.powi(2),
                )
            }
        };
        let profile = CurvedProfile {
            frame: self.frame,
            outer,
            holes: Vec::new(),
        };
        let origins = roles
            .iter()
            .map(|role| Origin {
                source: id,
                role: format!("{prefix}.{role}"),
            })
            .collect::<Vec<_>>();
        let solid = match backend {
            GeometryBackend::Rust => {
                Solid::rust_extrude_normal_profile_named(&profile, height, id, &origins, policy)
            }
            GeometryBackend::OpenCascade => Solid::extrude_curved_profile_vector_named(
                &profile,
                [0., 0., height],
                id,
                &origins,
                policy,
            ),
        }
        .map_err(|e| error(Some(id), e))?;
        measured(solid, id, area * height)
    }

    fn axes(&self) -> [[f64; 2]; 2] {
        [
            [self.frame.u[0], self.frame.u[1]],
            [self.frame.v[0], self.frame.v[1]],
        ]
    }

    /// Both members are vertical sweeps of convex sections. Once their Z
    /// intervals overlap, open interiors overlap exactly when these sections do.
    fn overlaps(&self, other: &Self) -> bool {
        let offset = std::array::from_fn(|i| other.frame.origin[i] - self.frame.origin[i]);
        let dot = |a: [f64; 2], b: [f64; 2]| a[0].mul_add(b[0], a[1] * b[1]);
        // Account for roundoff at authored contacts when section frames are rotated.
        let guard = 16.
            * f64::EPSILON
            * [self, other]
                .into_iter()
                .flat_map(|section| {
                    let [x, y, _] = section.frame.origin;
                    let width = match section.footprint {
                        Footprint::Rectangle([w, d]) => w.max(d),
                        Footprint::Circle(d) => d,
                    };
                    [x.abs(), y.abs(), width]
                })
                .fold(0., f64::max);
        let rectangle_circle = |rectangle: &Self, size: [f64; 2], diameter, offset| {
            let [u, v] = rectangle.axes();
            let outside = |axis, extent: f64| (dot(offset, axis).abs() - extent).max(0.);
            outside(u, size[0] / 2.).hypot(outside(v, size[1] / 2.)) + guard < diameter / 2.
        };
        match (self.footprint, other.footprint) {
            (Footprint::Rectangle(a), Footprint::Rectangle(b)) => {
                let [a_axes, b_axes] = [self.axes(), other.axes()];
                let radius = |size: [f64; 2], [u, v]: [[f64; 2]; 2], axis| {
                    (size[0] * dot(u, axis).abs() + size[1] * dot(v, axis).abs()) / 2.
                };
                a_axes.into_iter().chain(b_axes).all(|axis| {
                    dot(offset, axis).abs() + guard
                        < radius(a, a_axes, axis) + radius(b, b_axes, axis)
                })
            }
            (Footprint::Circle(a), Footprint::Circle(b)) => {
                offset[0].hypot(offset[1]) + guard < (a + b) / 2.
            }
            (Footprint::Rectangle(a), Footprint::Circle(b)) => rectangle_circle(self, a, b, offset),
            (Footprint::Circle(a), Footprint::Rectangle(b)) => {
                rectangle_circle(other, b, a, offset.map(|value| -value))
            }
        }
    }
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
    solids: &BTreeMap<EntityId, (Solid, Section)>,
    document: &Document,
    run: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
    backend: GeometryBackend,
) -> Result<(), ModelError> {
    let mut bounds: Vec<(EntityId, Bounds)> = solids
        .iter()
        .map(|(id, (solid, _))| {
            solid
                .bounds()
                .map(|b| (*id, b))
                .map_err(|e| error(Some(*id), e))
        })
        .collect::<Result<_, _>>()?;
    // Sweep the longest world extent. Bounds only prune candidates.
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
            let (a_solid, a_section) = &solids[&a];
            let (b_solid, b_section) = &solids[&b];
            let overlap = match backend {
                GeometryBackend::Rust => a_section.overlaps(b_section),
                GeometryBackend::OpenCascade => a_solid
                    .boolean(
                        b_solid,
                        BooleanOperation::Intersect,
                        GeometryPolicy {
                            boolean_fuzzy_mm: 0.,
                            ..policy
                        },
                    )
                    .and_then(|common| common.as_ref().map(Solid::volume_mm3).transpose())
                    .map_err(|e| error(Some(run), e))?
                    .is_some_and(|volume| volume > 0.),
            };
            checkpoint(run, interrupt)?;
            if overlap {
                let name = |id| {
                    model::read_fence_member(document, id)
                        .ok()
                        .and_then(|member| document.get(member.output.body))
                        .map_or("Fence member", |entity| entity.name.as_str())
                };
                return Err(error(
                    Some(run),
                    format!(
                        "{} and {} overlap in material; repair post positions or infill dimensions/offsets",
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
    fn vertical_section_overlap_distinguishes_rotated_contact_and_round_gaps() {
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let rectangle = |offset: f64| {
            Section::new(
                [-offset * d, offset * d, 0.],
                [d, d],
                Footprint::Rectangle([4., 2.]),
            )
        };
        assert!(!rectangle(0.).overlaps(&rectangle(2.)));
        assert!(rectangle(0.).overlaps(&rectangle(1.999)));
        assert!(!rectangle(0.).overlaps(&rectangle(2.001)));

        let box_section = Section::new([0.; 3], [1., 0.], Footprint::Rectangle([4., 2.]));
        let circle = |x, y| Section::new([x, y, 0.], [1., 0.], Footprint::Circle(2.));
        for (x, y, overlaps) in [
            (3., 0., false),
            (2.999, 0., true),
            (2.8, 1.8, false),
            (2.6, 1.6, true),
        ] {
            let circle = circle(x, y);
            assert_eq!(box_section.overlaps(&circle), overlaps);
            assert_eq!(circle.overlaps(&box_section), overlaps);
        }
        assert!(!circle(0., 0.).overlaps(&circle(2., 0.)));
        assert!(circle(0., 0.).overlaps(&circle(1.999, 0.)));
    }

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
        let mut context = Context::new(
            session.document(),
            session.parameters(),
            GeometryBackend::default(),
        );
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
