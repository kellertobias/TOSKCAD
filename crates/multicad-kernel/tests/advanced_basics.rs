use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn side(solid: &Solid, owner: EntityId, normal: [f64; 3]) -> TopologyKey {
    solid.topology_snapshot(owner,0).unwrap().elements.into_iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame}) if frame.normal().into_iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).unwrap().key
}
#[test]
fn asymmetric_chamfer_uses_the_explicit_face_and_distance_angle() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, GeometryPolicy::default()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let key=before.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm:a,b_mm:b}) if a[0]==0. && b[0]==0. && a[1]==0. && b[1]==0.)).unwrap().key;
    let mut selection = SidedTreatmentEdge {
        edge: TreatmentEdge {
            key,
            origin: Origin {
                source: feature,
                role: "edge/explicit".into(),
            },
        },
        reference_face: side(&source, owner, [-1., 0., 0.]),
    };
    for (face, ratio) in [([-1., 0., 0.], 0.5), ([0., -1., 0.], 2.)] {
        selection.reference_face = side(&source, owner, face);
        let result = source
            .chamfer_edges_sided_named(
                std::slice::from_ref(&selection),
                SidedChamfer::Distances {
                    first_mm: 2.,
                    second_mm: 4.,
                },
                feature,
                GeometryPolicy::default(),
            )
            .unwrap();
        near(result.volume_mm3().unwrap(), 72000. - 120.);
        let snapshot = result.topology_snapshot(feature, 0).unwrap();
        let face = snapshot
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face && e.origins.contains(&selection.edge.origin)
            })
            .unwrap();
        let Some(AnalyticGeometry::Plane { frame }) = face.geometry else {
            panic!("chamfer face is not planar")
        };
        near((frame.normal()[0] / frame.normal()[1]).abs(), ratio);
        GeometryIntent::capture(&snapshot, face.key).unwrap();
    }
    selection.reference_face = side(&source, owner, [-1., 0., 0.]);
    let angled = source
        .chamfer_edges_sided_named(
            std::slice::from_ref(&selection),
            SidedChamfer::DistanceAngle {
                distance_mm: 2.,
                angle_rad: std::f64::consts::FRAC_PI_4,
            },
            feature,
            GeometryPolicy::default(),
        )
        .unwrap();
    near(angled.volume_mm3().unwrap(), 72000. - 60.);
    selection.reference_face = side(&source, owner, [1., 0., 0.]);
    assert!(
        source
            .chamfer_edges_sided_named(
                &[selection],
                SidedChamfer::Distances {
                    first_mm: 2.,
                    second_mm: 4.
                },
                feature,
                GeometryPolicy::default()
            )
            .err()
            .unwrap()
            .message
            .contains("adjacent")
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}
#[test]
fn reflection_preserves_volume_source_and_right_handed_outward_plane_anchors() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, GeometryPolicy::default()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let key = side(&source, owner, [1., 0., 0.]);
    let face = before.elements.iter().find(|e| e.key == key).unwrap();
    let anchor = face.frames[0].origin.clone();
    let mut intent = GeometryIntent::capture(&before, key).unwrap();
    intent.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let result = source
        .mirrored_named(
            PlaneFrame {
                origin: [10., 0., 0.],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            feature,
            GeometryPolicy::default(),
        )
        .unwrap();
    near(result.volume_mm3().unwrap(), 72000.);
    near(result.bounds().unwrap().min[0], -40.);
    near(result.bounds().unwrap().max[0], 20.);
    let snapshot = result.topology_snapshot(feature, 1).unwrap();
    let Resolution::Resolved {
        frame: Some(frame),
        geometry: Some(AnalyticGeometry::Plane { frame: analytic }),
        ..
    } = resolve(&intent, &snapshot, 1)
    else {
        panic!("reflection lost plane ancestry")
    };
    assert_eq!(frame, analytic);
    near(frame.normal()[0], -1.);
    near(frame.origin_mm[0], -40.);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mirror.step");
    result.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    near(loaded.volume_mm3().unwrap(), result.volume_mm3().unwrap());
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
    assert!(
        AffineTransform {
            matrix: [[-1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [0.; 3]
        }
        .validate()
        .is_err()
    );
    assert!(
        source
            .mirrored_named(
                PlaneFrame {
                    u: [0.; 3],
                    ..Default::default()
                },
                feature,
                GeometryPolicy::default()
            )
            .is_err()
    );
}
