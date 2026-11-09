use crate::{
    camera::Camera,
    navigation::DisplayStyle,
    theme,
    viewport::{self, Instance, Scene, SceneStamp, Transform},
};
use eframe::egui;
use multicad_core::{
    document::EntityId,
    exchange::{ExportPreview, OutputSubject, PreviewKey},
    model::OccurrencePath,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_INSTANCES: usize = 512;
const MAX_TRIANGLES: usize = 2_000_000;
const MAX_EDGE_SEGMENTS: usize = 1_000_000;

#[derive(Clone)]
struct CachedProjection {
    key: PreviewKey,
    stamp: SceneStamp,
    scene: Option<Arc<Scene>>,
}

fn is_current(scene: &Scene, preview: &ExportPreview) -> bool {
    let expected = &preview.key.source;
    let stamp = scene.stamp();
    scene.is_full_model()
        && stamp.document == expected.document
        && stamp.revision == expected.revision
        && preview.selection.stamp == *expected
        && scene
            .model_evaluation()
            .is_some_and(|native| native.source_hash == expected.source_hash)
}

/// Draw the 3D source of the resolved native artifacts in the Common export
/// workspace. The exact output preflight remains the authority for writing.
pub(super) fn show(ui: &mut egui::Ui, scene: &Scene, preview: &ExportPreview) -> bool {
    let width = ui.available_width();
    if width < 32. || !is_current(scene, preview) {
        return false;
    }
    let id = egui::Id::new("native-export-projection");
    let cached = ui
        .data(|data| data.get_temp::<CachedProjection>(id))
        .filter(|entry| entry.key == preview.key && entry.stamp == scene.stamp());
    let projected = cached.map_or_else(
        || {
            let projection = build(scene, preview);
            ui.data_mut(|data| {
                data.insert_temp(
                    id,
                    CachedProjection {
                        key: preview.key.clone(),
                        stamp: scene.stamp(),
                        scene: projection.clone(),
                    },
                );
            });
            projection
        },
        |entry| entry.scene,
    );
    let Some(projected) = projected else {
        return false;
    };
    let Some(bounds) = projected.world_bounds() else {
        return false;
    };
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width, (width * 0.62).clamp(154., 280.)),
        egui::Sense::hover(),
    );
    painter.rect_filled(response.rect, 5., theme::PANEL);
    let rect = response.rect.shrink(5.);
    let mut camera = Camera::default();
    camera.fit(
        &bounds.map(|point| point.map(|coordinate| coordinate as f32)),
        rect,
    );
    viewport::paint_scene(
        &painter,
        rect,
        projected,
        camera,
        None,
        DisplayStyle::VisibleEdges,
    );
    true
}

fn build(scene: &Scene, preview: &ExportPreview) -> Option<Arc<Scene>> {
    let exported: BTreeSet<_> = preview
        .artifacts
        .iter()
        .filter(|artifact| artifact.planar.is_none())
        .flat_map(|artifact| match &artifact.output.subject {
            OutputSubject::Part(part) => std::slice::from_ref(part),
            OutputSubject::Parts(parts) => parts.as_slice(),
            _ => &[],
        })
        .collect();
    if exported.is_empty() || exported.len() > MAX_INSTANCES {
        return None;
    }
    let parts = preview
        .selection
        .parts
        .iter()
        .map(|part| &part.reference)
        .filter(|part| exported.contains(part));
    let roots: BTreeMap<_, _> = scene
        .reference_instances()
        .iter()
        .map(|instance| (instance.id, instance))
        .collect();
    let leaves: BTreeMap<_, _> = scene
        .assembly_evaluation()
        .into_iter()
        .flat_map(|assembly| &assembly.leaves)
        .map(|leaf| (&leaf.path, leaf))
        .collect();
    let mut geometries = BTreeMap::new();
    let mut mesh_counts = BTreeMap::new();
    let mut instances = Vec::with_capacity(exported.len());
    let (mut triangles, mut segments) = (0usize, 0usize);
    for reference in parts {
        let producer = scene.source_producer(reference.part)?;
        let instance = if let Some((&root, members)) = reference.occurrence_path.split_first() {
            let leaf = leaves.get(&OccurrencePath {
                root,
                members: members.to_vec(),
            })?;
            if leaf.component != reference.part || leaf.producer != producer {
                return None;
            }
            Instance {
                // The exact path above selected the geometry and pose. This ID
                // is only a unique draw handle for possibly repeated leaves.
                id: EntityId::new(),
                geometry: producer,
                transform: Transform::from_rigid_pose(leaf.pose).ok()?,
                color: roots
                    .get(&reference.part)
                    .unwrap_or(roots.get(&root)?)
                    .color,
            }
        } else {
            let instance = *roots.get(&reference.part)?;
            if instance.geometry != producer {
                return None;
            }
            instance.clone()
        };
        let geometry = scene.geometries().get(&producer)?;
        let (mesh_triangles, mesh_segments) = *mesh_counts.entry(producer).or_insert_with(|| {
            (
                geometry.indices().len() / 3,
                geometry
                    .edges()
                    .iter()
                    .map(|edge| edge.points_mm.len().saturating_sub(1))
                    .sum::<usize>(),
            )
        });
        triangles += mesh_triangles;
        segments += mesh_segments;
        if triangles > MAX_TRIANGLES || segments > MAX_EDGE_SEGMENTS {
            return None;
        }
        geometries
            .entry(producer)
            .or_insert_with(|| geometry.clone());
        instances.push(instance);
    }
    if instances.len() != exported.len() {
        return None;
    }
    Scene::new(scene.stamp(), geometries.into_values().collect(), instances)
        .ok()
        .map(Arc::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel},
        exchange::{ExportArtifact, ExportStamp, NamedOutput, PartRef, ResolvedPart, ResolvedSet},
        model::{AssemblyEvaluationInfo, AssemblyLeafReceipt, ModelEvaluationInfo, RigidPose},
        project_file::ContentHash,
    };

    #[test]
    fn resolves_only_exported_occurrence_paths_and_rejects_stale_receipts() {
        let [
            document,
            producer,
            component,
            root,
            member,
            sibling,
            set,
            recipe,
        ] = std::array::from_fn(|_| EntityId::new());
        let source_hash = ContentHash::of(b"exact evaluated document");
        let stamp = SceneStamp {
            document,
            revision: 7,
            generation: 1,
        };
        let pose = RigidPose {
            translation_mm: [17., 31., 0.],
            ..RigidPose::default()
        };
        let source = Scene::new(
            stamp,
            vec![Arc::new(
                viewport::Geometry::new(
                    producer,
                    vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                    vec![0, 1, 2],
                    vec![0],
                    vec![],
                )
                .unwrap(),
            )],
            vec![Instance {
                id: root,
                geometry: producer,
                transform: Transform::default(),
                color: [180, 180, 180, 255],
            }],
        )
        .unwrap()
        .with_reference_aliases([(component, producer)].into())
        .unwrap()
        .with_model_evaluation(Arc::new(ModelEvaluationInfo {
            source_hash,
            consumed_bodies: Default::default(),
            cut_targets: vec![],
            screw_connection_checks: vec![],
        }))
        .unwrap()
        .with_assembly_evaluation(Arc::new(AssemblyEvaluationInfo {
            source_hash,
            leaves: vec![AssemblyLeafReceipt {
                path: OccurrencePath {
                    root,
                    members: vec![member],
                },
                component,
                producer,
                pose,
            }],
        }))
        .unwrap();
        let exported = PartRef {
            part: component,
            occurrence_path: vec![root, member],
            root_assembly: None,
        };
        let other = PartRef {
            occurrence_path: vec![root, sibling],
            ..exported.clone()
        };
        let source_stamp = ExportStamp {
            document,
            revision: stamp.revision,
            source_hash,
        };
        let mut preview = ExportPreview {
            requested_by: Actor::local(Channel::Ui),
            key: PreviewKey {
                source: source_stamp.clone(),
                set,
                plan_hash: ContentHash::of(b"export plan"),
            },
            provider: "test".into(),
            selection: ResolvedSet {
                stamp: source_stamp,
                set,
                name: "Scope".into(),
                parts: vec![exported.clone(), other]
                    .into_iter()
                    .map(|reference| ResolvedPart {
                        reference,
                        name: "Selected part".into(),
                        occurrence_names: vec![],
                    })
                    .collect(),
                pages: vec![],
                recipes: vec![],
                naming: Default::default(),
            },
            recipes: vec![],
            artifacts: vec![ExportArtifact {
                output: NamedOutput {
                    recipe,
                    subject: OutputSubject::Part(exported),
                    filename: "part.step".into(),
                },
                producer,
                solid: root,
                bounds_mm: [[17., 31., 0.], [18., 32., 0.]],
                volume_mm3: 1.,
                solid_count: 1,
                planar: None,
                mesh: None,
                drawing: None,
                schedule: None,
            }],
            excluded: vec![],
        };
        assert!(is_current(&source, &preview));
        let projected = build(&source, &preview).unwrap();
        assert_eq!(projected.instances().len(), 1);
        let instance = &projected.instances()[0];
        assert_eq!(instance.geometry, producer);
        assert_eq!(instance.transform.point([0.; 3]), pose.translation_mm);
        assert_eq!(
            projected.world_bounds(),
            Some([[17., 31., 0.], [18., 32., 0.]])
        );
        assert!(Arc::ptr_eq(
            &projected.geometries()[&producer],
            &source.geometries()[&producer],
        ));

        let wrong_hash = ContentHash::of(b"foreign geometry");
        preview.key.source.source_hash = wrong_hash;
        preview.selection.stamp.source_hash = wrong_hash;
        assert!(!is_current(&source, &preview));
    }
}
