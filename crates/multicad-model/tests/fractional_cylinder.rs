use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn fractional_cylinder_keeps_authored_dimensions_despite_derived_rational_volume_overflow() {
    let radius = 7.7625;
    let height = 3.37 * (25.4 / 12.);
    let parameters = PrimitiveParameters::Cylinder {
        radius: length(radius),
        height: length(height),
        frame: Default::default(),
    };
    let request = PrimitiveRequest::new("Fractional receiving stock", parameters.clone());
    let mut s = Session::new(Document::new("Fractional cylinder").unwrap()).unwrap();
    let candidate = model::prepare_primitive(&s, 0, Actor::local(Channel::Ui), &request).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let checked = compiler
        .compile_with_backend(
            candidate.prepared.document(),
            1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let solid = checked.solid(request.feature).unwrap();
    assert!(solid.uses_rust_geometry());
    let expected = std::f64::consts::PI * radius * radius * height;
    assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-10);
    assert!((solid.bounds().unwrap().max[2] - height).abs() < 1e-14);
    s.commit(candidate.prepared).unwrap();
    assert_eq!(
        model::read_primitive(s.document(), request.feature)
            .unwrap()
            .parameters,
        parameters
    );
    #[cfg(feature = "occt")]
    {
        let native = compiler
            .compile_with_backend(
                s.document(),
                s.revision(),
                GeometryBackend::OpenCascade,
                None,
            )
            .unwrap();
        assert!(
            (native.solid(request.feature).unwrap().volume_mm3().unwrap() - expected).abs() < 1e-8
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fractional.step");
    solid.write_step(&path).unwrap();
    let imported = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        multicad_core::document::EntityId::new(),
        Default::default(),
    )
    .unwrap();
    assert!((imported.volume_mm3().unwrap() - expected).abs() < 1e-7);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(
        model::read_primitive(&reopened.document, request.feature)
            .unwrap()
            .parameters,
        parameters
    );
    let rebuilt = compiler
        .compile_with_backend(&reopened.document, 0, GeometryBackend::Rust, None)
        .unwrap();
    assert!(
        (rebuilt
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - expected)
            .abs()
            < 1e-10
    );
}
