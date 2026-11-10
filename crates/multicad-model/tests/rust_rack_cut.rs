use multicad_core::{document::EntityId, project_file, references::AnalyticGeometry};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend, export::verify_rust_step_shape};

#[test]
fn saved_toolcase_rack_retains_dovetail_faces_and_all_sixteen_exact_blind_bores() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let feature = EntityId::parse("c5978ce8-9734-43e1-a9ed-14d1fdbeced5").unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &[feature], backend, None)
            .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
    });
    let [actual, expected] = [&rust, &native].map(|model| model.solid(feature).unwrap());
    assert!(actual.uses_rust_geometry());
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    let topology = actual.topology_snapshot(feature, 0).unwrap();
    for radius in [2., 2.6] {
        assert_eq!(
            topology
                .elements
                .iter()
                .filter(|edge| matches!(
                    edge.geometry,
                    Some(AnalyticGeometry::CircularArc { radius_mm, .. }) if radius_mm == radius
                ))
                .count(),
            64
        );
    }
    assert!(
        !actual
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("toolcase-rack.step");
    actual.write_step(&path).unwrap();
    let [independent, reimported] = [
        Solid::read_step(&path, compiler.policy).unwrap(),
        Solid::import_rust_step(
            &std::fs::read_to_string(&path).unwrap(),
            feature,
            compiler.policy,
        )
        .unwrap(),
    ];
    let expected_bounds = expected.bounds().unwrap();
    for solid in [actual, &independent, &reimported] {
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
        near(area(solid), area(expected));
        let bounds = solid.bounds().unwrap();
        for i in 0..3 {
            near(bounds.min[i], expected_bounds.min[i]);
            near(bounds.max[i], expected_bounds.max[i]);
        }
    }
    let receipt = verify_rust_step_shape(
        &path,
        rust.solid(feature).unwrap(),
        &rust.snapshot().shapes[&feature],
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    near(receipt, actual.volume_mm3().unwrap());
}

#[test]
fn saved_rack_coupon_crops_one_blind_bore_into_an_exact_open_notch() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let feature = EntityId::parse("f7bf396b-be67-4d17-9bb3-f4e5e7d5ca29").unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &[feature], backend, None)
            .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
    });
    let [actual, expected] = [&rust, &native].map(|model| model.solid(feature).unwrap());
    assert!(actual.uses_rust_geometry());
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    let topology = actual.topology_snapshot(feature, 0).unwrap();
    assert!(topology.elements.iter().any(|edge| matches!(
        edge.geometry,
        Some(AnalyticGeometry::CircularArc { radius_mm: 2., .. })
    )));
    assert!(!topology.elements.iter().any(|edge| matches!(
        edge.geometry,
        Some(AnalyticGeometry::CircularArc { radius_mm: 2.6, .. })
    )));
    assert!(
        !actual
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("rack-coupon.step");
    actual.write_step(&path).unwrap();
    let [independent, reimported] = [
        Solid::read_step(&path, compiler.policy).unwrap(),
        Solid::import_rust_step(
            &std::fs::read_to_string(&path).unwrap(),
            feature,
            compiler.policy,
        )
        .unwrap(),
    ];
    let expected_bounds = expected.bounds().unwrap();
    near(expected_bounds.max[1], 30.);
    for solid in [actual, &independent, &reimported] {
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
        near(area(solid), area(expected));
        let bounds = solid.bounds().unwrap();
        for i in 0..3 {
            near(bounds.min[i], expected_bounds.min[i]);
            near(bounds.max[i], expected_bounds.max[i]);
        }
    }
    let receipt = verify_rust_step_shape(
        &path,
        rust.solid(feature).unwrap(),
        &rust.snapshot().shapes[&feature],
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    near(receipt, actual.volume_mm3().unwrap());
}
