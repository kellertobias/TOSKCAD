//! Editing upstream or consumed producers requires different output evidence
//! from creating new final bodies. All evidence still belongs to one candidate.
use super::*;
use multicad_core::document::Document;
use std::collections::BTreeSet;
pub(super) enum SceneProof {
    Create,
    Edit {
        producers: Vec<CreatedBody>,
        bodies: BTreeSet<EntityId>,
        removed: Vec<EntityId>,
    },
}
impl SceneProof {
    pub fn edited(
        mut producers: Vec<CreatedBody>,
        restored: &[multicad_core::model::BodyInput],
        removed: Vec<EntityId>,
    ) -> Self {
        producers.extend(restored.iter().map(|input| CreatedBody {
            body: input.body,
            feature: input.producer,
        }));
        let mut seen = BTreeSet::new();
        producers.retain(|output| seen.insert((output.body, output.feature)));
        let bodies = producers
            .iter()
            .map(|output| output.body)
            .chain(restored.iter().map(|input| input.body))
            .collect();
        Self::Edit {
            producers,
            bodies,
            removed,
        }
    }
    pub fn validate(
        &self,
        document: &Document,
        scene: &Scene,
        outputs: &[CreatedBody],
    ) -> Result<(), String> {
        match self {
            Self::Create => {
                for output in outputs {
                    producer(document, scene, output, false)?;
                    if !scene.reference_instances().iter().any(|instance| {
                        instance.id == output.body && instance.geometry == output.feature
                    }) {
                        return Err(
                            "Native preview did not produce every requested body and producer"
                                .into(),
                        );
                    }
                }
            }
            Self::Edit {
                producers,
                bodies,
                removed,
            } => {
                let definitions = multicad_core::model::assembly_definition_parts(document)
                    .map_err(|e| e.to_string())?;
                let info = scene
                    .model_evaluation()
                    .ok_or("Editing requires a source-bound native evaluation")?;
                for output in producers {
                    producer(document, scene, output, true)?;
                }
                for body in bodies {
                    let Some(EntityData::Part { features, .. }) =
                        document.get(*body).map(|entity| &entity.data)
                    else {
                        return Err(
                            "An affected source body is missing from the native edit candidate"
                                .into(),
                        );
                    };
                    let valid = if info.consumed_bodies.contains(body) {
                        !scene
                            .reference_instances()
                            .iter()
                            .any(|instance| instance.id == *body)
                    } else {
                        let head = features.iter().rev().copied().find(|id| {
                            matches!(
                                document.get(*id).map(|entity| &entity.data),
                                Some(EntityData::Feature {
                                    suppressed: false,
                                    ..
                                })
                            )
                        });
                        head.is_some_and(|head| {
                            nonempty(scene, head)
                                && ((definitions.contains(body)
                                    && scene.source_producer(*body) == Some(head))
                                    || scene.reference_instances().iter().any(|instance| {
                                        instance.id == *body && instance.geometry == head
                                    }))
                        })
                    };
                    if !valid {
                        return Err("Native edit preview did not produce the current final head or consumed state of every affected body".into());
                    }
                }
                for feature in removed {
                    if document.get(*feature).is_some()
                        || scene.geometries().contains_key(feature)
                        || scene
                            .reference_instances()
                            .iter()
                            .any(|instance| instance.geometry == *feature)
                    {
                        return Err(
                            "Native edit preview retained a removed operation producer".into()
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
fn nonempty(scene: &Scene, feature: EntityId) -> bool {
    scene
        .geometries()
        .get(&feature)
        .is_some_and(|geometry| !geometry.vertices().is_empty() && !geometry.indices().is_empty())
}
fn producer(
    document: &Document,
    scene: &Scene,
    output: &CreatedBody,
    named: bool,
) -> Result<(), String> {
    if !matches!(document.get(output.body).map(|entity|&entity.data),Some(EntityData::Part{features,..}) if features.contains(&output.feature))
        || !nonempty(scene, output.feature)
        || (named
            && !scene
                .geometry_snapshot(output.feature)
                .is_some_and(|snapshot| !snapshot.elements.is_empty()))
    {
        return Err(
            "Native preview did not retain every requested producer and its source body".into(),
        );
    }
    Ok(())
}

/// Every canonical occurrence must be represented by its actual current
/// component producer and evaluated pose. Instance IDs are never Part outputs.
pub(super) fn placements(document: &Document, revision: u64, scene: &Scene) -> Result<(), String> {
    let members =
        multicad_core::model::assembly_member_instances(document).map_err(|e| e.to_string())?;
    if document
        .entities()
        .any(|e| matches!(&e.data,EntityData::Assembly{instances} if !instances.is_empty()))
    {
        crate::assembly::validate_scene(document, revision, scene)?;
    }
    let snapshots = scene.reference_snapshots();
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
    for entity in document.entities() {
        let EntityData::Instance { component, .. } = entity.data else {
            continue;
        };
        if members.contains(&entity.id) {
            continue;
        }
        let pose = multicad_core::model::resolve_instance_pose_with_snapshots(
            document,
            entity.id,
            &parameters,
            revision,
            &snapshots,
        )
        .map_err(|e| e.to_string())?;
        let producer = match document.get(component).map(|e| &e.data) {
            Some(EntityData::Assembly { instances }) if !instances.is_empty() => component,
            Some(EntityData::Part { features, .. }) => *features
                .iter()
                .rev()
                .find(|id| {
                    matches!(
                        document.get(**id).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .ok_or("Placed component has no active producer")?,
            _ => {
                return Err(
                    "Placement preview requires a Part or reusable Assembly component".into(),
                );
            }
        };
        let instance = scene
            .reference_instances()
            .iter()
            .find(|i| i.id == entity.id)
            .ok_or("Native preview omitted a placed Instance")?;
        if instance.geometry != producer || !nonempty(scene, producer) {
            return Err("Native preview returned the wrong placed component producer".into());
        }
        let actual = instance.transform;
        if actual
            .point([0.; 3])
            .iter()
            .zip(pose.translation_mm)
            .any(|(a, b)| (a - b).abs() > 1e-8)
            || [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
                .into_iter()
                .any(|axis| {
                    actual
                        .vector(axis)
                        .iter()
                        .zip(pose.vector(axis))
                        .any(|(a, b)| (a - b).abs() > 1e-9)
                })
        {
            return Err("Native preview returned an incorrect Instance pose".into());
        }
    }
    Ok(())
}
