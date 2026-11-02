//! One cutter validates every registered receiver before any generated result
//! becomes available, including during a source-scoped native preview.
use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, CompoundReceiverKind, ResolvedCompoundBox},
    parameters::EvaluatedParameters,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Context {
    results: BTreeMap<EntityId, Solid>,
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
) -> Result<Solid, ModelError> {
    let e = |e| error(Some(owner), e);
    let (s, c) = item.rectangle.rotation_deg.to_radians().sin_cos();
    let w = item.rectangle.width;
    let d = item.rectangle.depth;
    Solid::box_named([w, d, item.top_mm - item.bottom_mm], owner, policy)
        .map_err(e)?
        .transformed(RigidTransform {
            rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
            translation_mm: [
                item.rectangle.center[0] - c * w / 2. + s * d / 2.,
                item.rectangle.center[1] - s * w / 2. - c * d / 2.,
                item.bottom_mm,
            ],
        })
        .map_err(e)?
        .with_component_origin(owner, item.id)
        .map_err(e)
}
fn union(
    solids: Vec<Solid>,
    id: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    let mut iter = solids.into_iter();
    let mut result = iter
        .next()
        .ok_or_else(|| error(Some(id), "Compound cutout has no active boxes"))?;
    for tool in iter {
        checkpoint(id, interrupt)?;
        result = result
            .boolean(&tool, BooleanOperation::Union, policy)
            .map_err(|e| error(Some(id), e))?
            .ok_or_else(|| error(Some(id), "Empty compound cutout union"))?;
    }
    Ok(result)
}
fn overlap(a: &Solid, b: &Solid, id: EntityId, policy: GeometryPolicy) -> Result<f64, ModelError> {
    a.boolean(b, BooleanOperation::Intersect, policy)
        .map_err(|e| error(Some(id), e))?
        .map_or(Ok(0.), |solid| {
            solid.volume_mm3().map_err(|e| error(Some(id), e))
        })
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
    pub(crate) fn build(
        &mut self,
        document: &Document,
        parameters: &EvaluatedParameters,
        id: EntityId,
        solids: &BTreeMap<EntityId, Solid>,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Solid, ModelError> {
        checkpoint(id, interrupt)?;
        let source = model::read_compound_cutout(document, id).map_err(|e| error(Some(id), e))?;
        let resolved = model::resolve_compound_cutout(document, parameters, &source)
            .map_err(|e| error(Some(id), e))?;
        let boxes = resolved
            .boxes
            .iter()
            .map(|item| box_solid(item, id, policy))
            .collect::<Result<Vec<_>, _>>()?;
        let cutter = union(
            resolved
                .boxes
                .iter()
                .map(|item| box_solid(item, id, policy))
                .collect::<Result<Vec<_>, _>>()?,
            id,
            policy,
            interrupt,
        )?;
        if cutter.solid_count().map_err(|e| error(Some(id), e))? != 1 {
            return Err(error(
                Some(id),
                "Cutout boxes must overlap into one connected cutting volume",
            ));
        }
        let mut hits = vec![false; boxes.len()];
        let mut results = BTreeMap::new();
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
            for (i, tool) in boxes.iter().enumerate() {
                if overlap(target, tool, id, policy)? > 1e-6 {
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
                let mut previous = crate::walls::cut_tools(
                    &wall,
                    &plan,
                    receiver.base.feature,
                    policy,
                    interrupt,
                )?;
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
            let mut pieces = vec![];
            for component in target
                .solid_components_interruptible(policy, interrupt)
                .map_err(|e| error(Some(id), e))?
            {
                checkpoint(id, interrupt)?;
                let materials = model::wall_face_materials(
                    document,
                    &component
                        .topology_snapshot(member, 0)
                        .map_err(|e| error(Some(id), e))?,
                )
                .map_err(|e| error(Some(id), e))?;
                let mut tool = cutter
                    .transformed(Default::default())
                    .map_err(|e| error(Some(id), e))?;
                if let Some(first) = materials.values().next() {
                    if materials
                        .values()
                        .any(|other| other.wall != first.wall || other.layer != first.layer)
                    {
                        return Err(error(
                            Some(id),
                            "Receiving wall component has ambiguous layer ownership",
                        ));
                    }
                    tool = tool
                        .with_component_origin(first.wall, first.layer)
                        .map_err(|e| error(Some(id), e))?;
                }
                let cut = component
                    .boolean(&tool, BooleanOperation::Subtract, policy)
                    .map_err(|e| error(Some(id), e))?
                    .ok_or_else(|| {
                        error(
                            Some(id),
                            "Cutout removes an entire receiving solid; change its boxes",
                        )
                    })?;
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
                        box_solid(&item, id, policy)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let through = union(projected, id, policy, interrupt)?;
                let expected = overlap(target, &through, id, policy)?;
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
        Ok(cutter)
    }
}
