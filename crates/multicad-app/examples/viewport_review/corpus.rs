//! Native viewport acceptance assets, deliberately separate from production
//! feature providers and solid-creation commands.
use multicad_core::document::*;
use multicad_kernel::*;
use multicad_ui::viewport::{
    self, DisplayQuality, Refinement, SceneBackend, SceneReply, SceneStamp,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};
pub fn document(mixed: bool) -> Result<Document, String> {
    let count = if mixed { 100 } else { 1000 };
    let mut edits = vec![];
    let material = Entity::new(
        "Workshop steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [151, 173, 195, 255],
            section_hatch: None,
        },
    );
    edits.push(Mutation::Insert(material.clone()));
    let mut parts = vec![];
    for i in 0..if mixed { 100 } else { 1 } {
        let feature = Entity::new(
            format!("Corpus source {i}"),
            EntityData::Feature {
                definition: Definition {
                    type_id: "viewport.review.primitive".into(),
                    schema_version: 1,
                    data: serde_json::json!({"kind":if mixed {i%3}else {0},"width":30.+(i%7) as f64,"depth":22.,"height":12.+(i%3) as f64*4.}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let part = Entity::new(
            format!("Corpus part {i}"),
            EntityData::Part {
                features: vec![feature.id],
                material: Some(material.id),
                tags: BTreeSet::from(["viewport-review".into()]),
            },
        );
        edits.extend([Mutation::Insert(feature), Mutation::Insert(part.clone())]);
        parts.push(part.id);
    }
    for i in 0..count {
        let columns = if mixed { 10 } else { 32 };
        edits.push(Mutation::Insert(Entity::new(
            format!("Part {:04}", i + 1),
            EntityData::Instance {
                component: parts[if mixed { i } else { 0 }],
                placement: Placement {
                    translation_mm: [(i % columns) as f64 * 45., (i / columns) as f64 * 40., 0.],
                    ..Default::default()
                },
            },
        )));
    }
    Document::new(format!(
        "Viewport review - {count} {}",
        if mixed { "mixed parts" } else { "instances" }
    ))
    .map_err(|e| e.to_string())?
    .edited(&edits)
    .map_err(|e| e.to_string())
}
struct Pending {
    stamp: SceneStamp,
    job: GeometryJob<GeometrySnapshot>,
    ids: Vec<EntityId>,
    instances: Vec<viewport::Instance>,
    deflection_mm: f64,
    reuse: BTreeMap<EntityId, Arc<viewport::Geometry>>,
}
type SceneBuild = (SceneStamp, Receiver<Result<Arc<viewport::Scene>, String>>);
pub struct Backend {
    worker: Result<GeometryWorker, String>,
    pending: Option<Pending>,
    build: Option<SceneBuild>,
    generation: Arc<AtomicU64>,
    keys: BTreeMap<EntityId, SolidKey>,
    curved: BTreeSet<EntityId>,
    scene: Option<Arc<viewport::Scene>>,
}
impl Backend {
    pub fn new() -> Self {
        Self {
            worker: GeometryWorker::new().map_err(|e| e.to_string()),
            pending: None,
            build: None,
            generation: Arc::new(AtomicU64::new(0)),
            keys: BTreeMap::new(),
            curved: BTreeSet::new(),
            scene: None,
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Release);
    }
}
impl SceneBackend for Backend {
    fn request(&mut self, doc: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.pending = None;
        self.build = None;
        self.keys.clear();
        self.curved.clear();
        self.scene = None;
        self.generation.store(stamp.generation, Ordering::Release);
        let mut ids = vec![];
        let mut operations = vec![];
        for entity in doc.entities() {
            if let EntityData::Feature {
                definition,
                suppressed: false,
                ..
            } = &entity.data
            {
                if definition.type_id != "viewport.review.primitive"
                    || definition.schema_version != 1
                {
                    return Err("This review backend only reads viewport corpus fixtures".into());
                }
                let number = |field| {
                    definition
                        .data
                        .get(field)
                        .and_then(|v| v.as_f64())
                        .ok_or_else(|| format!("Missing corpus field {field}"))
                };
                let w = number("width")?;
                let d = number("depth")?;
                let h = number("height")?;
                let operation = match definition.data.get("kind").and_then(|v| v.as_u64()) {
                    Some(0) => GeometryOperation::NamedBox {
                        size_mm: [w, d, h],
                        feature: entity.id,
                    },
                    Some(kind @ 1..=2) => {
                        let profile = if kind == 1 {
                            Profile {
                                frame: Default::default(),
                                outer: vec![ProfileEdge::Circle {
                                    center: [w / 2., d / 2.],
                                    radius_mm: w / 2.,
                                }],
                                holes: vec![vec![ProfileEdge::Circle {
                                    center: [w / 2., d / 2.],
                                    radius_mm: w / 5.,
                                }]],
                            }
                        } else {
                            let mut profile = Profile::rectangle(w, d);
                            profile.holes.push(vec![ProfileEdge::Circle {
                                center: [w / 2., d / 2.],
                                radius_mm: 4.,
                            }]);
                            profile
                        };
                        let count =
                            profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>();
                        let curves = (0..count)
                            .map(|i| multicad_core::references::Origin {
                                source: entity.id,
                                role: format!("corpus-curve-{i}"),
                            })
                            .collect();
                        GeometryOperation::NamedExtrude {
                            profile,
                            height_mm: h,
                            feature: entity.id,
                            curve_origins: curves,
                        }
                    }
                    _ => return Err("Unknown corpus primitive".into()),
                };
                operations.push(operation);
                ids.push(entity.id);
            }
        }
        let known: BTreeSet<_> = ids.iter().copied().collect();
        let mut instances = vec![];
        for entity in doc.entities() {
            if let EntityData::Instance {
                component,
                placement,
            } = &entity.data
            {
                let Some(Entity {
                    data:
                        EntityData::Part {
                            features, material, ..
                        },
                    ..
                }) = doc.get(*component)
                else {
                    return Err("Missing corpus part".into());
                };
                if features.len() != 1 || !known.contains(&features[0]) {
                    return Err("Corpus instance has no supported geometry".into());
                }
                let color = material
                    .and_then(|id| doc.get(id))
                    .and_then(|e| {
                        if let EntityData::Material { color_rgba, .. } = e.data {
                            Some(color_rgba)
                        } else {
                            None
                        }
                    })
                    .unwrap_or([151, 173, 195, 255]);
                instances.push(viewport::Instance {
                    id: entity.id,
                    geometry: features[0],
                    transform: viewport::Transform::from_placement(*placement)?,
                    color,
                });
            }
        }
        let outputs = (0..operations.len()).collect();
        let job = self
            .worker
            .as_ref()
            .map_err(Clone::clone)?
            .submit(GeometryProgram {
                revision: stamp.revision,
                policy: Default::default(),
                operations,
                outputs,
                mesh: MeshOptions {
                    linear_deflection_mm: 0.2,
                    ..Default::default()
                },
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some(Pending {
            stamp,
            job,
            ids,
            instances,
            deflection_mm: 0.2,
            reuse: BTreeMap::new(),
        });
        Ok(())
    }
    fn refine(&mut self, stamp: SceneStamp, quality: DisplayQuality) -> Result<Refinement, String> {
        let scene = self.scene.as_ref().ok_or("No retained source scene")?;
        if (scene.stamp().document, scene.stamp().revision) != (stamp.document, stamp.revision) {
            return Err("Refinement source is stale".into());
        }
        // Straight edges and planar triangles already describe the exact corpus
        // boxes. Only curved assets need a different display approximation.
        if self.curved.is_empty() {
            return Ok(Refinement::Unchanged);
        }
        let ids: Vec<_> = self.curved.iter().copied().collect();
        let keys = ids.iter().map(|id| self.keys[id]).collect();
        let deflection_mm = quality.deflection_mm();
        let job = self
            .worker
            .as_ref()
            .map_err(Clone::clone)?
            .retessellate(
                keys,
                stamp.revision,
                MeshOptions {
                    linear_deflection_mm: deflection_mm,
                    angular_deflection_rad: if deflection_mm >= 0.5 {
                        0.5
                    } else if deflection_mm >= 0.1 {
                        0.2
                    } else {
                        0.05
                    },
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
        self.generation.store(stamp.generation, Ordering::Release);
        self.build = None;
        self.pending = Some(Pending {
            stamp,
            job,
            ids,
            instances: scene.instances().to_vec(),
            deflection_mm,
            reuse: scene.geometries().clone(),
        });
        Ok(Refinement::Started)
    }
    fn poll(&mut self) -> Option<SceneReply> {
        if let Some((stamp, receive)) = self.build.take() {
            match receive.try_recv() {
                Ok(result) => {
                    if let Ok(scene) = &result {
                        self.scene = Some(scene.clone());
                    }
                    return Some(SceneReply { stamp, result });
                }
                Err(TryRecvError::Empty) => {
                    self.build = Some((stamp, receive));
                    return None;
                }
                Err(TryRecvError::Disconnected) => {
                    return Some(SceneReply {
                        stamp,
                        result: Err("Scene builder stopped".into()),
                    });
                }
            }
        }
        let pending = self.pending.as_mut()?;
        let snapshot = match pending.job.try_take() {
            Ok(None) => return None,
            Ok(Some(snapshot)) => snapshot,
            Err(e) => {
                let pending = self.pending.take().expect("pending");
                return Some(SceneReply {
                    stamp: pending.stamp,
                    result: Err(e.to_string()),
                });
            }
        };
        let pending = self.pending.take().expect("ready request");
        if pending.ids.len() != snapshot.solids.len() || snapshot.solids.iter().any(Option::is_none)
        {
            return Some(SceneReply {
                stamp: pending.stamp,
                result: Err("Missing corpus output".into()),
            });
        }
        for (id, solid) in pending.ids.iter().zip(&snapshot.solids) {
            let solid = solid.as_ref().expect("validated output");
            self.keys.insert(*id, solid.key);
            if solid.faces.iter().any(|face| face.plane.is_none()) {
                self.curved.insert(*id);
            }
        }
        let stamp = pending.stamp;
        let current = self.generation.clone();
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let build = || {
                let mut geometries = pending.reuse;
                for (id, solid) in pending.ids.into_iter().zip(snapshot.solids) {
                    if current.load(Ordering::Acquire) != stamp.generation {
                        return Err("Scene build superseded".into());
                    }
                    let solid = solid.ok_or("Corpus output is empty")?;
                    let error_mm = if solid.faces.iter().any(|f| f.plane.is_none()) {
                        pending.deflection_mm
                    } else {
                        0.001
                    };
                    let geometry = viewport::Geometry::new(
                        id,
                        solid.mesh.vertices.as_chunks::<3>().0.to_vec(),
                        solid.mesh.indices,
                        solid.mesh.face_indices,
                        solid.topology,
                    )?
                    .with_topology_display(
                        solid
                            .edges
                            .into_iter()
                            .map(|e| viewport::DisplayEdge {
                                index: e.index,
                                points_mm: e.points_mm,
                            })
                            .collect(),
                        solid
                            .vertices
                            .into_iter()
                            .map(|v| viewport::DisplayPoint {
                                index: v.index,
                                position_mm: v.position_mm,
                            })
                            .collect(),
                        error_mm,
                    )?;
                    geometries.insert(id, Arc::new(geometry));
                }
                Ok(Arc::new(viewport::Scene::new(
                    stamp,
                    geometries.into_values().collect(),
                    pending.instances,
                )?))
            };
            let _ = send.send(build());
        });
        self.build = Some((stamp, receive));
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_corpora_build_real_geometry_and_keep_instance_source_identities() {
        for mixed in [false, true] {
            let document = document(mixed).unwrap();
            assert!(document.reference_diagnostics().is_empty());
            let stamp = SceneStamp {
                document: document.id(),
                revision: 0,
                generation: 1,
            };
            let mut backend = Backend::new();
            backend.request(&document, stamp).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let scene = loop {
                if let Some(reply) = backend.poll() {
                    assert_eq!(reply.stamp, stamp);
                    break reply.result.unwrap();
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Native corpus build timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            assert_eq!(scene.instances().len(), if mixed { 100 } else { 1000 });
            assert_eq!(scene.geometries().len(), if mixed { 100 } else { 1 });
            for instance in scene.instances() {
                assert!(document.get(instance.id).is_some());
                let geometry = &scene.geometries()[&instance.geometry];
                assert!(!geometry.indices().is_empty());
                assert!(!geometry.edges().is_empty());
                assert!(
                    !scene
                        .topology_snapshot(instance.id)
                        .unwrap()
                        .elements
                        .is_empty()
                );
            }
            let next = SceneStamp {
                generation: 2,
                ..stamp
            };
            if !mixed {
                assert_eq!(
                    backend.refine(next, DisplayQuality::Precision).unwrap(),
                    Refinement::Unchanged
                );
                continue;
            }
            let keys = backend.keys.clone();
            assert_eq!(
                backend.refine(next, DisplayQuality::Detail).unwrap(),
                Refinement::Started
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let refined = loop {
                if let Some(reply) = backend.poll() {
                    break reply.result.unwrap();
                }
                assert!(std::time::Instant::now() < deadline, "Refinement timed out");
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            assert_eq!(refined.stamp(), next);
            assert_eq!(backend.keys, keys); // Manufacturing handles remain valid.
            assert_eq!(refined.instances().len(), scene.instances().len());
            for old in scene.instances() {
                let new = refined.instances().iter().find(|i| i.id == old.id).unwrap();
                assert_eq!(new.geometry, old.geometry);
                assert_eq!(
                    refined.topology_snapshot(old.id).unwrap().elements,
                    scene.topology_snapshot(old.id).unwrap().elements
                );
            }
            for (id, old) in scene.geometries() {
                let new = &refined.geometries()[id];
                if backend.curved.contains(id) {
                    assert!(new.indices().len() > old.indices().len());
                } else {
                    assert!(Arc::ptr_eq(old, new));
                }
            }
        }
    }
}
