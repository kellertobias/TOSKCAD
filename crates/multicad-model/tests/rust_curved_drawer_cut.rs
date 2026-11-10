use multicad_core::{document::EntityId, project_file, references::AnalyticGeometry};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend, export::verify_rust_step_shape};

#[test]
fn saved_toolcase_drawer_cuts_front_pull_and_lead_ins_without_losing_fillet_geometry() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let feature = EntityId::parse("18714835-1359-4a41-b8c0-6deae91075ab").unwrap();
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
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>()
    };
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    assert!(actual.uses_rust_geometry());
    assert_eq!(actual.solid_count().unwrap(), 1);
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    near(area(actual), area(expected));
    let [a, b] = [actual, expected].map(|solid| solid.bounds().unwrap());
    for i in 0..3 {
        near(a.min[i], b.min[i]);
        near(a.max[i], b.max[i]);
    }
    let snapshot = actual.topology_snapshot(feature, 0).unwrap();
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| matches!(e.geometry, Some(AnalyticGeometry::EllipticArc { .. })))
    );
    assert!(
        !actual
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("drawer.step");
    actual.write_step(&path).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    for imported in [
        Solid::read_step(&path, compiler.policy).unwrap(),
        Solid::import_rust_step(&content, feature, compiler.policy).unwrap(),
    ] {
        assert_eq!(imported.solid_count().unwrap(), 1);
        near(
            imported.volume_mm3().unwrap(),
            expected.volume_mm3().unwrap(),
        );
        near(area(&imported), area(expected));
    }
    near(
        verify_rust_step_shape(
            &path,
            rust.solid(feature).unwrap(),
            &rust.snapshot().shapes[&feature],
            compiler.policy.linear_tolerance_mm,
        )
        .unwrap(),
        expected.volume_mm3().unwrap(),
    );
}
