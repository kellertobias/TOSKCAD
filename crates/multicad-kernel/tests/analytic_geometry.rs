use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{GeometryPolicy, PlaneFrame, RigidTransform, Solid};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn point_near(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        near(a, b);
    }
}

#[test]
fn box_descriptors_are_native_edges_vertices_and_outward_planes() {
    let id = EntityId::new();
    let solid = Solid::box_named([100., 60., 18.], id, GeometryPolicy::default()).unwrap();
    let snapshot = solid.topology_snapshot(id, 4).unwrap();
    let mut lengths = vec![];
    for e in &snapshot.elements {
        let geometry = e.geometry.expect("all box topology has exact geometry");
        geometry.validate().unwrap();
        let intent = GeometryIntent::capture(&snapshot, e.key).unwrap();
        let Resolution::Resolved {
            geometry: Some(resolved),
            ..
        } = resolve(&intent, &snapshot, 4)
        else {
            panic!("unresolved exact descriptor")
        };
        assert_eq!(resolved, geometry);
        match geometry {
            AnalyticGeometry::Line { a_mm, b_mm } => lengths.push(
                a_mm.iter()
                    .zip(b_mm)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
                    .sqrt(),
            ),
            AnalyticGeometry::Plane { frame } => {
                let face = solid
                    .faces()
                    .unwrap()
                    .into_iter()
                    .find(|f| f.index == e.key.ordinal)
                    .unwrap();
                let (origin, normal) = face.plane.unwrap();
                point_near(frame.origin_mm, origin);
                point_near(frame.normal(), normal);
            }
            AnalyticGeometry::Point { position_mm } => {
                assert!([0., 100.].contains(&position_mm[0]));
                assert!([0., 60.].contains(&position_mm[1]));
                assert!([0., 18.].contains(&position_mm[2]));
            }
            _ => panic!("box has no curved edges"),
        }
    }
    lengths.sort_by(f64::total_cmp);
    assert_eq!(
        lengths,
        vec![
            18., 18., 18., 18., 60., 60., 60., 60., 100., 100., 100., 100.
        ]
    );
}

#[test]
fn current_brep_descriptors_follow_named_source_edits_and_rigid_transforms() {
    let id = EntityId::new();
    let first = Solid::box_named([100., 60., 18.], id, GeometryPolicy::default())
        .unwrap()
        .topology_snapshot(id, 0)
        .unwrap();
    let top = first.elements.iter().find(|e| e.key.kind==TopologyKind::Edge && matches!(e.geometry,Some(AnalyticGeometry::Line {a_mm,b_mm}) if a_mm[2]==18.&&b_mm[2]==18.)).unwrap();
    let intent = GeometryIntent::capture(&first, top.key).unwrap();
    let edited = Solid::box_named([120., 60., 24.], id, GeometryPolicy::default()).unwrap();
    let transform = RigidTransform {
        rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
        translation_mm: [20., 30., 40.],
    };
    let moved = edited
        .transformed(transform)
        .unwrap()
        .topology_snapshot(id, 1)
        .unwrap();
    let Resolution::Resolved {
        geometry: Some(AnalyticGeometry::Line { a_mm, b_mm }),
        ..
    } = resolve(&intent, &moved, 1)
    else {
        panic!("edited line lost semantic source")
    };
    near(a_mm[2], 64.);
    near(b_mm[2], 64.);
    assert!(matches!(
        resolve(&intent, &moved, 0),
        Resolution::Stale { .. }
    ));
    // The descriptors are recomputed from the transformed native result rather
    // than inherited unchanged from the old source's coordinates.
    assert_ne!(Some(AnalyticGeometry::Line { a_mm, b_mm }), top.geometry);
}

#[test]
fn circles_keep_exact_radius_frame_and_trim_domain_independent_of_mesh() {
    let id = EntityId::new();
    let solid = Solid::cylinder(PlaneFrame::default(), 7., 25., GeometryPolicy::default()).unwrap();
    let first = solid.topology_snapshot(id, 0).unwrap();
    let circles: Vec<_> = first
        .elements
        .iter()
        .filter_map(|e| match e.geometry {
            Some(AnalyticGeometry::CircularArc {
                frame,
                radius_mm,
                start_rad,
                sweep_rad,
            }) => Some((e.key, frame, radius_mm, start_rad, sweep_rad)),
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 2);
    for (_, frame, radius, start, sweep) in circles {
        near(radius, 7.);
        near(start, 0.);
        near(sweep, std::f64::consts::TAU);
        assert!([0., 25.].contains(&frame.origin_mm[2]));
    }
    solid
        .mesh(multicad_kernel::MeshOptions {
            linear_deflection_mm: 0.8,
            ..Default::default()
        })
        .unwrap();
    let after = solid.topology_snapshot(id, 0).unwrap();
    assert_eq!(first, after);
}
