use multicad_core::{
    document::EntityId,
    references::{
        AnalyticGeometry, Attachment, GeometryIntent, Resolution, TopologyKey, TopologyKind,
        resolve,
    },
};
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame, RigidTransform,
    Solid,
    rust_kernel::{
        Circle,
        cylinder::AxialCylinder,
        headed_screw::{ScrewHead, ScrewSpec},
        parse_length_mm,
    },
};

fn metric(left_hand: bool, starts: u32) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: 49.8,
        depth_mm: 1.25 * 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts,
        left_hand,
    }
}

#[test]
fn facade_keeps_real_m8_material_and_analytic_topology_after_rigid_placement() {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let spec = metric(false, 1);
    let source =
        Solid::rust_helical_thread(spec, None, owner, PlaneFrame::default(), policy).unwrap();
    let p = [3.8, 0., 2.5];
    let q = [3.8, 0., 2.5 + spec.pitch_mm * 0.5];
    assert!(source.rust_thread_contains(p).unwrap());
    assert!(!source.rust_thread_contains(q).unwrap());
    assert!(!source.rust_thread_contains([0., 0., 50.]).unwrap());

    let d = 0.5_f64.sqrt();
    let transform = RigidTransform {
        rotation: [[d, 0., d], [0., 1., 0.], [-d, 0., d]],
        translation_mm: [17., -3., 21.],
    };
    let placed = source.transformed(transform).unwrap();
    let moved = |point: [f64; 3]| {
        std::array::from_fn(|i| {
            point
                .into_iter()
                .zip(transform.rotation[i])
                .map(|(a, b)| a * b)
                .sum::<f64>()
                + transform.translation_mm[i]
        })
    };
    assert!(placed.rust_thread_contains(moved(p)).unwrap());
    assert!(!placed.rust_thread_contains(moved(q)).unwrap());
    assert_eq!(source.volume_mm3().unwrap(), placed.volume_mm3().unwrap());
    let topology = placed.topology_snapshot(owner, 3).unwrap();
    let cap = topology
        .elements
        .iter()
        .find(|face| face.origins.iter().any(|o| o.role == "cap.start"))
        .unwrap();
    assert!(
        matches!(cap.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.origin_mm == moved([0.; 3]))
    );
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .count(),
        6
    );
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
            .count(),
        4
    );
    let display = placed.topology_with_display(owner, 3, Some(0.05)).unwrap();
    assert_eq!(display.topology, topology);
    assert_eq!(display.edges.len(), 12);
    assert_eq!(display.edges, placed.display_edges(0.05).unwrap());
    assert_eq!(display.vertices.len(), 8);
    assert_eq!(display.vertices, placed.display_vertices().unwrap());
    assert_eq!(placed.solid_count().unwrap(), 1);
    assert!(!placed.supports_step_export());
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("thread.step");
    assert_eq!(
        placed.write_step(&path).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    assert!(!path.exists());
}

#[test]
fn manufacturing_mesh_keeps_all_real_face_ordinals_and_closed_indexed_shells() {
    let spec = metric(true, 2);
    let source = Solid::rust_helical_thread(
        spec,
        None,
        EntityId::new(),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let copy = source
        .transformed(RigidTransform {
            translation_mm: [20., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let compound = Solid::compound_named(
        &[&source, &copy],
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let options = MeshOptions {
        linear_deflection_mm: 0.05,
        angular_deflection_rad: 0.3,
        maximum_triangles: 500_000,
    };
    let receipt = compound.manufacturing_mesh_with_receipt(options).unwrap();
    assert!(receipt.mesh.indices.len() > 12_000);
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let faces: std::collections::BTreeSet<_> = receipt.mesh.face_indices.iter().copied().collect();
    assert_eq!(faces, (0..20).collect());
    assert_eq!(compound.solid_count().unwrap(), 2);
}

fn cylindrical_stock(feature: EntityId, frame: PlaneFrame) -> Solid {
    let length = |text| parse_length_mm(text).unwrap();
    let zero = length("0 mm");
    let stock = AxialCylinder::new(
        feature.uuid_bits(),
        Circle::new([zero; 2], length("6 mm")).unwrap(),
        zero,
        length("10 mm"),
    )
    .unwrap();
    Solid::from_axial_cylinder_sweep(
        stock,
        frame,
        [1., 2.],
        false,
        multicad_core::references::Origin {
            source: EntityId::new(),
            role: "authored.circle".into(),
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}

#[test]
fn reverse_through_thread_keeps_stock_caps_wall_and_rims_and_removes_real_material() {
    let policy = GeometryPolicy::default();
    let stock_id = EntityId::new();
    let owner = EntityId::new();
    let stock_frame = PlaneFrame {
        origin: [21., 30., 50.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let stock = cylindrical_stock(stock_id, stock_frame);
    let frame = PlaneFrame {
        origin: [31., 30., 50.],
        u: [0., 0., 1.],
        v: [0., 1., 0.],
    };
    let mut spec = metric(false, 1);
    spec.length_mm = 10.;
    let tool = Solid::rust_helical_thread(spec, None, owner, frame, policy).unwrap();
    let cut = stock
        .rust_cut_internal_thread(spec, 1., owner, frame, policy)
        .unwrap();
    assert_eq!(cut.bounds().unwrap(), stock.bounds().unwrap());
    let removed = stock.volume_mm3().unwrap() - cut.volume_mm3().unwrap();
    assert!((removed - tool.volume_mm3().unwrap()).abs() < 1e-9);
    assert!(!cut.rust_thread_contains([28.5, 30., 53.8]).unwrap());
    assert!(cut.rust_thread_contains([27.875, 30., 53.8]).unwrap());
    assert!(cut.rust_thread_contains([31., 30., 55.]).unwrap());
    assert!(!cut.rust_thread_contains([26., 30., 50.]).unwrap());
    assert!(!cut.rust_thread_contains([26., 30., 57.]).unwrap());
    assert_eq!(cut.solid_count().unwrap(), 1);
    let before = stock.topology_snapshot(stock_id, 0).unwrap();
    let after = cut.topology_snapshot(owner, 1).unwrap();
    for element in &before.elements {
        let mut intent = GeometryIntent::capture(&before, element.key).unwrap();
        if let Some(anchor) = element.frames.first() {
            intent.attachment = Some(Attachment {
                anchor: anchor.origin.clone(),
                offset_mm: [0.; 3],
                angle_rad: 0.,
            });
        }
        let Resolution::Resolved {
            key,
            frame,
            geometry,
        } = resolve(&intent, &after, 1)
        else {
            panic!("Original receiving topology was lost for {:?}", element.key);
        };
        let expected_ordinal = match (element.key.kind, element.key.ordinal) {
            (TopologyKind::Face, 0 | 1) => 1 - element.key.ordinal,
            (TopologyKind::Face, 2) => 6,
            (TopologyKind::Edge, ordinal) => 12 + ordinal,
            _ => unreachable!(),
        };
        assert_eq!(
            key,
            TopologyKey {
                ordinal: expected_ordinal,
                ..element.key
            }
        );
        assert_eq!(frame, element.frames.first().map(|anchor| anchor.frame));
        if let Some(AnalyticGeometry::CircularArc { .. }) = element.geometry {
            assert_eq!(geometry, element.geometry);
        }
    }
    assert_eq!(cut.display_edges(0.05).unwrap().len(), 14);
    let receipt = cut
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let faces: std::collections::BTreeSet<_> = receipt.mesh.face_indices.into_iter().collect();
    assert_eq!(faces, (0..7).collect());
}

#[test]
fn unsupported_stock_placement_and_insufficient_wall_fail_without_changing_stock() {
    let policy = GeometryPolicy::default();
    let stock_id = EntityId::new();
    let stock = cylindrical_stock(stock_id, PlaneFrame::default());
    let owner = EntityId::new();
    let mut spec = metric(true, 2);
    spec.length_mm = 10.;
    let displaced = PlaneFrame {
        origin: [0.1, 0., 0.],
        ..PlaneFrame::default()
    };
    for (spec, frame) in [
        (spec, displaced),
        (
            HelicalThread {
                length_mm: 8.,
                ..spec
            },
            PlaneFrame::default(),
        ),
        (
            spec,
            PlaneFrame {
                v: [0., 0., 1.],
                ..PlaneFrame::default()
            },
        ),
    ] {
        let Err(cause) = stock.rust_cut_internal_thread(spec, 1., owner, frame, policy) else {
            panic!("Incompatible receiving cylinder was accepted");
        };
        assert_eq!(cause.kind, GeometryErrorKind::Unsupported);
    }
    for (minimum_wall, feature) in [(2.1, owner), (1., stock_id)] {
        let Err(cause) = stock.rust_cut_internal_thread(
            spec,
            minimum_wall,
            feature,
            PlaneFrame::default(),
            policy,
        ) else {
            panic!("Invalid through-thread was accepted");
        };
        assert_eq!(cause.kind, GeometryErrorKind::Input);
    }
    assert_eq!(stock.faces().unwrap().len(), 3);
    assert!((stock.volume_mm3().unwrap() - 360. * std::f64::consts::PI).abs() < 1e-9);
}

#[test]
fn rotated_socket_screw_has_one_shell_and_analytic_socket_faces() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let mut thread = metric(false, 1);
    thread.length_mm = 12.;
    let underhead = PlaneFrame {
        origin: [-8., 3., 4.],
        u: [0., 0., 1.],
        v: [0., -1., 0.],
    };
    let spec = ScrewSpec {
        head: ScrewHead::SocketCap {
            radius_mm: 6.5,
            height_mm: 8.,
            socket_across_flats_mm: 6.,
            socket_depth_mm: 4.,
        },
        shank_length_mm: 5.,
        shank_radius_mm: 4.,
    };
    let screw = Solid::rust_machine_screw(thread, None, spec, owner, underhead, policy).unwrap();
    let shaft = Solid::rust_helical_thread(thread, None, owner, underhead, policy).unwrap();
    let head = std::f64::consts::PI * (6.5_f64.powi(2) * 8. + 4_f64.powi(2) * 5.);
    let socket = 3_f64.sqrt() * 6_f64.powi(2) * 4. / 2.;
    let expected = shaft.volume_mm3().unwrap() + head - socket;
    assert!((screw.volume_mm3().unwrap() - expected).abs() < 1e-9);
    let bounds = screw.bounds().unwrap();
    assert_eq!(bounds.min, [-16., -3.5, -2.5]);
    assert!((bounds.max[0] - 9.).abs() < 1e-10);
    assert_eq!(bounds.max[1..], [9.5, 10.5]);
    assert!(!screw.rust_thread_contains([-14., 3., 4.]).unwrap());
    assert!(screw.rust_thread_contains([-14., 3., 9.]).unwrap());
    assert!(screw.rust_thread_contains([-11., 3., 4.]).unwrap());
    assert!(screw.rust_thread_contains([-6., 3., 4.]).unwrap());
    let topology = screw.topology_snapshot(owner, 4).unwrap();
    let named = |role| {
        topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face
                    && element.origins.iter().any(|o| o.role == role)
            })
            .unwrap()
    };
    let floor = named("screw.socket/end");
    let Some(AnalyticGeometry::Plane { frame }) = floor.geometry else {
        panic!("Socket floor has no analytic plane");
    };
    assert_eq!(frame.origin_mm, [-12., 3., 4.]);
    assert_eq!(frame.normal(), [-1., 0., 0.]);
    assert_eq!(floor.boundary.len(), 6);
    let shoulder = named("screw.thread/start");
    assert_eq!(shoulder.boundary.len(), 6);
    assert!(
        shoulder
            .boundary
            .iter()
            .all(|origin| !origin.role.ends_with("/crest"))
    );
    assert_eq!(screw.solid_count().unwrap(), 1);
    assert!(!screw.supports_step_export());
    let receipt = screw
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let mesh_faces: std::collections::BTreeSet<_> = receipt.mesh.face_indices.into_iter().collect();
    assert_eq!(mesh_faces, (0..17).collect());
    assert!(screw.display_edges(0.05).unwrap().len() >= 35);
}

#[test]
fn fully_threaded_hex_screw_preserves_head_seat_and_six_planar_wrench_faces() {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let mut thread = metric(true, 1);
    thread.length_mm = 9.8;
    let spec = ScrewSpec {
        head: ScrewHead::ExternalHex {
            across_flats_mm: 12.,
            height_mm: 5.,
        },
        shank_length_mm: 0.,
        shank_radius_mm: 4.,
    };
    let make = |thread| {
        Solid::rust_machine_screw(thread, None, spec, owner, PlaneFrame::default(), policy).unwrap()
    };
    let screw = make(thread);
    let before = screw.topology_snapshot(owner, 1).unwrap();
    let seat = before
        .elements
        .iter()
        .find(|element| {
            element
                .origins
                .iter()
                .any(|origin| origin.role == "screw.head/end")
        })
        .unwrap();
    assert_eq!(seat.key.ordinal, 0);
    let intent = GeometryIntent::capture(&before, seat.key).unwrap();
    let changed = make(HelicalThread {
        left_hand: false,
        ..thread
    });
    let after = changed.topology_snapshot(owner, 2).unwrap();
    assert!(
        matches!(resolve(&intent, &after, 2), Resolution::Resolved { key, .. } if key == seat.key)
    );
    assert_eq!(
        screw
            .faces()
            .unwrap()
            .iter()
            .filter(|face| face.plane.is_some())
            .count(),
        9
    );
    let bounds = screw.bounds().unwrap();
    assert!((bounds.max[0] - 6.).abs() < 1e-10);
    assert!((bounds.max[1] - 12. / 3_f64.sqrt()).abs() < 1e-10);
    assert!(!screw.rust_thread_contains([6.2, 0., -2.]).unwrap());
    assert!(screw.rust_thread_contains([0., 6.2, -2.]).unwrap());
    let receipt = screw
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
}

#[test]
fn rotated_hex_nut_keeps_the_exact_flats_and_authored_end_attachment() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let mut thread = metric(false, 1);
    thread.length_mm = 6.5;
    let frame = PlaneFrame {
        origin: [10., 20., 30.],
        u: [0., 0., 1.],
        v: [1., 0., 0.],
    };
    let nut = Solid::rust_hex_nut(thread, 13., 1., owner, frame, policy).unwrap();
    let tool = Solid::rust_helical_thread(thread, None, owner, frame, policy).unwrap();
    let expected = 3_f64.sqrt() * 13_f64.powi(2) * 6.5 / 2. - tool.volume_mm3().unwrap();
    assert!((nut.volume_mm3().unwrap() - expected).abs() < 1e-9);
    let bounds = nut.bounds().unwrap();
    let corner = 13. / 3_f64.sqrt();
    assert_eq!(bounds.min, [10. - corner, 20., 23.5]);
    assert_eq!(bounds.max, [10. + corner, 26.5, 36.5]);
    assert!(!nut.rust_thread_contains([10., 22.5, 33.8]).unwrap());
    assert!(nut.rust_thread_contains([10., 23.125, 33.8]).unwrap());
    assert!(!nut.rust_thread_contains([10., 23., 36.51]).unwrap());
    assert!(nut.rust_thread_contains([17., 23., 30.]).unwrap());
    assert!(!nut.rust_thread_contains([10., 23., 30.]).unwrap());
    assert_eq!(nut.solid_count().unwrap(), 1);
    let faces = nut.faces().unwrap();
    assert_eq!(faces.len(), 12);
    assert_eq!(faces.iter().filter(|face| face.plane.is_some()).count(), 8);
    let snapshot = nut.topology_snapshot(owner, 1).unwrap();
    let cap = snapshot
        .elements
        .iter()
        .find(|element| {
            element.key
                == TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: 0,
                }
        })
        .unwrap();
    assert_eq!(cap.boundary.len(), 10);
    assert_eq!(cap.frames[0].frame.normal(), frame.normal());
    assert!(matches!(
        cap.geometry,
        Some(AnalyticGeometry::Plane { frame }) if frame.normal() == [0., -1., 0.]
    ));
    let side = snapshot
        .elements
        .iter()
        .find(|element| {
            element.key
                == TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: 6,
                }
        })
        .unwrap();
    assert_eq!(side.boundary.len(), 4);
    assert!(
        side.origins
            .iter()
            .any(|origin| origin.role == "hardware.outer/0")
    );
    assert_eq!(nut.display_edges(0.05).unwrap().len(), 30);
    assert_eq!(nut.display_vertices().unwrap().len(), 20);
    let receipt = nut
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let labels: std::collections::BTreeSet<_> = receipt.mesh.face_indices.into_iter().collect();
    assert_eq!(labels, (0..12).collect());
    let Err(error) = Solid::rust_hex_nut(thread, 9., 1., owner, frame, policy) else {
        panic!("Nut accepted stock below the minimum wall");
    };
    assert_eq!(error.kind, GeometryErrorKind::Input);
}
