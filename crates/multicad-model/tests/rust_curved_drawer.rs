use multicad_core::{document::EntityId, project_file, references::AnalyticGeometry};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn saved_toolcase_drawer_keeps_exact_floor_fillet_intersections() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let features = [
        EntityId::parse("13c5fd30-fc3f-4a30-ab7a-d0de04cbe358").unwrap(),
        EntityId::parse("ae86a32d-9a2c-458f-9b8c-1c824d96a3ef").unwrap(),
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
        assert_eq!(actual.solid_count().unwrap(), 1);
        near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
        near(area(actual), area(expected));
        let [a, b] = [actual, expected].map(|solid| solid.bounds().unwrap());
        for i in 0..3 {
            near(a.min[i], b.min[i]);
            near(a.max[i], b.max[i]);
        }
        let topology = actual.topology_snapshot(feature, 0).unwrap();
        assert!(
            topology.elements.iter().any(|element| matches!(
                element.geometry,
                Some(AnalyticGeometry::EllipticArc { .. })
            ))
        );
        assert!(actual.supports_step_export());
        let path = scratch.path().join(format!("drawer-{feature}.step"));
        actual.write_step(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("=ELLIPSE(")
        );
        let independent = Solid::read_step(&path, compiler.policy).unwrap();
        assert_eq!(independent.solid_count().unwrap(), 1);
        near(
            independent.volume_mm3().unwrap(),
            expected.volume_mm3().unwrap(),
        );
        near(area(&independent), area(expected));
        let exported = independent.bounds().unwrap();
        for i in 0..3 {
            near(exported.min[i], b.min[i]);
            near(exported.max[i], b.max[i]);
        }
    }
    for feature in features {
        let actual = rust.solid(feature).unwrap();
        assert!(
            !actual
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap()
                .mesh
                .indices
                .is_empty()
        );
    }
}
