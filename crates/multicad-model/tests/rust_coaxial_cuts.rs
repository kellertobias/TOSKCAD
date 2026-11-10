use multicad_core::{model, project_file, references::TopologyKind};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn toolcase_magnet_stops_keep_exact_rims_and_round_trip_through_step() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let features: Vec<_> = document
        .entities()
        .filter_map(|e| {
            let boolean = model::read_boolean(&document, e.id).ok()?;
            document
                .get(boolean.target.producer)?
                .name
                .starts_with("Rear magnet stop ")
                .then_some((e.id, boolean))
        })
        .collect();
    assert_eq!(features.len(), 4);
    let roots: Vec<_> = features.iter().map(|(id, _)| *id).collect();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let evaluate = |backend| {
        compiler
            .compile_sources_with_backend(&document, 0, &roots, backend, None)
            .unwrap()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(evaluate);
    let temp = tempfile::tempdir().unwrap();
    let expected = std::f64::consts::PI * 3. * (9_f64.powi(2) - 5.1_f64.powi(2));
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    for (id, source) in features {
        let [solid, reference] = [&rust, &native].map(|m| m.solid(id).unwrap());
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), expected);
        near(reference.volume_mm3().unwrap(), expected);
        let [a, b] = [solid.bounds().unwrap(), reference.bounds().unwrap()];
        for (x, y) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            near(x, y);
        }
        let topology = solid.topology_snapshot(id, 0).unwrap();
        assert_eq!(solid.faces().unwrap().len(), 4);
        for (ordinal, producer) in [(2, source.target.producer), (3, source.tools[0].producer)] {
            let wall = topology
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == ordinal)
                .unwrap();
            assert!(wall.origins.iter().any(|origin| origin.source == producer));
        }
        let receipt = solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        assert!(!receipt.mesh.indices.is_empty());
        let path = temp.path().join(format!("{id}.step"));
        solid.write_step(&path).unwrap();
        let independent = Solid::read_step(&path, compiler.policy).unwrap();
        near(independent.volume_mm3().unwrap(), expected);
    }
}
