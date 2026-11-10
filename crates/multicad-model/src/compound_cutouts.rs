//! One cutter validates every registered receiver before any generated result
//! becomes available, including during a source-scoped preview.
use crate::{GeometryBackend, ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, CompoundReceiverKind, ResolvedCompoundBox},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    RigidTransform, Solid,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct Context {
    results: BTreeMap<EntityId, Solid>,
    planar_receivers: BTreeSet<EntityId>,
}
fn checkpoint(id: EntityId, interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ModelError> {
    if interrupt.is_some_and(|f| f()) {
        return Err(error(Some(id), "Compound cutout evaluation superseded"));
    }
    Ok(())
}
fn box_solid(
    item: &ResolvedCompoundBox,
    owner: EntityId,
    policy: GeometryPolicy,
    backend: GeometryBackend,
) -> Result<Solid, ModelError> {
    let e = |e| error(Some(owner), e);
    let w = item.rectangle.width;
    let d = item.rectangle.depth;
    let height = item.top_mm - item.bottom_mm;
    let (s, c) = match backend {
        GeometryBackend::OpenCascade => item.rectangle.rotation_deg.to_radians().sin_cos(),
        GeometryBackend::Rust => match item.rectangle.rotation_deg.rem_euclid(360.) {
            0. => (0., 1.),
            90. => (1., 0.),
            180. => (0., -1.),
            270. => (-1., 0.),
            angle => angle.to_radians().sin_cos(),
        },
    };
    let origin = [
        item.rectangle.center[0] - c * w / 2. + s * d / 2.,
        item.rectangle.center[1] - s * w / 2. - c * d / 2.,
        item.bottom_mm,
    ];
    let solid = match backend {
        GeometryBackend::OpenCascade => Solid::box_named([w, d, height], owner, policy)
            .map_err(e)?
            .transformed(RigidTransform {
                rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
                translation_mm: origin,
            })
            .map_err(e)?,
        GeometryBackend::Rust => {
            let corners = [[0., 0.], [w, 0.], [w, d], [0., d]];
            let profile = CurvedProfile {
                frame: PlaneFrame {
                    origin,
                    u: [c, s, 0.],
                    v: [-s, c, 0.],
                },
                outer: (0..4)
                    .map(|i| CurvedEdge2d::Line {
                        a: corners[i],
                        b: corners[(i + 1) % 4],
                    })
                    .collect(),
                holes: Vec::new(),
            };
            let origins = std::array::from_fn::<_, 4, _>(|i| Origin {
                source: owner,
                role: format!("cutout.{}.side.{i}", item.id),
            });
            crate::rust_backend::stock_profile(owner, &profile, &origins, height, policy)?
        }
    };
    solid.with_component_origin(owner, item.id).map_err(e)
}
fn union(
    solids: &[Solid],
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    let (first, rest) = solids
        .split_first()
        .ok_or_else(|| error(Some(id), "Compound cutout has no active boxes"))?;
    if first.uses_rust_geometry() && !rest.is_empty() {
        checkpoint(id, interrupt)?;
        let operands: Vec<_> = solids.iter().collect();
        if let Some(result) = Solid::union_planar_pieces(&operands, id, policy) {
            return result.map_err(|e| error(Some(id), e));
        }
    }
    let mut result = first
        .transformed(Default::default())
        .map_err(|e| error(Some(id), e))?;
    for tool in rest {
        checkpoint(id, interrupt)?;
        result = result
            .boolean(tool, BooleanOperation::Union, policy)
            .map_err(|e| error(Some(id), e))?
            .ok_or_else(|| error(Some(id), "Empty compound cutout union"))?;
    }
    Ok(result)
}
fn overlap(a: &Solid, b: &Solid, id: EntityId, policy: GeometryPolicy) -> Result<f64, ModelError> {
    if a.uses_rust_geometry() && b.uses_rust_geometry() {
        return a
            .rust_occupied_overlap_volume(b, policy)
            .map_err(|e| error(Some(id), e));
    }
    a.boolean(b, BooleanOperation::Intersect, policy)
        .map_err(|e| error(Some(id), e))?
        .map_or(Ok(0.), |solid| {
            solid.volume_mm3().map_err(|e| error(Some(id), e))
        })
}

fn subtract(
    target: &Solid,
    tools: &[Solid],
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<(Option<Solid>, bool), ModelError> {
    let result = if target.uses_rust_geometry() {
        match target.rust_subtract_orthogonal_tools(tools, policy) {
            Err(cause) if cause.kind == GeometryErrorKind::Unsupported => target
                .rust_subtract_planar_tools(tools, id, policy)
                .map(|solid| (solid, true)),
            result => result.map(|solid| (solid, false)),
        }
    } else {
        target
            .boolean(&tools[0], BooleanOperation::Subtract, policy)
            .map(|solid| (solid, false))
    };
    result.map_err(|e| error(Some(id), e))
}
impl Context {
    pub(crate) fn take(&mut self, id: EntityId) -> Result<Solid, ModelError> {
        self.results.remove(&id).ok_or_else(|| {
            error(
                Some(id),
                "Complete compound receiver proof is missing; repair its source",
            )
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        &mut self,
        document: &Document,
        parameters: &EvaluatedParameters,
        id: EntityId,
        solids: &BTreeMap<EntityId, Solid>,
        policy: GeometryPolicy,
        backend: GeometryBackend,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Solid, ModelError> {
        checkpoint(id, interrupt)?;
        let source = model::read_compound_cutout(document, id).map_err(|e| error(Some(id), e))?;
        let resolved = model::resolve_compound_cutout(document, parameters, &source)
            .map_err(|e| error(Some(id), e))?;
        let boxes = resolved
            .boxes
            .iter()
            .map(|item| box_solid(item, id, policy, backend))
            .collect::<Result<Vec<_>, _>>()?;
        let cutter = union(&boxes, id, policy, interrupt)?;
        if cutter.solid_count().map_err(|e| error(Some(id), e))? != 1 {
            return Err(error(
                Some(id),
                "Cutout boxes must overlap into one connected cutting volume",
            ));
        }
        let mut hits = vec![false; boxes.len()];
        let mut results = BTreeMap::new();
        let mut planar_receivers = BTreeSet::new();
        for receiver in &resolved.receivers {
            checkpoint(id, interrupt)?;
            let member = receiver.receiver.feature;
            let target = solids
                .get(&receiver.receiver.target.producer)
                .ok_or_else(|| {
                    error(
                        Some(member),
                        "Cutout receiver input is missing; repair its frozen producer",
                    )
                })?;
            // A planar difference publishes an exact boundary rather than a
            // positive cell decomposition. Previous and current cutters are
            // disjoint by the receiver proof, so replay their union on the
            // original receiver when extending a planar cutout history.
            let from_base = self
                .planar_receivers
                .contains(&receiver.receiver.target.producer);
            let material = if from_base {
                solids.get(&receiver.base.feature).ok_or_else(|| {
                    error(
                        Some(member),
                        "Original compound cutout receiver is unavailable",
                    )
                })?
            } else {
                target
            };
            let components = material
                .solid_components_interruptible(policy, interrupt)
                .map_err(|e| error(Some(id), e))?;
            for (i, tool) in boxes.iter().enumerate() {
                if hits[i] {
                    continue;
                }
                if overlap(material, tool, id, policy)? > 1e-6 {
                    hits[i] = true;
                }
            }
            if receiver.base.kind == CompoundReceiverKind::Wall {
                let wall = model::resolve_wall(
                    document,
                    parameters,
                    &model::read_wall(document, receiver.base.feature)
                        .map_err(|e| error(Some(id), e))?,
                )
                .map_err(|e| error(Some(id), e))?;
                let plan = wall.plan().map_err(|e| error(Some(id), e))?;
                let wall_id = receiver.base.feature;
                let mut previous = match backend {
                    GeometryBackend::OpenCascade => {
                        crate::walls::cut_tools(&wall, &plan, wall_id, policy, interrupt)?
                    }
                    GeometryBackend::Rust => crate::walls::cut_tools_with_backend(
                        &wall, &plan, wall_id, policy, interrupt, backend,
                    )?,
                };
                for previous_id in &receiver.base.previous {
                    previous.push(
                        solids
                            .get(previous_id)
                            .ok_or_else(|| {
                                error(Some(id), "Earlier compound cutter is unavailable")
                            })?
                            .transformed(Default::default())
                            .map_err(|e| error(Some(id), e))?,
                    );
                }
                for old in previous {
                    if overlap(&cutter, &old, id, policy)? > 1e-6 {
                        return Err(error(
                            Some(id),
                            "Compound cutout overlaps an existing wall opening or cutout; edit that source instead",
                        ));
                    }
                }
            }
            let before = target.volume_mm3().map_err(|e| error(Some(id), e))?;
            let mut cutters = Vec::new();
            if from_base {
                for previous in receiver.base.previous.iter().rev() {
                    cutters.push(solids.get(previous).ok_or_else(|| {
                        error(Some(id), "Earlier compound cutter is unavailable")
                    })?);
                }
            }
            cutters.push(&cutter);
            let mut pieces = vec![];
            for component in components {
                checkpoint(id, interrupt)?;
                let materials = model::wall_face_materials(
                    document,
                    &component
                        .topology_snapshot(member, 0)
                        .map_err(|e| error(Some(id), e))?,
                )
                .map_err(|e| error(Some(id), e))?;
                let layer = materials.values().next();
                if layer.is_some_and(|first| {
                    materials
                        .values()
                        .any(|other| other.wall != first.wall || other.layer != first.layer)
                }) {
                    return Err(error(
                        Some(id),
                        "Receiving wall component has ambiguous layer ownership",
                    ));
                }
                let tools = cutters
                    .iter()
                    .map(|cutter| {
                        let tool = cutter.transformed(Default::default())?;
                        match layer {
                            Some(layer) => tool.with_component_origin(layer.wall, layer.layer),
                            None => Ok(tool),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| error(Some(id), e))?;
                let (cut, planar) = subtract(&component, &tools, member, policy)?;
                let cut = cut.ok_or_else(|| {
                    error(
                        Some(id),
                        "Cutout removes an entire receiving solid; change its boxes",
                    )
                })?;
                if from_base || planar {
                    planar_receivers.insert(member);
                }
                if cut.solid_count().map_err(|e| error(Some(id), e))? != 1 {
                    return Err(error(
                        Some(id),
                        "Cutout would split a receiving wall/slab solid; change its boxes",
                    ));
                }
                pieces.push(cut);
            }
            let result = if pieces.len() == 1 {
                pieces.pop().unwrap()
            } else {
                Solid::compound_named(&pieces.iter().collect::<Vec<_>>(), member, policy)
                    .map_err(|e| error(Some(id), e))?
            };
            let removed = before - result.volume_mm3().map_err(|e| error(Some(id), e))?;
            if removed <= 1e-6 {
                return Err(error(
                    Some(member),
                    "Chosen receiver is not cut by these boxes; repair the explicit target or extent",
                ));
            }
            if receiver.base.kind == CompoundReceiverKind::Slab {
                let projected = resolved
                    .boxes
                    .iter()
                    .filter(|item| {
                        item.top_mm > receiver.bottom_mm + 1e-6
                            && item.bottom_mm < receiver.top_mm - 1e-6
                    })
                    .map(|item| {
                        let mut item = item.clone();
                        item.bottom_mm = receiver.bottom_mm;
                        item.top_mm = receiver.top_mm;
                        box_solid(&item, id, policy, backend)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let through = union(&projected, id, policy, interrupt)?;
                let expected = overlap(material, &through, id, policy)?;
                if (expected - removed).abs() > 1e-3_f64.max(expected * 1e-9) {
                    return Err(error(
                        Some(member),
                        "Slab cutout must pass completely through its thickness; adjust box heights",
                    ));
                }
            }
            model::wall_face_materials(
                document,
                &result
                    .topology_snapshot(member, 0)
                    .map_err(|e| error(Some(id), e))?,
            )
            .map_err(|e| error(Some(id), e))?;
            results.insert(member, result);
        }
        if hits.contains(&false) {
            return Err(error(
                Some(id),
                "Every cutout box must intersect at least one explicitly chosen receiver",
            ));
        }
        self.results.extend(results);
        self.planar_receivers.extend(planar_receivers);
        Ok(cutter)
    }
}
