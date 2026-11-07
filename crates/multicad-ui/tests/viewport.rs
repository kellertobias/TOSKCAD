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
        .unwrap()
        .with_topology_display(
            solid
                .display_edges(0.1)
                .unwrap()
                .into_iter()
                .map(|e| DisplayEdge {
                    index: e.index,
                    points_mm: e.points_mm,
                })
                .collect(),
            solid
                .display_vertices()
                .unwrap()
                .into_iter()
                .map(|v| DisplayPoint {
                    index: v.index,
                    position_mm: v.position_mm,
                })
                .collect(),
            0.1,
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
fn transfer_counts_require_matching_complete_receipts_and_never_escape_source_inspection() {
    use multicad_core::{
        model::{ModelEvaluationInfo, TransferEvaluationInfo},
        project_file::ContentHash,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let geometry = box_geometry();
    let placed = instance(&geometry, [0.; 3]);
    let source = ContentHash::of(b"counted native candidate");
    let make = || {
        Scene::new(stamp(), vec![geometry.clone()], vec![placed.clone()])
            .unwrap()
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                screw_connection_checks: vec![],
                source_hash: source,
                consumed_bodies: BTreeSet::new(),
                cut_targets: vec![],
            }))
            .unwrap()
    };
    let info = TransferEvaluationInfo {
        source_hash: source,
        counts: BTreeMap::from([(geometry.id(), 0)]),
    };
    let scene = make()
        .with_transfer_evaluation(Arc::new(info.clone()))
        .unwrap();
    assert_eq!(
        scene.transfer_evaluation().unwrap().counts[&geometry.id()],
        0
    );
    let hidden = scene.visible_subset(&BTreeSet::new()).unwrap();
    assert_eq!(hidden.transfer_evaluation().unwrap().counts, info.counts);
    let inspected = scene.isolated_source(geometry.id()).unwrap();
    assert!(inspected.transfer_evaluation().is_none());
    let mut bad = info.clone();
    bad.source_hash = ContentHash::of(b"other source");
    assert!(make().with_transfer_evaluation(Arc::new(bad)).is_err());
    let mut bad = info.clone();
    bad.counts = BTreeMap::from([(EntityId::new(), 1)]);
    assert!(make().with_transfer_evaluation(Arc::new(bad)).is_err());
    let mut bad = info.clone();
    bad.counts.insert(geometry.id(), 257);
    assert!(make().with_transfer_evaluation(Arc::new(bad)).is_err());
    assert!(
        make()
            .with_source_scope(vec![geometry.id()])
            .unwrap()
            .with_transfer_evaluation(Arc::new(info))
            .is_err()
    );
}
#[test]
fn explicit_leaf_inspection_uses_the_complete_path_receipt_instead_of_its_outer_instance_pose() {
    use multicad_core::{
        model::{AssemblyEvaluationInfo, AssemblyLeafReceipt, OccurrencePath, RigidPose},
        project_file::ContentHash,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let geometry = box_geometry();
    let root = instance(&geometry, [100., 0., 0.]);
    let component = EntityId::new();
    let path = OccurrencePath {
        root: root.id,
        members: vec![EntityId::new(), EntityId::new()],
    };
    let pose = RigidPose {
        rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
        translation_mm: [35., 4., 9.],
    };
    let source_hash = ContentHash::of(b"complete native leaf source");
    let scene = Scene::new(stamp(), vec![geometry.clone()], vec![root.clone()])
        .unwrap()
        .with_reference_aliases(BTreeMap::from([(component, geometry.id())]))
        .unwrap()
        .with_model_evaluation(Arc::new(multicad_core::model::ModelEvaluationInfo {
            screw_connection_checks: vec![],
            source_hash,
            consumed_bodies: BTreeSet::new(),
            cut_targets: vec![],
        }))
        .unwrap()
        .with_assembly_evaluation(Arc::new(AssemblyEvaluationInfo {
            source_hash,
            leaves: vec![AssemblyLeafReceipt {
                path: path.clone(),
                component,
                producer: geometry.id(),
                pose,
            }],
        }))
        .unwrap();
    let view = scene.isolated_occurrence_source(&path).unwrap();
    assert!(!view.is_full_model());
    assert!(view.assembly_evaluation().is_none());
    assert_eq!(view.reference_instances().len(), 1);
    let inspected = &view.reference_instances()[0];
    assert_eq!(inspected.id, root.id);
    assert_eq!(inspected.geometry, geometry.id());
    assert_eq!(inspected.transform.point([0.; 3]), [35., 4., 9.]);
    assert_eq!(inspected.transform.point([1., 0., 0.]), [35., 5., 9.]);
    assert_eq!(root.transform.point([0.; 3]), [100., 0., 0.]);
    let mut unknown = path;
    unknown.members.pop();
    assert!(scene.isolated_occurrence_source(&unknown).is_err());
    assert!(view.isolated_occurrence_source(&unknown).is_err());
}
#[test]
fn assembly_receipts_require_current_complete_known_leaf_sources() {
    use multicad_core::{
        model::{AssemblyEvaluationInfo, AssemblyLeafReceipt, OccurrencePath},
        project_file::ContentHash,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let geometry = box_geometry();
    let part = instance(&geometry, [0.; 3]);
    let component = EntityId::new();
    let source = ContentHash::of(b"assembly source");
    let make = || {
        Scene::new(stamp(), vec![geometry.clone()], vec![part.clone()])
            .unwrap()
            .with_reference_aliases(BTreeMap::from([(component, geometry.id())]))
            .unwrap()
            .with_model_evaluation(Arc::new(multicad_core::model::ModelEvaluationInfo {
                screw_connection_checks: vec![],
                source_hash: source,
                consumed_bodies: BTreeSet::new(),
                cut_targets: vec![],
            }))
            .unwrap()
    };
    let info = AssemblyEvaluationInfo {
        source_hash: source,
        leaves: vec![AssemblyLeafReceipt {
            path: OccurrencePath {
                root: part.id,
                members: vec![],
            },
            component,
            producer: geometry.id(),
            pose: Default::default(),
        }],
    };
    let scene = make()
        .with_assembly_evaluation(Arc::new(info.clone()))
        .unwrap();
    assert_eq!(scene.assembly_evaluation().unwrap().as_ref(), &info);
    assert!(
        scene
            .visible_subset(&BTreeSet::new())
            .unwrap()
            .assembly_evaluation()
            .is_some()
    );
    assert!(
        scene
            .presentation_offsets(&BTreeMap::new())
            .unwrap()
            .assembly_evaluation()
            .is_none()
    );
    let mut bad = info.clone();
    bad.source_hash = ContentHash::of(b"changed source");
    assert!(make().with_assembly_evaluation(Arc::new(bad)).is_err());
    let mut bad = info.clone();
    bad.leaves[0].component = EntityId::new();
    assert!(make().with_assembly_evaluation(Arc::new(bad)).is_err());
    let mut bad = info.clone();
    bad.leaves.push(bad.leaves[0].clone());
    assert!(make().with_assembly_evaluation(Arc::new(bad)).is_err());
    assert!(
        make()
            .with_source_scope(vec![geometry.id()])
            .unwrap()
            .with_assembly_evaluation(Arc::new(info))
            .is_err()
    );
}
#[test]
fn exploded_presentation_keeps_source_geometry_and_cannot_author_native_operations() {
    use std::collections::BTreeMap;
    let geometry = box_geometry();
    let part = instance(&geometry, [10., 20., 30.]);
    let original = Arc::new(Scene::new(stamp(), vec![geometry], vec![part.clone()]).unwrap());
    let original_topology = original.topology_snapshot(part.id).unwrap();
    let offsets = BTreeMap::from([(part.id, [200., 0., 0.])]);
    let view = original.presentation_offsets(&offsets).unwrap();
    assert_eq!(view.scope(), &SceneScope::Presentation);
    assert!(!view.is_full_model());
    assert!(view.model_evaluation().is_none());
    assert!(view.native_instance_bounds(part.id).is_none());
    assert_eq!(view.instance_bounds(part.id).unwrap()[0], [210., 20., 30.]);
    assert_eq!(
        original.instance_bounds(part.id).unwrap()[0],
        [10., 20., 30.]
    );
    assert_eq!(view.topology_snapshot(part.id).unwrap(), original_topology);
    assert!(
        view.pick_face(Ray::new([220., 30., 100.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let mut store = SceneStore::default();
    store.begin(original.stamp()).unwrap();
    store.publish(original.clone()).unwrap();
    assert!(store.publish(view.clone()).is_err());
    assert!(Arc::ptr_eq(store.visible().unwrap(), &original));
    assert!(view.presentation_offsets(&offsets).is_err());
    assert!(
        original
            .presentation_offsets(&BTreeMap::from([(EntityId::new(), [0.; 3])]))
            .is_err()
    );
    assert!(
        original
            .presentation_offsets(&BTreeMap::from([(part.id, [f64::NAN; 3])]))
            .is_err()
    );
}
#[test]
fn display_detail_follows_projected_size_and_preserves_selection_intent() {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
    let bounds = [[0., 0., 0.], [100., 60., 18.]];
    let mut camera = Camera {
        scale: 0.1,
        ..Default::default()
    };
    assert_eq!(
        DisplayQuality::for_view(camera, rect, bounds),
        Some(DisplayQuality::Distant)
    );
    camera.scale = 1.;
    assert_eq!(
        DisplayQuality::for_view(camera, rect, bounds),
        Some(DisplayQuality::Coarse)
    );
    camera.scale = 20.;
    assert_eq!(
        DisplayQuality::for_view(camera, rect, bounds),
        Some(DisplayQuality::Fine)
    );
    camera.scale = 100.;
    assert_eq!(
        DisplayQuality::for_view(camera, rect, bounds),
        Some(DisplayQuality::Detail)
    );
    camera.perspective = true;
    camera.target = [50., 30., 9.];
    assert!(DisplayQuality::for_view(camera, rect, bounds).unwrap() >= DisplayQuality::Detail);
    assert!(DisplayQuality::for_view(camera, Rect::NOTHING, bounds).is_none());
    assert!(DisplayQuality::for_view(camera, rect, [[f64::NAN; 3]; 2]).is_none());

    let geo = box_geometry();
    let part = instance(&geo, [0.; 3]);
    let first = Scene::new(stamp(), vec![geo.clone()], vec![part.clone()]).unwrap();
    let hit = first
        .pick_face(Ray::new([20., 25., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    let second = Scene::new(
        SceneStamp {
            generation: 1,
            ..first.stamp()
        },
        vec![geo.clone()],
        vec![part.clone()],
    )
    .unwrap();
    let refreshed = second.refresh_hit(&hit).unwrap();
    assert_eq!(refreshed.stamp, second.stamp());
    assert_eq!(refreshed.intent, hit.intent);
    assert_eq!(refreshed.key, hit.key);
    let changed = Scene::new(
        SceneStamp {
            revision: 1,
            generation: 2,
            ..first.stamp()
        },
        vec![geo],
        vec![part],
    )
    .unwrap();
    assert!(changed.refresh_hit(&hit).is_none());
}
#[test]
fn reference_inspection_keeps_hidden_producers_and_can_isolate_placed_or_raw_sources() {
    let geometry = box_geometry();
    let placed = instance(&geometry, [10., 20., 30.]);
    let source = geometry.id();
    let scene = Scene::new(stamp(), vec![geometry], vec![placed.clone()]).unwrap();
    let snapshots = scene.reference_snapshots();
    assert_eq!(snapshots[&source].owner, source);
    let cap = |snapshot: &TopologySnapshot| {
        snapshot
            .elements
            .iter()
            .find(|e| {
                e.origins.iter().any(|o| o.role == "cap.end") && e.key.kind == TopologyKind::Face
            })
            .unwrap()
            .frames[0]
            .frame
            .origin_mm
    };
    assert_eq!(cap(&snapshots[&source]), [0., 0., 18.]);
    assert_eq!(cap(&snapshots[&placed.id]), [10., 20., 48.]);
    let raw = scene.isolated_source(source).unwrap();
    assert_eq!(raw.instances()[0].id, source);
    let ray = Ray::new([20., 25., 100.], [0., 0., -1.]).unwrap();
    assert_eq!(raw.pick_face(ray).unwrap().point_mm[2], 18.);
    let world = scene.isolated_source(placed.id).unwrap();
    assert_eq!(world.pick_face(ray).unwrap().point_mm[2], 48.);
    assert_eq!(scene.instances().len(), 1);
    assert_eq!(scene.instances()[0].id, placed.id);
    assert!(scene.isolated_source(EntityId::new()).is_err());
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
fn section_picking_skips_removed_surfaces_and_clips_edges_without_new_source_faces() {
    assert!(SectionPlane::new([0.; 3], 0.).is_err());
    assert!(SectionPlane::new([1., 0., 0.], f64::NAN).is_err());
    let below = SectionPlane::new([0., 0., -2.], -9.).unwrap();
    assert_eq!(
        below.clip_segment([0., 0., 0.], [0., 0., 18.]),
        Some([[0., 0., 0.], [0., 0., 9.]])
    );
    assert_eq!(
        below.clip_segment([0., 0., 18.], [0., 0., 0.]),
        Some([[0., 0., 9.], [0., 0., 0.]])
    );
    assert!(below.clip_segment([0., 0., 10.], [0., 0., 20.]).is_none());
    let geometry = box_geometry();
    let part = instance(&geometry, [0.; 3]);
    let scene = Scene::new(stamp(), vec![geometry], vec![part.clone()]).unwrap();
    let ray = Ray::new([20., 25., 100.], [0., 0., -1.]).unwrap();
    assert_eq!(scene.pick_face(ray).unwrap().point_mm[2], 18.);
    let bottom = scene.pick_face_section(ray, Some(below)).unwrap();
    assert_eq!(bottom.point_mm, [20., 25., 0.]); // Continue past removed front triangles.
    assert!(
        bottom
            .intent
            .unwrap()
            .origins
            .iter()
            .any(|o| o.role == "cap.start")
    );
    assert_eq!(scene.stamp().revision, 0);
    assert!(
        scene
            .pick_face_section(ray, Some(SectionPlane::new([1., 0., 0.], 50.).unwrap()))
            .is_none()
    );
    assert!(
        scene
            .pick_face_section(
                Ray::new([20., 25., 4.], [1., 0., 0.]).unwrap(),
                Some(SectionPlane::new([0., 0., 1.], 9.).unwrap())
            )
            .is_none()
    );
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        target: [50., 30., 18.],
        scale: 3.,
        perspective: false,
    };
    let right = SectionPlane::new([1., 0., 0.], 50.).unwrap();
    for filter in [
        PickFilter::Auto,
        PickFilter::Parts,
        PickFilter::Faces,
        PickFilter::Edges,
        PickFilter::Vertices,
    ] {
        let p = camera.project([0., 0., 18.], rect);
        assert!(
            scene
                .pick_screen_with(
                    camera,
                    rect,
                    Pos2::new(p[0], p[1]),
                    PickOptions {
                        filter,
                        include_hidden: true,
                        section: Some(right)
                    }
                )
                .is_none()
        );
    }
    let p = camera.project([70., 0., 18.], rect);
    let edge = scene
        .pick_screen_with(
            camera,
            rect,
            Pos2::new(p[0], p[1]),
            PickOptions {
                filter: PickFilter::Edges,
                include_hidden: false,
                section: Some(right),
            },
        )
        .unwrap();
    assert_eq!(edge.key.kind, TopologyKind::Edge);
    assert!((edge.point_mm[0] - 70.).abs() < 1e-4);
    assert_eq!(edge.instance, part.id);
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
    let [low, high] = scene.instance_bounds(part.id).unwrap();
    let world = scene.topology_snapshot(part.id).unwrap();
    for element in &world.elements {
        let points = match element.geometry.unwrap() {
            multicad_core::references::AnalyticGeometry::Point { position_mm } => vec![position_mm],
            multicad_core::references::AnalyticGeometry::Line { a_mm, b_mm } => vec![a_mm, b_mm],
            multicad_core::references::AnalyticGeometry::Plane { frame } => {
                frame.validate().unwrap();
                vec![frame.origin_mm]
            }
            _ => panic!("box descriptor must be planar or linear"),
        };
        for point in points {
            for i in 0..3 {
                assert!(point[i] >= low[i] - 1e-9 && point[i] <= high[i] + 1e-9);
            }
        }
    }
    for (actual, expected) in low
        .into_iter()
        .zip([-50., 20., 30.])
        .chain(high.into_iter().zip([10., 120., 48.]))
    {
        assert!((actual - expected).abs() < 1e-10);
    }
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
#[test]
fn screen_picking_uses_real_vertices_edges_and_faces_with_occlusion() {
    let geometry = box_geometry();
    let far = instance(&geometry, [0.; 3]);
    let near = instance(&geometry, [-10., -10., 30.]);
    let scene = Scene::new(stamp(), vec![geometry], vec![far.clone(), near.clone()]).unwrap();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 3.,
        target: [40., 20., 20.],
        perspective: false,
    };
    let screen = |p| {
        let p = camera.project(p, rect);
        Pos2::new(p[0], p[1])
    };
    let corner = screen([0., 0., 18.]);
    assert!(
        scene
            .pick_screen(camera, rect, corner, PickFilter::Vertices, false)
            .is_none()
    );
    let hidden = scene
        .pick_screen(camera, rect, corner, PickFilter::Vertices, true)
        .unwrap();
    assert_eq!(hidden.instance, far.id);
    assert_eq!(hidden.key.kind, TopologyKind::Vertex);
    assert!(hidden.intent.is_some());
    let automatic = scene
        .pick_screen(camera, rect, corner, PickFilter::Auto, false)
        .unwrap();
    assert_eq!(automatic.instance, near.id);
    assert_eq!(automatic.key.kind, TopologyKind::Face);
    let vertex = scene
        .pick_screen(
            camera,
            rect,
            screen([-10., -10., 48.]) + Vec2::new(2., 2.),
            PickFilter::Auto,
            false,
        )
        .unwrap();
    assert_eq!(vertex.instance, near.id);
    assert_eq!(vertex.key.kind, TopologyKind::Vertex);
    let edge = scene
        .pick_screen(
            camera,
            rect,
            screen([30., -10., 48.]),
            PickFilter::Auto,
            false,
        )
        .unwrap();
    assert_eq!(edge.instance, near.id);
    assert_eq!(edge.key.kind, TopologyKind::Edge);
    assert!(edge.intent.is_some());
    assert!(
        matches!(resolve(edge.intent.as_ref().unwrap(),&scene.topology_snapshot(near.id).unwrap(),0),Resolution::Resolved {key,..} if key==edge.key)
    );
}
#[test]
fn perspective_edge_hit_reprojects_to_the_pointer_and_display_data_is_validated() {
    let geometry = box_geometry();
    let part = instance(&geometry, [0.; 3]);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
    let camera = Camera {
        yaw: -0.6,
        pitch: 0.6,
        scale: 3.,
        target: [50., 30., 18.],
        perspective: true,
    };
    let scene = Scene::new(stamp(), vec![geometry.clone()], vec![part]).unwrap();
    for edge in geometry.edges() {
        let a = edge.points_mm[0];
        let b = edge.points_mm[1];
        let point = std::array::from_fn(|i| (a[i] * 0.73 + b[i] * 0.27) as f32);
        let screen = camera.project(point, rect);
        let pointer = Pos2::new(screen[0], screen[1]);
        let hit = scene
            .pick_screen(camera, rect, pointer, PickFilter::Edges, true)
            .unwrap();
        let projected = camera.project(hit.point_mm.map(|v| v as f32), rect);
        assert!((Pos2::new(projected[0], projected[1]) - pointer).length() < 0.01);
    }
    let id = EntityId::new();
    let solid = Solid::box_named([10.; 3], id, Default::default()).unwrap();
    let mesh = solid.mesh(Default::default()).unwrap();
    let fresh = || {
        Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices.clone(),
            mesh.face_indices.clone(),
            solid.topology_snapshot(id, 0).unwrap().elements,
        )
        .unwrap()
    };
    assert!(
        fresh()
            .with_topology_display(
                vec![DisplayEdge {
                    index: 999,
                    points_mm: vec![[0.; 3], [1.; 3]]
                }],
                vec![],
                0.1
            )
            .is_err()
    );
    assert!(
        fresh()
            .with_topology_display(
                vec![],
                vec![DisplayPoint {
                    index: 0,
                    position_mm: [f64::NAN; 3]
                }],
                0.1
            )
            .is_err()
    );
}
#[test]
fn floor_grid_follows_distant_views_is_bounded_and_keeps_snap_unit_multiples() {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
    for scale in [0.02, 0.2, 3., 100.] {
        let camera = Camera {
            target: [10000., 20000., 0.],
            scale,
            ..Default::default()
        };
        let lines = floor_grid(camera, rect, 10.);
        assert!(!lines.is_empty());
        assert!(lines.len() <= 402);
        assert!(lines.iter().flatten().flatten().all(|v| v.is_finite()));
        let low = lines
            .iter()
            .flatten()
            .map(|p| p[0])
            .fold(f64::INFINITY, f64::min);
        let high = lines
            .iter()
            .flatten()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(low < 10000. && high > 10000.);
        for line in lines {
            for p in line {
                for value in p {
                    assert!((value / 10. - (value / 10.).round()).abs() < 1e-8);
                }
            }
        }
    }
    assert!(
        floor_grid(
            Camera {
                pitch: 0.,
                ..Default::default()
            },
            rect,
            10.
        )
        .is_empty()
    );
    assert!(floor_grid(Camera::default(), rect, f64::NAN).is_empty());
}

#[test]
fn evaluation_metadata_is_shared_without_relabeling_across_visibility_and_stale_display() {
    use multicad_core::{model::*, project_file::ContentHash};
    use std::collections::BTreeSet;
    let geometry = box_geometry();
    let placed = instance(&geometry, [0.; 3]);
    let consumed = EntityId::new();
    let stamp = stamp();
    let body = BodyInput {
        body: placed.id,
        producer: geometry.id(),
    };
    let info = Arc::new(ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::of(b"Exact immutable source snapshot"),
        consumed_bodies: BTreeSet::from([consumed]),
        cut_targets: vec![
            CutTargetProbeResult {
                id: EntityId::new(),
                result: Ok(CutTargetSelection {
                    defaults: vec![body.clone()],
                    targets: vec![body],
                    not_intersecting: vec![],
                }),
            },
            CutTargetProbeResult {
                id: EntityId::new(),
                result: Err(ModelQueryError {
                    entity: Some(consumed),
                    message: "Tool is consumed".into(),
                }),
            },
        ],
    });
    let scene = Arc::new(
        Scene::new(stamp, vec![geometry.clone()], vec![placed.clone()])
            .unwrap()
            .with_model_evaluation(info.clone())
            .unwrap(),
    );
    assert!(Arc::ptr_eq(scene.model_evaluation().unwrap(), &info));
    let hidden = scene.visible_subset(&BTreeSet::new()).unwrap();
    assert!(hidden.instances().is_empty());
    assert_eq!(hidden.reference_instances().len(), 1);
    assert_eq!(hidden.stamp(), stamp);
    assert!(Arc::ptr_eq(hidden.model_evaluation().unwrap(), &info));
    let visible = hidden.visible_subset(&BTreeSet::from([placed.id])).unwrap();
    assert!(Arc::ptr_eq(visible.model_evaluation().unwrap(), &info));
    assert!(
        scene
            .isolated_source(geometry.id())
            .unwrap()
            .model_evaluation()
            .is_none(),
        "A raw upstream repair view is not a canonical evaluation output"
    );
    let mut store = SceneStore::default();
    store.begin(stamp).unwrap();
    store.publish(hidden.clone()).unwrap();
    store
        .begin(SceneStamp {
            revision: stamp.revision + 1,
            generation: stamp.generation + 1,
            ..stamp
        })
        .unwrap();
    assert!(!store.is_current());
    assert_eq!(store.visible().unwrap().stamp(), stamp);
    assert!(Arc::ptr_eq(
        store.visible().unwrap().model_evaluation().unwrap(),
        &info
    ));
    assert!(store.publish(visible).is_err());
    assert!(Arc::ptr_eq(store.visible().unwrap(), &hidden));
}

#[test]
fn scene_rejects_malformed_or_contradictory_evaluation_metadata() {
    use multicad_core::{model::*, project_file::ContentHash};
    use std::collections::BTreeSet;
    let geometry = box_geometry();
    let placed = instance(&geometry, [0.; 3]);
    let build = |info| {
        Scene::new(stamp(), vec![geometry.clone()], vec![placed.clone()])
            .unwrap()
            .with_model_evaluation(Arc::new(info))
    };
    let empty = ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::of(b"source"),
        consumed_bodies: BTreeSet::new(),
        cut_targets: vec![],
    };
    assert!(
        Scene::new(stamp(), vec![], vec![])
            .unwrap()
            .model_evaluation()
            .is_none()
    );
    let mut contradictory = empty.clone();
    contradictory.consumed_bodies.insert(placed.id);
    assert!(build(contradictory.clone()).is_err());
    let hidden = Scene::new(stamp(), vec![geometry.clone()], vec![placed.clone()])
        .unwrap()
        .visible_subset(&BTreeSet::new())
        .unwrap();
    // Consumed membership is checked against all semantic occurrences, including
    // those omitted by the current visibility view.
    let hidden = Arc::try_unwrap(hidden).ok().unwrap();
    assert!(
        hidden
            .with_model_evaluation(Arc::new(contradictory))
            .is_err()
    );
    let probe = CutTargetProbeResult {
        id: EntityId::new(),
        result: Ok(CutTargetSelection {
            defaults: vec![],
            targets: vec![],
            not_intersecting: vec![],
        }),
    };
    let mut duplicate = empty.clone();
    duplicate.cut_targets = vec![probe.clone(), probe.clone()];
    assert!(build(duplicate).is_err());
    let mut excessive = empty.clone();
    excessive.cut_targets = (0..=MAX_CUT_TARGET_PROBES)
        .map(|_| CutTargetProbeResult {
            id: EntityId::new(),
            ..probe.clone()
        })
        .collect();
    assert!(build(excessive).is_err());
    let mut nil = empty;
    nil.cut_targets = vec![CutTargetProbeResult {
        id: EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap(),
        ..probe
    }];
    assert!(build(nil).is_err());
}

#[test]
fn per_face_material_palette_preserves_native_picking_and_rejects_unknown_faces() {
    let id = EntityId::new();
    let solid = Solid::box_named([100., 60., 18.], id, GeometryPolicy::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let topology = solid.topology_snapshot(id, 0).unwrap();
    let face = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face)
        .unwrap()
        .key
        .ordinal;
    let make = || {
        Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices.clone(),
            mesh.face_indices.clone(),
            topology.elements.clone(),
        )
        .unwrap()
    };
    let colored = make()
        .with_face_colors(std::collections::BTreeMap::from([(
            face,
            [180, 90, 40, 255],
        )]))
        .unwrap();
    assert_eq!(colored.face_color(face), Some([180, 90, 40, 255]));
    assert_eq!(colored.triangle_faces(), mesh.face_indices);
    assert_eq!(colored.face_color(u32::MAX), None);
    assert!(
        make()
            .with_face_colors(std::collections::BTreeMap::from([(u32::MAX, [0; 4])]))
            .is_err()
    );
}
