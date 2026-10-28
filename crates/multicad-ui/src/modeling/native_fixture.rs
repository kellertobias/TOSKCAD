//! Tests use the production native model worker and detached geometry receipt.
use crate::viewport::{self, Scene, SceneBackend, SceneReply, SceneStamp};
use multicad_core::{
    document::Document,
    model::{EvaluationOptions, ModelEvaluationInfo},
    project_file::ProjectResources,
};
use multicad_model::{Compiler, ModelError, ModelSnapshot, Runtime};
use std::sync::Arc;
pub(crate) fn scene(
    _document: &Document,
    model: &ModelSnapshot,
    generation: u64,
) -> Result<Arc<Scene>, ModelError> {
    let make = || -> Result<Scene, String> {
        let geometries = model
            .shapes
            .iter()
            .map(|(id, shape)| {
                viewport::Geometry::new(
                    *id,
                    shape.mesh.vertices.as_chunks::<3>().0.to_vec(),
                    shape.mesh.indices.clone(),
                    shape.mesh.face_indices.clone(),
                    shape.topology.elements.clone(),
                )?
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
                )
                .map(Arc::new)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut instances: Vec<_> = model
            .bodies
            .iter()
            .map(|(body, producer)| viewport::Instance {
                id: *body,
                geometry: *producer,
                transform: Default::default(),
                color: [128, 154, 204, 255],
            })
            .collect();
        for occurrence in model.occurrences.values() {
            instances.push(viewport::Instance {
                id: occurrence.instance,
                geometry: occurrence.producer,
                transform: viewport::Transform::from_rigid_pose(occurrence.pose)?,
                color: [128, 154, 204, 255],
            });
        }
        let mut native_bounds: std::collections::BTreeMap<_, _> = model
            .shapes
            .iter()
            .map(|(id, shape)| (*id, [shape.bounds.min, shape.bounds.max]))
            .collect();
        for (body, producer) in &model.body_sources {
            let bounds = model
                .shapes
                .get(producer)
                .ok_or("Native Part bounds have no current producer")?
                .bounds;
            native_bounds.insert(*body, [bounds.min, bounds.max]);
        }
        for occurrence in model.occurrences.values() {
            native_bounds.insert(
                occurrence.instance,
                [occurrence.bounds.min, occurrence.bounds.max],
            );
        }
        let scene = Scene::new(
            SceneStamp {
                document: model.stamp.document,
                revision: model.stamp.revision,
                generation,
            },
            geometries,
            instances,
        )?
        .with_reference_aliases(model.body_sources.clone())?;
        let scene = match &model.scope {
            multicad_model::EvaluationScope::Full => scene,
            multicad_model::EvaluationScope::Sources(roots) => {
                scene.with_source_scope(roots.clone())?
            }
        };
        let scene = scene
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                source_hash: model.stamp.source_hash,
                consumed_bodies: model.consumed_bodies.clone(),
                cut_targets: model.cut_targets.clone(),
            }))?
            .with_native_bounds(model.stamp.source_hash, native_bounds)?;
        if !scene.is_full_model() {
            return Ok(scene);
        }
        let scene = scene.with_assembly_evaluation(Arc::new(
            multicad_core::model::AssemblyEvaluationInfo {
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
            },
        ))?;
        scene.with_transfer_evaluation(Arc::new(multicad_core::model::TransferEvaluationInfo {
            source_hash: model.stamp.source_hash,
            counts: model.transfer_counts.clone(),
        }))
    };
    make().map(Arc::new).map_err(|message| ModelError {
        entity: None,
        message,
    })
}
pub(crate) struct NativeBackend {
    runtime: Runtime<Arc<Scene>>,
    expected: Option<(u64, SceneStamp)>,
}
impl NativeBackend {
    pub fn new() -> Self {
        Self {
            runtime: Runtime::with_publisher(Compiler::default(), scene).unwrap(),
            expected: None,
        }
    }
}
impl SceneBackend for NativeBackend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.request_evaluation(document, &Default::default(), stamp, &Default::default())
    }
    fn request_evaluation(
        &mut self,
        document: &Document,
        _resources: &ProjectResources,
        stamp: SceneStamp,
        options: &EvaluationOptions,
    ) -> Result<(), String> {
        let generation =
            self.runtime
                .request_evaluation(document, stamp.revision, stamp.generation, options)?;
        self.expected = Some((generation, stamp));
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        let (expected, stamp) = self.expected?;
        let reply = self.runtime.poll()?;
        if reply.generation != expected {
            return None;
        }
        self.expected = None;
        Some(SceneReply {
            stamp,
            result: reply.result.map_err(|e| e.to_string()),
        })
    }
}
