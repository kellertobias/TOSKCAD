use multicad_core::{document::EntityData, model, project_file, references::TopologyKind};
use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn toolcase_circular_bore_chamfers_rebuild_with_named_faces_and_exact_step() {
    let mut bytes = include_bytes!("../../../examples/toolcase.tcd").as_slice();
    let document = project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let sources = document
        .entities()
        .filter_map(|entity| {
            let EntityData::Feature {
                definition,
                suppressed: false,
                ..
            } = &entity.data
            else {
                return None;
            };
            (definition.type_id == model::EDGE_TREATMENT_TYPE).then_some(entity.id)
        })
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 4);
    let compile = |backend| {
        Compiler::default()
            .compile_sources_with_backend(&document, 0, &sources, backend, None)
            .unwrap_or_else(|cause| panic!("{backend:?}: {cause}"))
    };
    let rust = compile(GeometryBackend::Rust);
    let native = compile(GeometryBackend::OpenCascade);
    let temporary = tempfile::tempdir().unwrap();
    let mut rims = 0;
    for (index, &source) in sources.iter().enumerate() {
        let actual = rust.solid(source).unwrap();
        let expected = native.solid(source).unwrap();
        assert!(actual.uses_rust_geometry());
        assert_eq!(actual.solid_count().unwrap(), 1);
        assert!((actual.volume_mm3().unwrap() - expected.volume_mm3().unwrap()).abs() < 1e-5);
        let (a, b) = (actual.bounds().unwrap(), expected.bounds().unwrap());
        for axis in 0..3 {
            assert!((a.min[axis] - b.min[axis]).abs() < 1e-6);
            assert!((a.max[axis] - b.max[axis]).abs() < 1e-6);
        }
        let (_, captures) = model::read_edge_treatment(&document, source).unwrap();
        let topology = actual.topology_snapshot(source, 0).unwrap();
        for capture in captures {
            let role = format!("edge/{}", capture.id);
            assert_eq!(
                topology
                    .elements
                    .iter()
                    .filter(|element| {
                        element.key.kind == TopologyKind::Face
                            && element
                                .origins
                                .iter()
                                .any(|origin| origin.source == source && origin.role == role)
                    })
                    .count(),
                1,
            );
            rims += 1;
        }
        let receipt = actual
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap();
        assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);
        let path = temporary.path().join(format!("chamfer-{index}.step"));
        actual.write_step(&path).unwrap();
        let policy = GeometryPolicy::default();
        for restored in [
            Solid::read_step(&path, policy).unwrap(),
            Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), source, policy)
                .unwrap(),
        ] {
            assert_eq!(restored.solid_count().unwrap(), 1);
            assert!((restored.volume_mm3().unwrap() - expected.volume_mm3().unwrap()).abs() < 1e-5);
        }
    }
    assert_eq!(rims, 20);
}
