use multicad_core::{
    document::{EntityId, Placement},
    references::*,
};
use multicad_kernel::{GeometryPolicy, MeshOptions, Profile, ProfileEdge, Solid};
use multicad_ui::{
    camera::Camera,
    egui::{Pos2, Rect, Vec2},
    viewport::*,
};
use std::sync::Arc;
fn geometry(solid: &Solid, id: EntityId) -> Arc<Geometry> {
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    Arc::new(
        Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid.topology_snapshot(id, 0).unwrap().elements,
        )
        .unwrap(),
    )
}
fn box_geometry() -> Arc<Geometry> {
    let id = EntityId::new();
    geometry(
        &Solid::box_named([100., 60., 18.], id, GeometryPolicy::default()).unwrap(),
        id,
    )
}
fn instance(geometry: &Geometry, translation_mm: [f64; 3]) -> Instance {
    Instance {
        id: EntityId::new(),
        geometry: geometry.id(),
        transform: Transform::from_placement(Placement {
            translation_mm,
            ..Default::default()
        })
        .unwrap(),
        color: [150, 160, 170, 255],
    }
}
fn stamp() -> SceneStamp {
    SceneStamp {
        document: EntityId::new(),
        revision: 0,
        generation: 0,
    }
}
#[test]
fn closest_surface_wins_independently_of_instance_order_and_yields_semantic_intent() {
    let geometry = box_geometry();
    let far = instance(&geometry, [0.; 3]);
    let near = instance(&geometry, [0., 0., 30.]);
    let ray = Ray::new([20., 25., 100.], [0., 0., -1.]).unwrap();
    for instances in [
        vec![far.clone(), near.clone()],
        vec![near.clone(), far.clone()],
    ] {
        let scene = Scene::new(stamp(), vec![geometry.clone()], instances).unwrap();
        let hit = scene.pick_face(ray).unwrap();
        assert_eq!(hit.instance, near.id);
        assert_eq!(hit.point_mm, [20., 25., 48.]);
        let intent = hit.intent.unwrap();
        assert!(intent.origins.iter().any(|o| o.role == "cap.end"));
        assert!(
            matches!(resolve(&intent,&scene.topology_snapshot(near.id).unwrap(),0),Resolution::Resolved {key,..} if key==hit.key)
        );
    }
}
#[test]
fn instance_frames_are_in_world_coordinates_and_camera_rays_match_both_projections() {
    let geometry = box_geometry();
    let mut part = instance(&geometry, [10., 20., 30.]);
    part.transform = Transform::from_placement(Placement {
        translation_mm: [10., 20., 30.],
        rotation_xyzw: [
            0.,
            0.,
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
    })
    .unwrap();
    let scene = Scene::new(stamp(), vec![geometry], vec![part.clone()]).unwrap();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
    for perspective in [false, true] {
        let camera = Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale: 3.,
            target: [-15., 40., 48.],
            perspective,
        };
        let screen = camera.project([-15., 40., 48.], rect);
        let ray = camera.ray(Pos2::new(screen[0], screen[1]), rect).unwrap();
        let hit = scene.pick_face(ray).unwrap();
        assert_eq!(hit.instance, part.id);
        for (a, b) in hit.point_mm.into_iter().zip([-15., 40., 48.]) {
            assert!((a - b).abs() < 1e-4);
        }
        let mut intent = hit.intent.unwrap();
        let anchor = intent
            .origins
            .iter()
            .find(|o| o.role == "cap.end")
            .unwrap()
            .clone();
        intent.attachment = Some(Attachment {
            anchor,
            offset_mm: [20., 25., 0.],
            angle_rad: 0.,
        });
        let Resolution::Resolved {
            frame: Some(frame), ..
        } = resolve(&intent, &scene.topology_snapshot(part.id).unwrap(), 0)
        else {
            panic!("world attachment missing")
        };
        for (a, b) in frame.origin_mm.into_iter().zip([-15., 40., 48.]) {
            assert!((a - b).abs() < 1e-10);
        }
    }
}
#[test]
fn picking_respects_real_profile_holes_and_curved_faces() {
    let id = EntityId::new();
    let mut profile = Profile::rectangle(40., 30.);
    profile.holes.push(vec![ProfileEdge::Circle {
        center: [20., 15.],
        radius_mm: 4.,
    }]);
    let curves = (0..5)
        .map(|i| Origin {
            source: id,
            role: format!("curve{i}"),
        })
        .collect::<Vec<_>>();
    let solid =
        Solid::extrude_profile_named(&profile, 8., id, &curves, Default::default()).unwrap();
    let geometry = geometry(&solid, id);
    let part = instance(&geometry, [0.; 3]);
    let scene = Scene::new(stamp(), vec![geometry], vec![part]).unwrap();
    assert!(
        scene
            .pick_face(Ray::new([20., 15., 100.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let hit = scene
        .pick_face(Ray::new([5., 5., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(hit.point_mm, [5., 5., 8.]);
    let hit = scene
        .pick_face(Ray::new([20., 15., 4.], [1., 0., 0.]).unwrap())
        .unwrap();
    assert!((hit.distance_mm - 4.).abs() < 0.02);
    assert!(hit.intent.unwrap().origins.contains(&curves[4]));
}
#[test]
fn one_thousand_instances_share_one_native_mesh_and_acceleration_structure() {
    let id = EntityId::new();
    let geometry = geometry(
        &Solid::box_named([10., 12., 8.], id, Default::default()).unwrap(),
        id,
    );
    let instances = (0..1000)
        .map(|i| {
            instance(
                &geometry,
                [(i % 32) as f64 * 20., (i / 32) as f64 * 20., 0.],
            )
        })
        .collect::<Vec<_>>();
    let ids = instances.iter().map(|i| i.id).collect::<Vec<_>>();
    let scene = Scene::new(stamp(), vec![geometry.clone()], instances).unwrap();
    assert_eq!(scene.geometries().len(), 1);
    assert!(Arc::ptr_eq(&geometry, &scene.geometries()[&id]));
    assert!(scene.storage_bytes() < 512_000);
    let mut timings = vec![];
    for i in (0..1000).step_by(7) {
        let start = std::time::Instant::now();
        let hit = scene
            .pick_face(
                Ray::new(
                    [(i % 32) as f64 * 20. + 5., (i / 32) as f64 * 20. + 6., 100.],
                    [0., 0., -1.],
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(hit.instance, ids[i]);
        timings.push(start.elapsed().as_secs_f64() * 1000.);
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "1000 native-box instances, shared geometry, headless CPU pick p95 {:.3} ms; mesh/BVH/instance payload {} bytes (not RSS or GPU acceptance)",
        timings[timings.len() * 95 / 100],
        scene.storage_bytes()
    );
}
#[test]
fn old_revision_generation_and_foreign_document_cannot_replace_visible_scene() {
    let first = stamp();
    let second = SceneStamp {
        revision: 1,
        generation: 1,
        ..first
    };
    let third = SceneStamp {
        generation: 2,
        ..second
    };
    let scene = |stamp| Arc::new(Scene::new(stamp, vec![], vec![]).unwrap());
    let old = scene(first);
    let mut store = SceneStore::default();
    store.begin(first).unwrap();
    store.publish(old.clone()).unwrap();
    store.begin(second).unwrap();
    assert!(!store.is_current());
    assert!(Arc::ptr_eq(store.visible().unwrap(), &old));
    assert!(store.publish(scene(first)).is_err());
    store.begin(third).unwrap();
    assert!(store.publish(scene(second)).is_err());
    assert!(store.begin(first).is_err());
    assert!(
        store
            .begin(SceneStamp {
                document: EntityId::new(),
                ..third
            })
            .is_err()
    );
    store.publish(scene(third)).unwrap();
    assert!(store.is_current());
}
#[test]
fn malformed_meshes_placements_and_rays_fail_and_unnamed_faces_remain_body_only() {
    let id = EntityId::new();
    assert!(Geometry::new(id, vec![[0.; 3]], vec![0, 1, 0], vec![0], vec![]).is_err());
    assert!(Geometry::new(id, vec![[f64::NAN; 3]], vec![0, 0, 0], vec![0], vec![]).is_err());
    assert!(Ray::new([0.; 3], [0.; 3]).is_err());
    assert!(Ray::new([0.; 3], [f64::MAX; 3]).is_err());
    assert!(
        Transform::from_placement(Placement {
            rotation_xyzw: [0.; 4],
            ..Default::default()
        })
        .is_err()
    );
    let unnamed = geometry(&Solid::box_solid([10.; 3], Default::default()).unwrap(), id);
    let part = instance(&unnamed, [0.; 3]);
    let scene = Scene::new(stamp(), vec![unnamed], vec![part.clone()]).unwrap();
    let hit = scene
        .pick_face(Ray::new([5., 5., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(hit.instance, part.id);
    assert!(hit.intent.is_none());
    assert!(Scene::new(stamp(), vec![], vec![part]).is_err());
}
