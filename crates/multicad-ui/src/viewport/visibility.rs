//! Source-revision-gated display views. Geometry and reference occurrences stay
//! retained; only the shared draw/pick instance list is filtered.
use super::Scene;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    hierarchy::{HierarchyError, Index},
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Default)]
pub struct VisibilityFilter {
    cached: Option<(Arc<Scene>, Arc<Scene>)>,
}
pub(crate) fn source_visible(
    document: &Document,
    index: &Index<'_>,
    entry: EntityId,
) -> Result<bool, String> {
    let mut pending = vec![entry];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) || document.hierarchy().hidden.contains(&id) {
            continue;
        }
        match index.parent(id) {
            Ok(None) => {
                if !document.hierarchy().hidden.contains(&document.id()) {
                    return Ok(true);
                }
            }
            Ok(Some(parent)) => pending.push(parent),
            // Display visibility is the union of known legacy parent paths;
            // ownership queries still refuse to guess one parent.
            Err(HierarchyError::AmbiguousOwner { parents, .. }) => pending.extend(parents),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(false)
}
impl VisibilityFilter {
    pub fn clear(&mut self) {
        self.cached = None;
    }
    pub fn apply(
        &mut self,
        scene: &Arc<Scene>,
        document: &Document,
        revision: u64,
    ) -> Result<Arc<Scene>, String> {
        if scene.stamp().document != document.id() {
            return Err("Visibility belongs to another document".into());
        }
        if let Some((source, view)) = &self.cached
            && Arc::ptr_eq(source, scene)
        {
            return Ok(view.clone());
        }
        if scene.stamp().revision != revision {
            return Err("Wait for a current model before applying source visibility".into());
        }
        let index = Index::new(document).map_err(|error| error.to_string())?;
        let mut visible = BTreeSet::new();
        for instance in scene.reference_instances() {
            let source = document
                .get(instance.id)
                .ok_or("Display instance has no canonical source entry")?;
            if !source_visible(document, &index, instance.id)? {
                continue;
            }
            if let EntityData::Instance { component, .. } = source.data
                && !source_visible(document, &index, component)?
            {
                continue;
            }
            visible.insert(instance.id);
        }
        let view = if visible.len() == scene.instances().len()
            && scene
                .instances()
                .iter()
                .all(|instance| visible.contains(&instance.id))
        {
            scene.clone()
        } else {
            scene.visible_subset(&visible)?
        };
        self.cached = Some((scene.clone(), view.clone()));
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::{Geometry, Instance, Ray, SceneStamp, Transform};
    use multicad_core::{
        document::{Definition, Entity, Mutation, Placement},
        hierarchy::Hierarchy,
    };
    use multicad_kernel::{MeshOptions, Solid};
    use std::collections::BTreeMap;
    fn fixture() -> (Document, Arc<Scene>, EntityId, EntityId, EntityId, EntityId) {
        let producer = Entity::new(
            "Box source",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.box".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let body = Entity::new(
            "Body",
            EntityData::Part {
                features: vec![producer.id],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        let group = Entity::new("Hidden group", EntityData::Group {});
        let component = Entity::new("Component", EntityData::Assembly { instances: vec![] });
        let entities: Vec<_> = (0..2)
            .map(|i| {
                Entity::new(
                    format!("Placed {i}"),
                    EntityData::Instance {
                        component: body.id,
                        placement: Placement {
                            translation_mm: [i as f64 * 50., 0., 0.],
                            ..Default::default()
                        },
                    },
                )
            })
            .collect();
        let doc = Document::new("Visibility test")
            .unwrap()
            .edited(&[
                Mutation::Insert(producer.clone()),
                Mutation::Insert(body.clone()),
                Mutation::Insert(group.clone()),
                Mutation::Insert(component.clone()),
                Mutation::Insert(entities[0].clone()),
                Mutation::Insert(entities[1].clone()),
                Mutation::SetHierarchy(Hierarchy {
                    members: BTreeMap::from([
                        (component.id, vec![group.id]),
                        (group.id, vec![entities[0].id]),
                    ]),
                    ..Default::default()
                }),
            ])
            .unwrap();
        let solid = Solid::box_named([20., 12., 8.], producer.id, Default::default()).unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        let geometry = Arc::new(
            Geometry::new(
                producer.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid.topology_snapshot(producer.id, 0).unwrap().elements,
            )
            .unwrap(),
        );
        let instances = entities
            .iter()
            .map(|entity| {
                let EntityData::Instance { placement, .. } = entity.data else {
                    unreachable!()
                };
                Instance {
                    id: entity.id,
                    geometry: producer.id,
                    transform: Transform::from_placement(placement).unwrap(),
                    color: [120, 130, 140, 255],
                }
            })
            .collect();
        let scene = Arc::new(
            Scene::new(
                SceneStamp {
                    document: doc.id(),
                    revision: 0,
                    generation: 1,
                },
                vec![geometry],
                instances,
            )
            .unwrap(),
        );
        (doc, scene, body.id, group.id, component.id, entities[0].id)
    }
    fn restamp(scene: &Scene, revision: u64) -> Arc<Scene> {
        Arc::new(
            Scene::new(
                SceneStamp {
                    revision,
                    generation: revision + 1,
                    ..scene.stamp()
                },
                scene.geometries().values().cloned().collect(),
                scene.reference_instances().to_vec(),
            )
            .unwrap(),
        )
    }
    #[test]
    fn ancestor_visibility_filters_draw_pick_and_bounds_but_retains_semantic_sources() {
        let (doc, scene, _, group, _, first) = fixture();
        let ray = Ray::new([10., 6., 100.], [0., 0., -1.]).unwrap();
        assert_eq!(scene.pick_face(ray).unwrap().instance, first);
        let mut filter = VisibilityFilter::default();
        assert!(Arc::ptr_eq(&filter.apply(&scene, &doc, 0).unwrap(), &scene));
        let mut state = doc.hierarchy().clone();
        state.hidden.insert(group);
        let hidden = doc.edited(&[Mutation::SetHierarchy(state)]).unwrap();
        let newer = restamp(&scene, 1);
        let view = filter.apply(&newer, &hidden, 1).unwrap();
        assert_eq!(view.instances().len(), 1);
        assert!(!view.instances().iter().any(|instance| instance.id == first));
        assert!(view.pick_face(ray).is_none());
        assert!(view.world_bounds().unwrap()[0][0] >= 50.);
        assert_eq!(view.reference_instances().len(), 2);
        assert!(view.topology_snapshot(first).is_some());
        assert!(view.reference_snapshots().contains_key(&first));
        for (id, geometry) in scene.geometries() {
            assert!(Arc::ptr_eq(geometry, &view.geometries()[id]));
        }
        assert!(Arc::ptr_eq(
            &view,
            &filter.apply(&newer, &hidden, 1).unwrap()
        ));
        assert_eq!(scene.instances().len(), 2);
    }
    #[test]
    fn definitions_and_nested_components_hide_occurrences_with_current_source_guards() {
        let (doc, scene, body, _, component, _) = fixture();
        let mut filter = VisibilityFilter::default();
        let old = filter.apply(&scene, &doc, 0).unwrap();
        let mut state = doc.hierarchy().clone();
        state.hidden.insert(body);
        let hidden = doc.edited(&[Mutation::SetHierarchy(state)]).unwrap();
        assert!(Arc::ptr_eq(
            &old,
            &filter.apply(&scene, &hidden, 1).unwrap()
        ));
        assert!(
            VisibilityFilter::default()
                .apply(&scene, &hidden, 1)
                .is_err()
        );
        let view = filter.apply(&restamp(&scene, 1), &hidden, 1).unwrap();
        assert!(view.instances().is_empty());
        assert!(view.world_bounds().is_none());
        let mut state = doc.hierarchy().clone();
        state.hidden.insert(component);
        let hidden = doc.edited(&[Mutation::SetHierarchy(state)]).unwrap();
        assert_eq!(
            VisibilityFilter::default()
                .apply(&restamp(&scene, 1), &hidden, 1)
                .unwrap()
                .instances()
                .len(),
            1
        );
        assert!(
            filter
                .apply(&scene, &Document::new("Other").unwrap(), 0)
                .is_err()
        );
        assert!(
            scene
                .visible_subset(&BTreeSet::from([EntityId::new()]))
                .is_err()
        );
    }
    #[test]
    fn legacy_shared_paths_have_union_visibility_without_guessing_ownership() {
        let (doc, scene, _, _, _, first) = fixture();
        let a = Entity::new(
            "Legacy A",
            EntityData::Assembly {
                instances: vec![first],
            },
        );
        let b = Entity::new(
            "Legacy B",
            EntityData::Assembly {
                instances: vec![first],
            },
        );
        let mut state = Hierarchy::default();
        state.hidden.insert(a.id);
        let doc = doc
            .edited(&[
                Mutation::Insert(a.clone()),
                Mutation::Insert(b.clone()),
                Mutation::SetHierarchy(state),
            ])
            .unwrap();
        let index = Index::new(&doc).unwrap();
        assert!(index.owning_component(first).is_err());
        assert!(source_visible(&doc, &index, first).unwrap());
        assert_eq!(
            VisibilityFilter::default()
                .apply(&scene, &doc, 0)
                .unwrap()
                .instances()
                .len(),
            2
        );
        let mut state = doc.hierarchy().clone();
        state.hidden.insert(b.id);
        let doc = doc.edited(&[Mutation::SetHierarchy(state)]).unwrap();
        assert_eq!(
            VisibilityFilter::default()
                .apply(&scene, &doc, 0)
                .unwrap()
                .instances()
                .len(),
            1
        );
    }
}
