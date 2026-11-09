//! Production composition: exact native evaluation to immutable UI scenes.
use multicad_core::{
    document::{Document, EntityData, EntityId},
    materials::Resolver,
    model::{EvaluationOptions, ModelEvaluationInfo},
    project_file::ProjectResources,
    references::{TopologyElement, TopologyKind},
};
use multicad_model::{Compiler, ModelError, ModelSnapshot, Runtime};
use multicad_ui::viewport::{self, SceneBackend, SceneReply, SceneStamp};
use std::{collections::BTreeMap, sync::Arc};
#[cfg(test)]
#[path = "modeling/architecture_authoring_tests.rs"]
mod architecture_authoring_tests;
#[path = "modeling/architecture_plan.rs"]
mod architecture_plan;
#[cfg(test)]
#[path = "modeling/tests.rs"]
mod tests;

pub struct Backend {
    runtime: Result<Runtime<Arc<viewport::Scene>>, String>,
    expected: Option<(u64, SceneStamp)>,
}
impl Backend {
    fn runtime() -> Result<Runtime<Arc<viewport::Scene>>, String> {
        Runtime::with_model_and_snapshot(
            Compiler::default(),
            |document, model, snapshot, generation| {
                let mut published = scene(document, snapshot, generation)?;
                if published.is_full_model() {
                    let plan = architecture_plan::build(document, model);
                    let published = Arc::get_mut(&mut published)
                        .expect("publication is not shared before return");
                    published.set_sheet_faces(model.sheet_authoring_faces(document));
                    published.set_sheet_bend_schedules(
                        model
                            .sheet_bend_schedules(document)
                            .into_iter()
                            .map(|(part, records)| {
                                let bends = records
                                    .into_iter()
                                    .enumerate()
                                    .map(|(order, record)| multicad_ui::sheet_pattern::Bend {
                                        order,
                                        line: record.line,
                                        direction: record.direction,
                                        angle_deg: record.angle_deg,
                                        inside_radius_mm: record.inside_radius_mm,
                                        allowance_mm: record.allowance_mm,
                                    })
                                    .collect();
                                (part, bends)
                            })
                            .collect(),
                    );
                    published
                        .set_architecture_plan(plan)
                        .map_err(|message| ModelError {
                            entity: None,
                            message,
                        })?;
                    if multicad_core::model::welds(document).next().is_some()
                        && let Ok(welds) = model.weld_schedule(document)
                    {
                        published.set_weld_markers(
                            welds
                                .into_iter()
                                .map(|weld| viewport::WeldMarker {
                                    mark: weld.mark,
                                    members: weld.members,
                                    position: weld.joint_plane.0,
                                    normal: weld.joint_plane.1,
                                })
                                .collect(),
                        );
                    }
                }
                Ok(published)
            },
        )
    }
    pub fn new() -> Self {
        Self {
            runtime: Self::runtime(),
            expected: None,
        }
    }
    /// Each backend publishes independently while exact geometry remains on
    /// one app-owned native worker across preview and canonical requests.
    pub fn factory() -> Result<viewport::BackendFactory, String> {
        let owner = Self::new().runtime?;
        Ok(Arc::new(move || {
            Box::new(Self {
                runtime: owner.fork(),
                expected: None,
            })
        }))
    }
    fn evaluate(
        &mut self,
        document: &Document,
        stamp: SceneStamp,
        options: &EvaluationOptions,
    ) -> Result<(), String> {
        if stamp.document != document.id() {
            return Err("Model scene request belongs to another document".into());
        }
        let runtime = self.runtime.as_mut().map_err(|e| e.clone())?;
        let generation =
            runtime.request_evaluation(document, stamp.revision, stamp.generation, options)?;
        self.expected = Some((generation, stamp));
        Ok(())
    }
}
impl SceneBackend for Backend {
    fn cancel(&mut self) {
        self.expected = None;
        if let Ok(runtime) = self.runtime.as_mut() {
            runtime.cancel();
        }
    }
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.evaluate(document, stamp, &EvaluationOptions::default())
    }
    fn request_evaluation(
        &mut self,
        document: &Document,
        _resources: &ProjectResources,
        stamp: SceneStamp,
        options: &EvaluationOptions,
    ) -> Result<(), String> {
        self.evaluate(document, stamp, options)
    }
    fn request_sources(
        &mut self,
        document: &Document,
        _resources: &ProjectResources,
        stamp: SceneStamp,
        roots: &[multicad_core::document::EntityId],
    ) -> Result<(), String> {
        if stamp.document != document.id() {
            return Err("Source inspection belongs to another document".into());
        }
        let runtime = self.runtime.as_mut().map_err(|e| e.clone())?;
        let generation =
            runtime.request_sources(document, stamp.revision, stamp.generation, roots)?;
        self.expected = Some((generation, stamp));
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        let (expected, stamp) = self.expected?;
        let reply = self.runtime.as_mut().ok()?.poll()?;
        if reply.generation != expected {
            return None;
        }
        self.expected = None;
        let result = reply.result.map_err(|e| e.to_string()).and_then(|scene| {
            if scene.stamp() != stamp {
                return Err("Native model publication stamp does not match its request".into());
            }
            Ok(scene)
        });
        Some(SceneReply { stamp, result })
    }
}
/// Native aid inspections as scene review data.
fn aid_reviews(
    inspections: &Result<Vec<multicad_model::AidInspection>, String>,
) -> Result<Vec<viewport::AidReview>, String> {
    let list = inspections.as_ref().map_err(Clone::clone)?;
    Ok(list
        .iter()
        .map(|a| viewport::AidReview {
            feature: a.feature,
            part: a.part,
            target: a.target.part,
            touches_target: a.target.relation != multicad_kernel::SolidPairRelation::Separated,
            target_distance_mm: a.target.distance_mm,
            crowded: a.crowded.clone(),
            issues: a.issues.clone(),
            contact_issues: a.contact_issues.clone(),
            bounds_mm: a.bounds_mm,
            exact_contact: a.exact_contact.map(|c| viewport::ExactContact {
                point_mm: c.point_mm,
                angle_deg: c.angle_deg,
            }),
        })
        .collect())
}

/// Follow the exact member origin at each level of the native compound.
/// Keeping the aggregate mesh preserves its face ordinals and pick identity.
fn assembly_face_part(
    document: &Document,
    mut component: EntityId,
    face: &TopologyElement,
) -> Option<EntityId> {
    for _ in 0..=32 {
        let instances = match &document.get(component)?.data {
            EntityData::Part { .. } => return Some(component),
            EntityData::Assembly { instances } => instances,
            _ => return None,
        };
        let mut members = face
            .origins
            .iter()
            .filter(|origin| origin.source == component)
            .filter_map(|origin| origin.role.strip_prefix("assembly/member/"));
        let member = EntityId::parse(members.next()?).ok()?;
        if members.next().is_some() || !instances.contains(&member) {
            return None;
        }
        let EntityData::Instance {
            component: next, ..
        } = document.get(member)?.data
        else {
            return None;
        };
        component = next;
    }
    None
}

fn scene(
    document: &Document,
    model: &ModelSnapshot,
    generation: u64,
) -> Result<Arc<viewport::Scene>, ModelError> {
    let invalid = |message: String| ModelError {
        entity: None,
        message,
    };
    let materials = Resolver::new(document).map_err(|e| invalid(e.to_string()))?;
    let mut part_colors = BTreeMap::new();
    let mut part_color = |part| -> Result<[u8; 4], String> {
        if let std::collections::btree_map::Entry::Vacant(entry) = part_colors.entry(part) {
            entry.insert(
                materials
                    .resolve(part)
                    .map_err(|e| e.to_string())?
                    .appearance
                    .color_rgba,
            );
        }
        Ok(part_colors[&part])
    };
    let geometries = model
        .shapes
        .iter()
        .map(|(id, shape)| {
            let face_materials =
                multicad_core::model::wall_face_materials(document, &shape.topology)
                    .map_err(|e| e.to_string())?;
            let mut palette = std::collections::BTreeMap::new();
            let mut face_colors = std::collections::BTreeMap::new();
            for (face, assignment) in face_materials {
                let material = assignment.material;
                if let std::collections::btree_map::Entry::Vacant(entry) = palette.entry(material) {
                    entry.insert(
                        materials
                            .resolve_material(material)
                            .map_err(|e| e.to_string())?
                            .appearance
                            .color_rgba,
                    );
                }
                face_colors.insert(face, palette[&material]);
            }
            if matches!(
                document.get(*id).map(|e| &e.data),
                Some(EntityData::Assembly { .. })
            ) {
                for face in shape
                    .topology
                    .elements
                    .iter()
                    .filter(|face| face.key.kind == TopologyKind::Face)
                {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        face_colors.entry(face.key.ordinal)
                        && let Some(part) = assembly_face_part(document, *id, face)
                    {
                        entry.insert(part_color(part)?);
                    }
                }
            }
            let mesh = &shape.mesh;
            let geometry = viewport::Geometry::new(
                *id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices.clone(),
                mesh.face_indices.clone(),
                shape.topology.elements.clone(),
            )?
            .with_face_colors(face_colors)?
            .with_topology_display(
                shape
                    .edges
                    .iter()
                    .map(|edge| viewport::DisplayEdge {
                        index: edge.index,
                        points_mm: edge.points_mm.clone(),
                    })
                    .collect(),
                shape
                    .vertices
                    .iter()
                    .map(|point| viewport::DisplayPoint {
                        index: point.index,
                        position_mm: point.position_mm,
                    })
                    .collect(),
                0.1,
            )?;
            Ok(Arc::new(geometry))
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(invalid)?;
    let mut instances: Vec<_> = model
        .bodies
        .iter()
        .map(|(body, producer)| {
            let color = part_color(*body).map_err(invalid)?;
            Ok(viewport::Instance {
                id: *body,
                geometry: *producer,
                transform: Default::default(),
                color,
            })
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    for occurrence in model.occurrences.values() {
        // Aggregate faces carry their member colors. The instance color is the
        // fallback for any face without a resolved member.
        let color = if matches!(
            document
                .get(occurrence.component)
                .map(|entity| &entity.data),
            Some(EntityData::Assembly { .. })
        ) {
            multicad_core::materials::Appearance::default().color_rgba
        } else {
            part_color(occurrence.component).map_err(invalid)?
        };
        instances.push(viewport::Instance {
            id: occurrence.instance,
            geometry: occurrence.producer,
            transform: viewport::Transform::from_rigid_pose(occurrence.pose).map_err(invalid)?,
            color,
        });
    }
    if let multicad_model::EvaluationScope::Sources(roots) = &model.scope {
        for root in roots {
            if model.shapes.contains_key(root)
                && !instances.iter().any(|instance| instance.geometry == *root)
            {
                instances.push(viewport::Instance {
                    id: *root,
                    geometry: *root,
                    transform: Default::default(),
                    color: [145, 173, 195, 255],
                });
            }
        }
    }
    let stamp = SceneStamp {
        document: model.stamp.document,
        revision: model.stamp.revision,
        generation,
    };
    let mut native_bounds: std::collections::BTreeMap<_, _> = model
        .shapes
        .iter()
        .map(|(id, shape)| (*id, [shape.bounds.min, shape.bounds.max]))
        .collect();
    for (body, producer) in &model.body_sources {
        let bounds = model
            .shapes
            .get(producer)
            .ok_or_else(|| invalid("Native Part bounds have no current producer".into()))?
            .bounds;
        native_bounds.insert(*body, [bounds.min, bounds.max]);
    }
    for occurrence in model.occurrences.values() {
        native_bounds.insert(
            occurrence.instance,
            [occurrence.bounds.min, occurrence.bounds.max],
        );
    }
    viewport::Scene::new(stamp, geometries, instances)
        .and_then(|scene| match &model.scope {
            multicad_model::EvaluationScope::Full => Ok(scene),
            multicad_model::EvaluationScope::Sources(roots) => {
                scene.with_source_scope(roots.clone())
            }
        })
        .and_then(|scene| scene.with_reference_aliases(model.body_sources.clone()))
        .and_then(|scene| {
            scene.with_model_evaluation(Arc::new(ModelEvaluationInfo {
                source_hash: model.stamp.source_hash,
                consumed_bodies: model.consumed_bodies.clone(),
                cut_targets: model.cut_targets.clone(),
                screw_connection_checks: model.screw_connection_checks.clone(),
            }))
        })
        .and_then(|scene| {
            scene.with_native_reference_report(model.stamp.source_hash, model.references.clone())
        })
        .and_then(|scene| {
            if !scene.is_full_model() {
                return Ok(scene);
            }
            scene.with_assembly_evaluation(Arc::new(multicad_core::model::AssemblyEvaluationInfo {
                source_hash: model.stamp.source_hash,
                leaves: model
                    .leaf_occurrences
                    .values()
                    .map(|leaf| multicad_core::model::AssemblyLeafReceipt {
                        path: leaf.path.clone(),
                        component: leaf.component,
                        producer: leaf.producer,
                        pose: leaf.pose,
                    })
                    .collect(),
            }))
        })
        .and_then(|scene| {
            if !scene.is_full_model() {
                return Ok(scene);
            }
            scene.with_transfer_evaluation(Arc::new(multicad_core::model::TransferEvaluationInfo {
                source_hash: model.stamp.source_hash,
                counts: model.transfer_counts.clone(),
            }))
        })
        .and_then(|scene| scene.with_native_bounds(model.stamp.source_hash, native_bounds))
        .and_then(|scene| scene.with_envelope_boxes(model.envelope_boxes.clone()))
        .and_then(|scene| scene.with_guide_outlines(model.guide_outlines.clone()))
        .and_then(|scene| {
            scene.with_fdm_reviews(model.stamp.source_hash, aid_reviews(&model.fdm_aids))
        })
        .map(Arc::new)
        .map_err(invalid)
}

/// Packaged-runtime check of the canonical source → native model → display
/// adapter path. The resulting editable project is also usable for UI review.
pub fn smoke(directory: &std::path::Path) -> Result<(), String> {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        constraint_sketch::source::Source,
        document::Document,
        model::{self, ExtrudeRequest},
        project_file::{self, ProjectFile},
        sketch::Datum,
        sketch_regions,
    };
    let actor = Actor::local(Channel::CommandLine);
    let (sketch, edits) =
        Source::create(Datum::Xy, "Shared profiles").map_err(|e| e.to_string())?;
    let mut document = Document::new("Parametric solids review")
        .map_err(|e| e.to_string())?
        .edited(&edits)
        .map_err(|e| e.to_string())?;
    let mut source = Source::read(&document, sketch)
        .map_err(|e| e.to_string())?
        .0;
    source
        .add_rectangle([0., 0.], [80., 60.])
        .map_err(|e| e.to_string())?;
    source
        .add_rectangle([100., 0.], [140., 40.])
        .map_err(|e| e.to_string())?;
    document = document
        .edited(
            &source
                .replace(&document, sketch)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let regions = sketch_regions::extract(&source);
    if !regions.diagnostics.is_empty() || regions.regions.len() != 2 {
        return Err("Canonical smoke profiles did not close".into());
    }
    let mut session = Session::new(document).map_err(|e| e.to_string())?;
    let request = ExtrudeRequest::new(
        sketch,
        regions.regions.iter().map(|r| r.intent()).collect(),
        Default::default(),
    );
    let prepared =
        model::prepare_extrude(&session, 0, actor, &request).map_err(|e| e.to_string())?;
    let exact = Compiler::default()
        .compile(prepared.prepared.document(), 0)
        .map_err(|e| e.to_string())?;
    if exact.snapshot().bodies.len() != 2
        || (exact
            .snapshot()
            .shapes
            .values()
            .map(|s| s.volume_mm3)
            .sum::<f64>()
            - 64000.)
            .abs()
            > 1e-5
    {
        return Err("Canonical native model volume/identity smoke check failed".into());
    }
    // The same exact candidate is committed only after native validation.
    session
        .commit(prepared.prepared)
        .map_err(|e| e.to_string())?;
    let exact = Compiler::default()
        .compile(session.document(), session.revision())
        .map_err(|e| e.to_string())?;
    let display = scene(session.document(), exact.snapshot(), 1).map_err(|e| e.to_string())?;
    if display.reference_instances().len() != 2 {
        return Err("Native model display lost a Part".into());
    }
    for (index, producer) in exact.snapshot().bodies.values().enumerate() {
        exact
            .solid(*producer)
            .ok_or("Model lost an exact producer")?
            .write_step(&directory.join(format!("parametric-body-{}.step", index + 1)))
            .map_err(|e| e.to_string())?;
    }
    let project = ProjectFile::new(session.document().clone())?;
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default())?;
    std::fs::write(directory.join("parametric-solids.multicad"), bytes)
        .map_err(|e| e.to_string())?;
    std::fs::write(directory.join("parametric-solids.txt"),"Canonical shared sketch; two independent native B-reps; checked preview/commit; viewport adapter; editable project and exact STEP bodies: passed\n").map_err(|e|e.to_string())?;
    Ok(())
}
