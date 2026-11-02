//! Local Architecture navigation over canonical levels and native geometry.
//! This metadata never creates a second building model or changes ownership.
use crate::{camera::Camera, viewport::Scene};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    equipment,
    hierarchy::Index,
    model,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Saved user-local context, separate from CAD source and Undo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recall {
    pub plan: bool,
    pub level: Option<EntityId>,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub id: EntityId,
    pub name: String,
    pub kind: &'static str,
}

#[derive(Clone, Debug)]
pub struct Level {
    pub id: Option<EntityId>,
    pub name: String,
    pub elevation_mm: Result<f64, String>,
    pub owner: Result<String, String>,
    pub members: Vec<Member>,
}

/// Source facts cached once per document revision, including repair diagnostics.
pub struct Levels {
    pub stamp: (EntityId, u64),
    pub rows: Vec<Level>,
    pub diagnostics: Vec<String>,
    membership: BTreeMap<EntityId, Option<EntityId>>,
}
impl Levels {
    pub fn new(document: &Document, revision: u64) -> Self {
        let parameters = multicad_core::parameters::evaluate(document).map_err(|e| e.to_string());
        let index = Index::new(document).map_err(|e| e.to_string());
        let owner = |id| {
            index.as_ref().map_err(Clone::clone).and_then(|index| {
                index
                    .parent(id)
                    .map(|parent| crate::model_tree::container_path(document, index, parent))
                    .map_err(|e| e.to_string())
            })
        };
        let mut rows: Vec<_> = model::levels(document)
            .map(|entity| Level {
                id: Some(entity.id),
                name: entity.name.clone(),
                elevation_mm: parameters
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|parameters| {
                        model::resolve_level(document, parameters, entity.id)
                            .map(|level| level.elevation_mm)
                            .map_err(|e| e.to_string())
                    }),
                owner: owner(entity.id),
                members: vec![],
            })
            .collect();
        rows.sort_by(|a, b| {
            a.elevation_mm
                .as_ref()
                .ok()
                .copied()
                .unwrap_or(f64::INFINITY)
                .total_cmp(
                    &b.elevation_mm
                        .as_ref()
                        .ok()
                        .copied()
                        .unwrap_or(f64::INFINITY),
                )
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        rows.push(Level {
            id: None,
            name: "World / unassigned".into(),
            elevation_mm: Ok(0.),
            owner: Ok(document.name().into()),
            members: vec![],
        });
        let mut result = Self {
            stamp: (document.id(), revision),
            rows,
            diagnostics: vec![],
            membership: BTreeMap::new(),
        };
        for entity in document.entities() {
            let source = match &entity.data {
                EntityData::Feature { definition, .. }
                | EntityData::Extension { definition, .. } => definition.type_id.as_str(),
                _ => continue,
            };
            let membership = match source {
                model::WALL_TYPE => model::read_wall(document, entity.id)
                    .map(|wall| (wall.output.body, wall.level, "Wall")),
                model::SLAB_TYPE => model::read_slab(document, entity.id).map(|slab| {
                    (
                        slab.output.body,
                        Some(slab.level),
                        match slab.kind {
                            model::SlabKind::Slab => "Floor slab",
                            model::SlabKind::Pedestal => "Pedestal",
                        },
                    )
                }),
                model::STAIR_TYPE => model::read_stair(document, entity.id)
                    .map(|stair| (stair.output.body, Some(stair.level), "Stair")),
                equipment::ATTACHMENT_TYPE => equipment::read_attachment(document, entity.id)
                    .map(|attachment| (attachment.owner, Some(attachment.floor), "Equipment")),
                _ => continue,
            };
            let membership = match membership {
                Ok(value) => Some(value),
                Err(error) => {
                    result.diagnostics.push(format!("{}: {error}", entity.name));
                    None
                }
            };
            if let Some((body, level, kind)) = membership {
                result.membership.insert(entity.id, level);
                result.membership.insert(body, level);
                if !result.rows.iter().any(|row| row.id == level) {
                    let message = format!("Missing level {}", level.expect("world row exists"));
                    result.diagnostics.push(message.clone());
                    result.rows.push(Level {
                        id: level,
                        name: message.clone(),
                        elevation_mm: Err(message),
                        owner: Err("Restore the level or repair its dependent objects".into()),
                        members: vec![],
                    });
                }
                let row = result
                    .rows
                    .iter_mut()
                    .find(|row| row.id == level)
                    .expect("level row");
                let name = document
                    .get(body)
                    .map_or_else(|| entity.name.clone(), |body| body.name.clone());
                row.members.push(Member {
                    id: body,
                    name,
                    kind,
                });
            }
        }
        for row in &mut result.rows {
            row.members
                .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        }
        result
    }
    pub fn level_for(&self, entity: EntityId) -> Option<Option<EntityId>> {
        self.membership.get(&entity).copied()
    }
    pub fn elevation(&self, level: Option<EntityId>) -> Result<f64, String> {
        self.rows
            .iter()
            .find(|row| row.id == level)
            .ok_or_else(|| "The active level is missing; choose or repair a level".to_owned())?
            .elevation_mm
            .clone()
    }
    /// Preserve source visibility while omitting other levels from both paint
    /// and picking. Reference instances/topology remain in the native receipt.
    pub fn plan_scene(
        &self,
        scene: &Arc<Scene>,
        level: Option<EntityId>,
    ) -> Result<Arc<Scene>, String> {
        if self.stamp != (scene.stamp().document, scene.stamp().revision) {
            return Err("Wait for a current native model before viewing this level".into());
        }
        self.elevation(level)?;
        let visible: BTreeSet<_> = scene
            .instances()
            .iter()
            .filter(|instance| {
                self.level_for(instance.id)
                    .is_none_or(|owner| owner == level)
            })
            .map(|instance| instance.id)
            .collect();
        if visible.len() == scene.instances().len() {
            Ok(scene.clone())
        } else {
            scene.visible_subset(&visible)
        }
    }
}

#[derive(Default)]
pub struct Context {
    pub enabled: bool,
    pub recall: Recall,
    initialized: bool,
    pub levels: Option<Levels>,
    spatial_camera: Option<Camera>,
    plan_camera: Option<Camera>,
    filtered: Option<(Arc<Scene>, Option<EntityId>, Arc<Scene>)>,
}
impl Context {
    pub fn sync(&mut self, document: &Document, revision: u64) {
        if self
            .levels
            .as_ref()
            .is_none_or(|levels| levels.stamp != (document.id(), revision))
        {
            self.levels = Some(Levels::new(document, revision));
            self.filtered = None;
        }
        if !self.initialized {
            self.recall.level = self.levels.as_ref().and_then(|levels| {
                levels
                    .rows
                    .iter()
                    .find(|level| level.id.is_some() && level.elevation_mm.is_ok())
                    .and_then(|level| level.id)
            });
            self.initialized = true;
        }
    }
    pub fn restore(&mut self, recall: Recall, camera: &mut Camera) {
        let enabled = self.enabled;
        self.set_enabled(false, camera);
        self.recall = recall;
        self.initialized = true;
        self.filtered = None;
        self.set_enabled(enabled, camera);
    }
    pub fn elevation(&self) -> Result<f64, String> {
        self.levels
            .as_ref()
            .ok_or("Architecture levels are not ready")?
            .elevation(self.recall.level)
    }
    pub fn planar(&self) -> bool {
        self.enabled && self.recall.plan
    }
    pub fn set_enabled(&mut self, enabled: bool, camera: &mut Camera) {
        if self.enabled == enabled {
            return;
        }
        if self.recall.plan {
            if enabled {
                self.enter_plan(camera);
            } else {
                self.leave_plan(camera);
            }
        }
        self.enabled = enabled;
    }
    pub fn set_plan(&mut self, plan: bool, camera: &mut Camera) {
        if self.recall.plan == plan {
            return;
        }
        self.recall.plan = plan;
        if self.enabled {
            if plan {
                self.enter_plan(camera);
            } else {
                self.leave_plan(camera);
            }
        }
    }
    fn enter_plan(&mut self, camera: &mut Camera) {
        self.spatial_camera = Some(*camera);
        if let Some(plan) = self.plan_camera {
            *camera = plan;
        }
        self.constrain(camera);
    }
    fn leave_plan(&mut self, camera: &mut Camera) {
        self.plan_camera = Some(*camera);
        if let Some(spatial) = self.spatial_camera.take() {
            *camera = spatial;
        }
    }
    pub fn constrain(&self, camera: &mut Camera) {
        camera.yaw = 0.;
        camera.pitch = std::f32::consts::FRAC_PI_2;
        camera.perspective = false;
        if let Ok(elevation) = self.elevation() {
            camera.target[2] = elevation as f32;
        }
    }
    pub fn home_plan(&self, bounds: Option<[[f64; 3]; 2]>, rect: crate::egui::Rect) -> Camera {
        let mut camera = Camera {
            target: [0.; 3],
            scale: 0.05,
            ..Default::default()
        };
        self.constrain(&mut camera);
        let elevation = camera.target[2];
        if let Some([low, high]) = bounds {
            let corners: Vec<_> = (0..8)
                .map(|corner| {
                    std::array::from_fn(|axis| {
                        if corner & (1 << axis) == 0 {
                            low[axis] as f32
                        } else {
                            high[axis] as f32
                        }
                    })
                })
                .collect();
            camera.fit(&corners, rect);
            camera.target[2] = elevation;
        }
        camera
    }
    pub fn scene(&mut self, scene: &Arc<Scene>) -> Result<Arc<Scene>, String> {
        if !self.planar() {
            return Ok(scene.clone());
        }
        if let Some((source, level, view)) = &self.filtered
            && Arc::ptr_eq(source, scene)
            && *level == self.recall.level
        {
            return Ok(view.clone());
        }
        let view = self
            .levels
            .as_ref()
            .ok_or("Architecture levels are not ready")?
            .plan_scene(scene, self.recall.level)?;
        self.filtered = Some((scene.clone(), self.recall.level, view.clone()));
        Ok(view)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::viewport::{Geometry, Instance, Ray, SceneStamp, Transform};
    use multicad_core::{
        commands::{Actor, Channel, Session},
        document::Placement,
        model::{BodyOutput, LevelDefinition, SlabDefinition, SlabKind, length},
    };
    use multicad_kernel::{MeshOptions, Solid};

    fn actor() -> Actor {
        Actor::local(Channel::Ui)
    }
    pub(crate) fn building() -> (Session, [EntityId; 2], [EntityId; 2], [EntityId; 2]) {
        let mut session = Session::new(Document::new("Two levels").unwrap()).unwrap();
        let mut levels = [EntityId::nil(); 2];
        let mut bodies = [EntityId::nil(); 2];
        let mut features = [EntityId::nil(); 2];
        for i in 0..2 {
            let (level, prepared) = model::prepare_level(
                &session,
                session.revision(),
                actor(),
                if i == 0 { "Ground" } else { "Upper" },
                &LevelDefinition {
                    elevation: length(i as f64 * 3000.),
                    height: length(3000.),
                    slab_thickness: length(200.),
                },
            )
            .unwrap();
            session.commit(prepared).unwrap();
            let body = EntityId::new();
            let prepared = model::prepare_slab(
                &session,
                session.revision(),
                actor(),
                if i == 0 {
                    "Ground floor"
                } else {
                    "Upper floor"
                },
                &SlabDefinition {
                    output: BodyOutput { body },
                    level,
                    kind: SlabKind::Slab,
                    outline: vec![
                        [length(0.), length(0.)],
                        [length(1000.), length(0.)],
                        [length(1000.), length(800.)],
                        [length(0.), length(800.)],
                    ],
                    thickness: None,
                    holes: vec![],
                },
                None,
            )
            .unwrap();
            features[i] = prepared.outputs[0].feature;
            session.commit(prepared.prepared).unwrap();
            levels[i] = level;
            bodies[i] = body;
        }
        (session, levels, bodies, features)
    }
    fn scene(session: &Session, bodies: [EntityId; 2], features: [EntityId; 2]) -> Arc<Scene> {
        let geometries = features
            .iter()
            .map(|feature| {
                let solid =
                    Solid::box_named([1000., 800., 200.], *feature, Default::default()).unwrap();
                let mesh = solid.mesh(MeshOptions::default()).unwrap();
                Arc::new(
                    Geometry::new(
                        *feature,
                        mesh.vertices.as_chunks::<3>().0.to_vec(),
                        mesh.indices,
                        mesh.face_indices,
                        solid
                            .topology_snapshot(*feature, session.revision())
                            .unwrap()
                            .elements,
                    )
                    .unwrap(),
                )
            })
            .collect();
        let instances = (0..2)
            .map(|i| Instance {
                id: bodies[i],
                geometry: features[i],
                transform: Transform::from_placement(Placement {
                    translation_mm: [0., 0., i as f64 * 3000. - 200.],
                    ..Default::default()
                })
                .unwrap(),
                color: [120, 130, 140, 255],
            })
            .collect();
        Arc::new(
            Scene::new(
                SceneStamp {
                    document: session.document().id(),
                    revision: session.revision(),
                    generation: 1,
                },
                geometries,
                instances,
            )
            .unwrap(),
        )
    }
    #[test]
    fn level_facts_follow_canonical_edits_and_history_without_changing_membership() {
        let (mut session, levels, bodies, features) = building();
        let mut context = Context::default();
        context.sync(session.document(), session.revision());
        assert_eq!(context.recall.level, Some(levels[0]));
        let facts = context.levels.as_ref().unwrap();
        assert_eq!(facts.rows.len(), 3);
        assert_eq!(facts.rows[1].name, "Upper");
        assert_eq!(facts.rows[1].elevation_mm, Ok(3000.));
        assert_eq!(facts.rows[1].owner, Ok("Two levels".into()));
        assert_eq!(facts.rows[1].members[0].id, bodies[1]);
        assert_eq!(facts.level_for(features[1]), Some(Some(levels[1])));
        let before = session.document().clone();
        context.recall.level = Some(levels[1]);
        let mut value = model::read_level(session.document(), levels[1]).unwrap();
        value.elevation = length(3500.);
        let prepared =
            model::prepare_level_edit(&session, session.revision(), actor(), levels[1], &value)
                .unwrap();
        session.commit(prepared).unwrap();
        context.sync(session.document(), session.revision());
        assert_eq!(context.elevation(), Ok(3500.));
        assert_eq!(
            model::read_slab(session.document(), features[1])
                .unwrap()
                .level,
            levels[1]
        );
        session.undo(session.revision(), actor()).unwrap();
        context.sync(session.document(), session.revision());
        assert_eq!(context.elevation(), Ok(3000.));
        assert_eq!(session.document(), &before);
        session.redo(session.revision(), actor()).unwrap();
        context.sync(session.document(), session.revision());
        assert_eq!(context.elevation(), Ok(3500.));
        assert_eq!(context.recall.level, Some(levels[1]));
    }
    #[test]
    fn plan_filters_draw_and_pick_preserves_hidden_state_native_sources_and_geometry() {
        let (session, levels, bodies, features) = building();
        let scene = scene(&session, bodies, features);
        let ray = Ray::new([500., 400., 5000.], [0., 0., -1.]).unwrap();
        assert_eq!(scene.pick_face(ray).unwrap().instance, bodies[1]);
        let mut context = Context::default();
        context.sync(session.document(), session.revision());
        let mut camera = Camera::default();
        context.set_enabled(true, &mut camera);
        context.set_plan(true, &mut camera);
        let before = session.document().clone();
        let ground = context.scene(&scene).unwrap();
        assert_eq!(ground.instances().len(), 1);
        assert_eq!(ground.pick_face(ray).unwrap().instance, bodies[0]);
        assert_eq!(ground.reference_instances().len(), 2);
        assert!(ground.topology_snapshot(bodies[1]).is_some());
        for (id, geometry) in scene.geometries() {
            assert!(Arc::ptr_eq(geometry, &ground.geometries()[id]));
        }
        assert!(Arc::ptr_eq(&ground, &context.scene(&scene).unwrap()));
        context.recall.level = Some(levels[1]);
        let upper = context.scene(&scene).unwrap();
        assert_eq!(upper.pick_face(ray).unwrap().instance, bodies[1]);
        let already_hidden = scene.visible_subset(&BTreeSet::from([bodies[0]])).unwrap();
        assert!(
            context
                .scene(&already_hidden)
                .unwrap()
                .instances()
                .is_empty()
        );
        context.set_plan(false, &mut camera);
        assert!(Arc::ptr_eq(&scene, &context.scene(&scene).unwrap()));
        assert_eq!(session.document(), &before);
        assert!(
            Levels::new(session.document(), session.revision() + 1)
                .plan_scene(&scene, Some(levels[0]))
                .is_err()
        );
    }
    #[test]
    fn repeated_view_and_workspace_switches_restore_both_cameras_without_source_edits() {
        let (session, levels, _, _) = building();
        let before = session.document().clone();
        let mut context = Context::default();
        context.sync(session.document(), session.revision());
        let mut camera = Camera {
            yaw: 0.8,
            pitch: 0.4,
            scale: 0.1,
            target: [500., 400., 1200.],
            perspective: true,
        };
        let spatial = camera;
        context.set_enabled(true, &mut camera);
        context.set_plan(true, &mut camera);
        assert!(!camera.perspective);
        camera.target[0] = 700.;
        camera.scale = 0.2;
        context.recall.level = Some(levels[1]);
        context.constrain(&mut camera);
        assert_eq!(camera.target[2], 3000.);
        let plan = camera;
        for _ in 0..3 {
            context.set_plan(false, &mut camera);
            assert_eq!(camera, spatial);
            context.set_plan(true, &mut camera);
            assert_eq!(camera, plan);
            context.set_enabled(false, &mut camera);
            assert_eq!(camera, spatial);
            context.set_enabled(true, &mut camera);
            assert_eq!(camera, plan);
        }
        context.restore(
            Recall {
                plan: false,
                level: Some(levels[0]),
            },
            &mut camera,
        );
        assert_eq!(camera, spatial);
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn missing_recalled_level_reports_repair_instead_of_silently_using_the_first_level() {
        let (session, _, bodies, features) = building();
        let mut context = Context::default();
        let missing = EntityId::new();
        let mut camera = Camera::default();
        context.restore(
            Recall {
                plan: true,
                level: Some(missing),
            },
            &mut camera,
        );
        context.sync(session.document(), session.revision());
        context.set_enabled(true, &mut camera);
        assert_eq!(context.recall.level, Some(missing));
        assert!(context.elevation().unwrap_err().contains("missing"));
        assert!(context.scene(&scene(&session, bodies, features)).is_err());
    }
}
