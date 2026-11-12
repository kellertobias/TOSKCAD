use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, chamfered_bores::Counterbore, polygon::PolygonPrism},
    *,
};
fn stock(h: i64, reverse: bool) -> Solid {
    let q = |n| Rational::new(n, 1).unwrap();
    let p = PolygonPrism::with_circular_holes(
        vec![
            vec![[0, 0], [20, 0], [20, 16], [0, 16]]
                .into_iter()
                .map(|p| p.map(q))
                .collect(),
        ],
        vec![Circle::new([q(10), q(8)], q(1)).unwrap()],
        q(h),
    )
    .unwrap();
    let f = EntityId::new();
    let origins: Vec<_> = (0..5)
        .map(|i| Origin {
            source: f,
            role: format!("edge.{i}"),
        })
        .collect();
    Solid::from_polygon_prism_sweep(
        p,
        PlaneFrame {
            origin: [40., -8., 12.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        reverse,
        f,
        &origins,
        &[false; 5],
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn seated(h: i64, r: f64, reverse: bool) -> Solid {
    let source = stock(h, reverse);
    let feature = EntityId::new();
    source
        .rust_counterbore_polygon(
            &[
                Counterbore {
                    hole: 0,
                    end: false,
                    radius_mm: r,
                    depth_mm: 1.,
                },
                Counterbore {
                    hole: 0,
                    end: true,
                    radius_mm: 2.,
                    depth_mm: 1.5,
                },
            ],
            &[
                Origin {
                    source: feature,
                    role: "entry".into(),
                },
                Origin {
                    source: feature,
                    role: "exit".into(),
                },
            ],
            GeometryPolicy::default(),
        )
        .unwrap()
}
fn encoded(s: &Solid) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seat.step");
    s.write_step(&path).unwrap();
    std::fs::read_to_string(path).unwrap()
}
#[test]
fn counterbore_shoulders_publish_real_planes_closed_mesh_and_exact_sketch_holes() {
    for reverse in [false, true] {
        let source = seated(6, 3., reverse);
        let feature = EntityId::new();
        let snap = source.topology_snapshot(feature, 1).unwrap();
        snap.validate().unwrap();
        let faces = source.faces().unwrap();
        assert_eq!(faces.len(), 11);
        for (index, area) in [
            (8, 8. * std::f64::consts::PI),
            (9, 3. * std::f64::consts::PI),
        ] {
            let face = &faces[index];
            assert!(face.plane.is_some());
            assert!((face.area_mm2 - area).abs() < 1e-10);
            let element = snap
                .elements
                .iter()
                .find(|e| {
                    e.key
                        == TopologyKey {
                            kind: TopologyKind::Face,
                            ordinal: index as u32,
                        }
                })
                .unwrap();
            let f = element.frames[0].frame;
            let contours = source
                .planar_face_contours(
                    index as u32,
                    PlaneFrame {
                        origin: f.origin_mm,
                        u: f.u,
                        v: f.v,
                    },
                )
                .unwrap();
            assert_eq!(contours.loops.len(), 2);
            contours.validate(1e-7).unwrap();
            assert!(
                (contours.loops.iter().map(|l| l.signed_area()).sum::<f64>() - area).abs() < 1e-8
            );
        }
        let options = MeshOptions {
            linear_deflection_mm: 0.001,
            angular_deflection_rad: 0.08,
            maximum_triangles: 100000,
        };
        let mesh = source.manufacturing_mesh(options).unwrap();
        rust_kernel::manufacturing::certify(&rust_kernel::display::DisplayMesh {
            vertices: mesh.vertices,
            indices: mesh.indices,
            face_indices: mesh.face_indices,
        })
        .unwrap();
        let read =
            Solid::import_rust_step(&encoded(&source), feature, GeometryPolicy::default()).unwrap();
        assert!((read.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs() < 1e-7);
        assert_eq!(read.faces().unwrap().len(), 11);
        assert!(
            read.faces()
                .unwrap()
                .iter()
                .filter(|f| f.plane.is_some())
                .count()
                == 8
        );
        let again =
            Solid::import_rust_step(&encoded(&read), feature, GeometryPolicy::default()).unwrap();
        assert!((again.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs() < 1e-7);
    }
}
#[test]
fn opaque_counterbore_references_bind_only_to_unchanged_finite_faces() {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let input = encoded(&seated(6, 3., false));
    let source = Solid::import_rust_step(&input, owner, policy).unwrap();
    let snap = source.topology_snapshot(owner, 1).unwrap();
    let same = Solid::import_rust_step(&input, owner, policy)
        .unwrap()
        .topology_snapshot(owner, 2)
        .unwrap();
    let taller = Solid::import_rust_step(&encoded(&seated(7, 3., false)), owner, policy)
        .unwrap()
        .topology_snapshot(owner, 2)
        .unwrap();
    let wider = Solid::import_rust_step(&encoded(&seated(6, 3.5, false)), owner, policy)
        .unwrap()
        .topology_snapshot(owner, 2)
        .unwrap();
    let authored = seated(6, 3., false).faces().unwrap();
    let fixed_centers = [
        authored[0].center_mm,
        authored[7].center_mm,
        authored[8].center_mm,
    ];
    let descriptors = source.faces().unwrap();
    let moved = source
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [100., -20., 5.],
        })
        .unwrap()
        .topology_snapshot(owner, 2)
        .unwrap();
    for face in snap
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        let intent = GeometryIntent::capture(&snap, face.key).unwrap();
        assert!(matches!(
            resolve(&intent, &same, 2),
            Resolution::Resolved { .. }
        ));
        assert!(matches!(
            resolve(&intent, &moved, 2),
            Resolution::Resolved { .. }
        ));
        let center = descriptors[face.key.ordinal as usize].center_mm;
        let fixed = fixed_centers
            .iter()
            .any(|p| p.iter().zip(center).all(|(a, b)| (a - b).abs() < 1e-7));
        assert_eq!(
            matches!(resolve(&intent, &taller, 2), Resolution::Resolved { .. }),
            fixed,
            "height face {}",
            face.key.ordinal
        );
        assert_eq!(
            matches!(resolve(&intent, &wider, 2), Resolution::Resolved { .. }),
            !fixed,
            "radius face {}",
            face.key.ordinal
        );
    }
    // Locate by physical descriptor rather than trusting imported STEP order.
    let entry_center = seated(6, 3., false).faces().unwrap()[7].center_mm;
    let outer = source
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane.is_none()
                && (f.area_mm2 - std::f64::consts::TAU * 3.).abs() < 1e-8
                && f.center_mm
                    .iter()
                    .zip(entry_center)
                    .all(|(a, b)| (a - b).abs() < 1e-7)
        })
        .unwrap();
    let intent = GeometryIntent::capture(
        &snap,
        TopologyKey {
            kind: TopologyKind::Face,
            ordinal: outer.index,
        },
    )
    .unwrap();
    assert!(!matches!(
        resolve(&intent, &wider, 2),
        Resolution::Resolved { .. }
    ));
}

#[test]
fn subsequent_unqualified_chamfer_keeps_all_seats_and_the_source_unchanged() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let source = stock(6, false)
        .rust_counterbore_polygon(
            &[Counterbore {
                hole: 0,
                end: false,
                radius_mm: 3.,
                depth_mm: 1.,
            }],
            &[Origin {
                source: feature,
                role: "seat".into(),
            }],
            policy,
        )
        .unwrap();
    let before = encoded(&source);
    let volume = source.volume_mm3().unwrap();
    let result = source.treat_edges_named(
        &[TreatmentEdge {
            key: TopologyKey {
                kind: TopologyKind::Edge,
                ordinal: 9,
            },
            origin: Origin {
                source: feature,
                role: "other-mouth".into(),
            },
        }],
        EdgeTreatment::Chamfer { distance_mm: 0.2 },
        feature,
        policy,
    );
    assert_eq!(result.err().unwrap().kind, GeometryErrorKind::Unsupported);
    assert_eq!(source.volume_mm3().unwrap(), volume);
    assert_eq!(encoded(&source), before);
    let read = Solid::import_rust_step(&before, feature, policy).unwrap();
    assert!((read.volume_mm3().unwrap() - volume).abs() < 1e-7);
}
