use multicad_core::{
    document::EntityId,
    model::{self, MechanicalPartParameters, ThreadRepresentation},
    project_file,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{GeometryErrorKind, MeshOptions, PlaneFrame};
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn the_toolcase_cosmetic_screws_and_nuts_match_native_solids() {
    let mut bytes = include_bytes!("../../../examples/toolcase.tcd").as_slice();
    let opened = project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let document = opened.project.document;
    let mut roots = vec![];
    let mut counts = [0, 0];
    for entity in document.entities() {
        let Ok(source) = model::read_mechanical_part(&document, entity.id) else {
            continue;
        };
        let (thread, slot) = match source.parameters {
            MechanicalPartParameters::MachineScrew { thread, .. } => (thread, 0),
            MechanicalPartParameters::HexNut { thread, .. } => (thread, 1),
            _ => continue,
        };
        assert_eq!(
            thread.representation,
            ThreadRepresentation::CosmeticEnvelope
        );
        roots.push((entity.id, slot));
        counts[slot] += 1;
    }
    assert_eq!(counts, [43, 25]);
    let sources: Vec<EntityId> = roots.iter().map(|&(feature, _)| feature).collect();
    let compile = |backend| {
        Compiler::default()
            .compile_sources_with_backend(&document, 0, &sources, backend, None)
            .unwrap()
    };
    let rust = compile(GeometryBackend::Rust);
    let native = compile(GeometryBackend::OpenCascade);
    for (feature, slot) in roots {
        let actual = &rust.snapshot().shapes[&feature];
        let expected = &native.snapshot().shapes[&feature];
        let label = &document.get(feature).unwrap().name;
        assert_eq!(actual.solid_count, 1, "{label}");
        assert!(
            (actual.volume_mm3 - expected.volume_mm3).abs() < 1e-7,
            "{label}: {} vs {}",
            actual.volume_mm3,
            expected.volume_mm3
        );
        for axis in 0..3 {
            assert!(
                (actual.bounds.min[axis] - expected.bounds.min[axis]).abs() < 1e-6,
                "{label}"
            );
            assert!(
                (actual.bounds.max[axis] - expected.bounds.max[axis]).abs() < 1e-6,
                "{label}"
            );
        }
        assert!(!actual.mesh.indices.is_empty());
        let role = if slot == 0 {
            "screw.head/top.land"
        } else {
            "cap.start"
        };
        let top = actual
            .topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face
                    && element
                        .origins
                        .iter()
                        .any(|origin| origin.source == feature && origin.role == role)
            })
            .unwrap();
        let Some(AnalyticGeometry::Plane { frame }) = top.geometry else {
            panic!("Expected planar ring on {label}");
        };
        let solid = rust.solid(feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert!(!solid.supports_step_export());
        let error = solid
            .manufacturing_mesh(MeshOptions::default())
            .unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("modeled thread geometry"));
        let profile = solid
            .planar_face_contours(
                top.key.ordinal,
                PlaneFrame {
                    origin: frame.origin_mm,
                    u: frame.u,
                    v: frame.v,
                },
            )
            .unwrap();
        assert_eq!(profile.loops.len(), 2);
        assert!(profile.loops[0].outer);
        assert!(!profile.loops[1].outer);
    }
}
