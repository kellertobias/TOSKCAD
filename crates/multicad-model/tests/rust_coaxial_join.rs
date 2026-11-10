use multicad_core::project_file;
use multicad_kernel::MeshOptions;
use multicad_model::{Compiler, GeometryBackend};
use std::f64::consts::PI;

#[test]
fn saved_front_panel_button_union_matches_the_native_solid() {
    let mut bytes = include_bytes!("../../../examples/front-panel.tcd").as_slice();
    let opened = project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let document = opened.project.document;
    let feature = document
        .entities()
        .find(|entity| entity.name == "Button union")
        .unwrap()
        .id;
    let compile = |backend| {
        Compiler::default()
            .compile_sources_with_backend(&document, 0, &[feature], backend, None)
            .unwrap()
    };
    let rust = compile(GeometryBackend::Rust);
    let native = compile(GeometryBackend::OpenCascade);
    let actual = rust.solid(feature).unwrap();
    let expected = native.solid(feature).unwrap();
    assert_eq!(actual.solid_count().unwrap(), 1);
    let volume = actual.volume_mm3().unwrap();
    assert!((volume - 1055. * PI).abs() < 1e-8);
    assert!((volume - expected.volume_mm3().unwrap()).abs() < 1e-7);
    let actual_bounds = actual.bounds().unwrap();
    let expected_bounds = expected.bounds().unwrap();
    for axis in 0..3 {
        assert!((actual_bounds.min[axis] - expected_bounds.min[axis]).abs() < 1e-6);
        assert!((actual_bounds.max[axis] - expected_bounds.max[axis]).abs() < 1e-6);
    }
    assert_eq!(actual.faces().unwrap().len(), 9);
    let mesh = actual
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert!(!mesh.mesh.indices.is_empty());
    assert!(!actual.supports_step_export());
}
