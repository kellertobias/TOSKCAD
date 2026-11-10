use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityId,
    model::{self, BooleanEdit},
    project_file,
    references::TopologyKind,
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend, export::verify_rust_step_shape};

#[test]
fn saved_toolcase_drawer_reinforcements_join_when_all_operands_are_planar() {
    let opened = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let session = Session::new(opened.project.document).unwrap();
    let feature = EntityId::parse("13c5fd30-fc3f-4a30-ab7a-d0de04cbe358").unwrap();
    let definition = model::read_boolean(session.document(), feature).unwrap();
    let tools: Vec<_> = definition
        .tools
        .into_iter()
        .filter(|input| {
            session.document().get(input.producer).unwrap().name
                != "Internal floor fillet extrusion"
        })
        .collect();
    assert_eq!(tools.len(), 13);
    let prepared = model::prepare_boolean_edit(
        &session,
        0,
        Actor::local(Channel::Ui),
        feature,
        &BooleanEdit {
            operation: definition.operation,
            tools: tools.clone(),
            keep_tools: definition.keep_tools,
        },
    )
    .unwrap();
    let document = prepared.prepared.document();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(document, 1, &[feature], backend, None)
            .unwrap_or_else(|error| panic!("{backend:?}: {error}"))
    });
    let [actual, expected] = [&rust, &native].map(|m| m.solid(feature).unwrap());
    assert!(actual.uses_rust_geometry());
    assert_eq!(actual.solid_count().unwrap(), 1);
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum::<f64>()
    };
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    near(area(actual), area(expected));
    let [a, b] = [actual, expected].map(|solid| solid.bounds().unwrap());
    for i in 0..3 {
        near(a.min[i], b.min[i]);
        near(a.max[i], b.max[i]);
    }
    let topology = actual.topology_snapshot(feature, 1).unwrap();
    let rib = tools
        .iter()
        .find(|input| {
            document.get(input.producer).unwrap().name == "Small drawer sliding rib extrusion"
        })
        .unwrap();
    assert!(topology.elements.iter().any(|element| {
        element.key.kind == TopologyKind::Face
            && element
                .origins
                .iter()
                .any(|origin| origin.source == rib.producer)
    }));
    let mesh = actual
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    assert!(!mesh.mesh.indices.is_empty());

    let temporary = tempfile::tempdir().unwrap();
    let step = temporary.path().join("drawer-reinforcement.step");
    actual.write_step(&step).unwrap();
    let content = std::fs::read_to_string(&step).unwrap();
    for independent in [
        Solid::read_step(&step, compiler.policy).unwrap(),
        Solid::import_rust_step(&content, feature, compiler.policy).unwrap(),
    ] {
        assert_eq!(independent.solid_count().unwrap(), 1);
        near(
            independent.volume_mm3().unwrap(),
            expected.volume_mm3().unwrap(),
        );
        near(area(&independent), area(expected));
    }
    near(
        verify_rust_step_shape(
            &step,
            rust.solid(feature).unwrap(),
            &rust.snapshot().shapes[&feature],
            compiler.policy.linear_tolerance_mm,
        )
        .unwrap(),
        expected.volume_mm3().unwrap(),
    );
}
