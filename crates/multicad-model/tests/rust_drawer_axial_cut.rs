use multicad_core::{document::EntityId, project_file, references::AnalyticGeometry};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend, export::verify_rust_step_shape};

#[test]
fn saved_toolcase_cuts_bores_and_captive_nut_recesses_in_both_drawers() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let features = [
        EntityId::parse("d12c787f-4e61-4797-854e-64278f2b81f9").unwrap(),
        EntityId::parse("ad943c73-02df-4940-9a62-a1048a7e9c21").unwrap(),
    ];
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &features, backend, None)
            .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
    });
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    let scratch = tempfile::tempdir().unwrap();
    for feature in features {
        let [actual, expected] = [&rust, &native].map(|model| model.solid(feature).unwrap());
        assert!(actual.uses_rust_geometry());
        let topology = actual.topology_snapshot(feature, 0).unwrap();
        let pocket_rims = topology
            .elements
            .iter()
            .filter(|edge| {
                matches!(edge.geometry, Some(AnalyticGeometry::CircularArc { radius_mm, .. })
                if radius_mm == 2.25 || radius_mm == 5.1)
            })
            .count();
        assert_eq!(pocket_rims, 48);
        assert!(
            !actual
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap()
                .mesh
                .indices
                .is_empty()
        );
        let path = scratch.path().join(format!("pockets-{feature}.step"));
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
}
