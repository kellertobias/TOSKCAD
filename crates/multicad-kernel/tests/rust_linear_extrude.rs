use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, DrawingProjection, DrawingSolid, GeometryPolicy, MeshOptions,
    PlaneFrame, RigidTransform, Solid, project_drawing,
    rust_kernel::{display::DisplayMesh, manufacturing::certify},
};

fn profile(
    frame: PlaneFrame,
    outer: &[[f64; 2]],
    hole: &[[f64; 2]],
) -> (CurvedProfile, Vec<Origin>) {
    let wire = |points: &[[f64; 2]]| {
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
            .collect()
    };
    (
        CurvedProfile {
            frame,
            outer: wire(outer),
            holes: vec![wire(hole)],
        },
        (0..outer.len() + hole.len())
            .map(|i| Origin {
                source: EntityId::new(),
                role: format!("edge/{i}"),
            })
            .collect(),
    )
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 2e-6, "{a} != {b}");
}

fn compare(rust: &Solid, native: &Solid, feature: EntityId, volume: f64) {
    for body in [rust, native] {
        assert_eq!(body.solid_count().unwrap(), 1);
        near(body.volume_mm3().unwrap(), volume);
    }
    let rb = rust.bounds().unwrap();
    let nb = native.bounds().unwrap();
    for (a, b) in rb
        .min
        .into_iter()
        .chain(rb.max)
        .zip(nb.min.into_iter().chain(nb.max))
    {
        near(a, b);
    }
    let rf = rust.faces().unwrap();
    let nf = native.faces().unwrap();
    assert_eq!(rf.len(), nf.len());
    let rt = rust.topology_snapshot(feature, 1).unwrap();
    let nt = native.topology_snapshot(feature, 0).unwrap();
    for element in nt
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        let intent = GeometryIntent::capture(&nt, element.key).unwrap();
        let Resolution::Resolved { key, .. } = resolve(&intent, &rt, 1) else {
            panic!("Native source face did not resolve: {:?}", element.origins);
        };
        let a = rf.iter().find(|face| face.index == key.ordinal).unwrap();
        let b = nf
            .iter()
            .find(|face| face.index == element.key.ordinal)
            .unwrap();
        near(a.area_mm2, b.area_mm2);
        let an = a.plane.unwrap().1;
        let bn = b.plane.unwrap().1;
        near(an.into_iter().zip(bn).map(|(x, y)| x * y).sum(), 1.);
    }
    let receipt = rust
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let mesh = receipt.mesh;
    let proof = certify(&DisplayMesh {
        vertices: mesh.vertices,
        indices: mesh.indices,
        face_indices: mesh.face_indices,
    })
    .unwrap();
    near(proof.signed_volume_mm3, volume);
}

#[test]
fn negative_oblique_concave_sweep_keeps_cap_attachment_and_actual_hole() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame::default();
    let (profile, origins) = profile(
        frame,
        &[[0., 0.], [8., 0.], [8., 4.], [4., 4.], [4., 8.], [0., 8.]],
        &[[1., 1.], [1., 3.], [2., 3.], [2., 1.]],
    );
    let displacement = [3., -2., -5.];
    let rust =
        Solid::rust_extrude_linear_named(&profile, displacement, 0., feature, &origins, policy)
            .unwrap();
    let native = Solid::extrude_curved_profile_vector_named(
        &profile,
        displacement,
        feature,
        &origins,
        policy,
    )
    .unwrap();
    compare(&rust, &native, feature, 230.);
    let end = PlaneFrame {
        origin: displacement,
        ..frame
    };
    let contours = rust.planar_face_contours(1, end).unwrap();
    assert_eq!(contours.loops.len(), 2);
    assert_eq!(contours.edge_count(), 10);
    near(contours.area_mm2(), 46.);
    let topology = rust.topology_snapshot(feature, 2).unwrap();
    let cap = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == 1)
        .unwrap();
    assert_eq!(cap.frames[0].frame.origin_mm, displacement);
    assert_eq!(cap.frames[0].frame.v, frame.v);
    let view = project_drawing(
        &[DrawingSolid {
            solid: &rust,
            hatch: None,
        }],
        DrawingProjection {
            section_offset_mm: Some(-2.5),
            section_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(view.len(), 10);
    assert!(
        view.iter()
            .all(|e| e.role == VectorRole::Section && matches!(e.curve, VectorCurve::Line { .. }))
    );
}

#[test]
fn drafted_oblique_sweep_keeps_exact_miter_walls_and_step_geometry() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame {
        origin: [3., 7., -2.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let (profile, origins) = profile(
        frame,
        &[[0., 0.], [10., 0.], [10., 8.], [0., 8.]],
        &[[3., 2.], [3., 5.], [6., 5.], [6., 2.]],
    );
    let displacement = [6., 2., 1.];
    let taper = (0.5 / 41_f64.sqrt()).atan();
    let rust =
        Solid::rust_extrude_linear_named(&profile, displacement, taper, feature, &origins, policy)
            .unwrap();
    let native = Solid::extrude_curved_profile_vector_tapered_named(
        &profile,
        displacement,
        taper,
        feature,
        &origins,
        policy,
    )
    .unwrap();
    compare(&rust, &native, feature, 498.);
    let start = rust.planar_face_contours(0, frame).unwrap();
    let end = PlaneFrame {
        origin: [9., 9., -1.],
        ..frame
    };
    let top = rust.planar_face_contours(1, end).unwrap();
    near(start.area_mm2(), 71.);
    near(top.area_mm2(), 95.);
    assert_eq!(top.loops.len(), 2);
    let moved = rust
        .transformed(RigidTransform {
            translation_mm: [100., 20., -10.],
            ..Default::default()
        })
        .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("draft.step");
    moved.write_step(&path).unwrap();
    let read = Solid::read_step(&path, policy).unwrap();
    let parsed = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        policy,
    )
    .unwrap();
    for body in [&read, &parsed] {
        near(body.volume_mm3().unwrap(), 498.);
        assert_eq!(body.solid_count().unwrap(), 1);
        let actual = body.bounds().unwrap();
        let expected = moved.bounds().unwrap();
        for (a, b) in actual
            .min
            .into_iter()
            .chain(actual.max)
            .zip(expected.min.into_iter().chain(expected.max))
        {
            near(a, b);
        }
    }
    let native_path = scratch.path().join("native-draft.step");
    native.write_step(&native_path).unwrap();
    let imported = Solid::import_rust_step(
        &std::fs::read_to_string(native_path).unwrap(),
        EntityId::new(),
        policy,
    )
    .unwrap();
    near(imported.volume_mm3().unwrap(), 498.);
    assert_eq!(imported.solid_count().unwrap(), 1);
}

#[test]
fn negative_oblique_extrusion_to_a_sloped_plane_keeps_hole_and_face_frames() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let (profile, origins) = profile(
        PlaneFrame::default(),
        &[[0., 0.], [10., 0.], [10., 6.], [0., 6.]],
        &[[3., 2.], [3., 4.], [5., 4.], [5., 2.]],
    );
    let direction = [0.6, 0., -0.8];
    let l = 1_f64.hypot(0.2);
    let end = PlaneFrame {
        origin: [0., 0., -10.],
        u: [1. / l, 0., 0.2 / l],
        ..PlaneFrame::default()
    };
    let native = Solid::extrude_curved_profile_to_plane_named(
        &profile, direction, end, feature, &origins, policy,
    )
    .unwrap();
    let rust = Solid::rust_extrude_to_planes_named(
        &profile, direction, None, end, feature, &origins, policy,
    )
    .unwrap();
    // Integrate the affine travel over the actual remaining sketch area.
    let volume = 0.8 * (56. * 10. - 0.2 * (60. * 5. - 4. * 4.)) / 0.92;
    compare(&rust, &native, feature, volume);
    let rust_faces = rust.faces().unwrap();
    let native_faces = native.faces().unwrap();
    for face in rust_faces {
        assert!(native_faces.iter().any(|native| {
            native
                .center_mm
                .into_iter()
                .zip(face.center_mm)
                .all(|(a, b)| (a - b).abs() < 2e-6)
        }));
    }
    let topology = rust.topology_snapshot(feature, 0).unwrap();
    let cap = &topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == 1)
        .unwrap()
        .frames[0]
        .frame;
    assert_eq!(cap.origin_mm, end.origin);
    assert_eq!(cap.u, end.u);
    assert_eq!(cap.v, end.v);
    assert_eq!(rust.planar_face_contours(1, end).unwrap().loops.len(), 2);
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("sloped.step");
    rust.write_step(&path).unwrap();
    let native_path = scratch.path().join("native-sloped.step");
    native.write_step(&native_path).unwrap();
    for readback in [
        Solid::read_step(&path, policy).unwrap(),
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), feature, policy).unwrap(),
        Solid::import_rust_step(
            &std::fs::read_to_string(&native_path).unwrap(),
            feature,
            policy,
        )
        .unwrap(),
    ] {
        near(readback.volume_mm3().unwrap(), volume);
        assert_eq!(readback.faces().unwrap().len(), 10);
    }
}

#[test]
fn each_associative_end_must_clear_the_entire_working_sketch() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame::default();
    let (profile, origins) = profile(
        frame,
        &[[0., 0.], [10., 0.], [10., 6.], [0., 6.]],
        &[[3., 2.], [3., 4.], [5., 4.], [5., 2.]],
    );
    let end = PlaneFrame {
        origin: [0., 0., 10.],
        ..frame
    };
    let crossing = PlaneFrame {
        origin: [5., 0., 0.],
        u: [0.8, 0., 0.6],
        ..frame
    };
    for (start, end) in [(None, crossing), (Some(crossing), end), (Some(frame), end)] {
        let error = Solid::rust_extrude_to_planes_named(
            &profile,
            [0., 0., 1.],
            start,
            end,
            feature,
            &origins,
            policy,
        )
        .err()
        .expect("crossing or coincident end must fail");
        assert_eq!(error.kind, multicad_kernel::GeometryErrorKind::Input);
    }
}
