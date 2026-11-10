//! Real printed guide geometry, coupled fence movement, and a native 3MF
//! package containing the printed parts rather than purchased hardware.
#[path = "../examples/drill_guide_demo.rs"]
#[allow(dead_code)]
mod demo;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, EntityId, Mutation},
    exchange::*,
    project_file,
};
use multicad_model::{Compiler, export::Exporter};
fn shipped() -> Document {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(demo::SHIPPED_PATH);
    project_file::read(
        &mut std::fs::File::open(path).unwrap(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
fn named(d: &Document, name: &str) -> EntityId {
    d.entities()
        .find(|e| e.name == name && matches!(e.data, EntityData::Part { .. }))
        .unwrap()
        .id
}
#[test]
fn shipped_guide_moves_the_fence_and_writes_only_the_four_printed_parts() {
    let mut session = Session::new(shipped()).unwrap();
    let old = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let fence = named(session.document(), "Printed sliding fence");
    let old_bounds = old
        .solid(old.snapshot().bodies[&fence])
        .unwrap()
        .bounds()
        .unwrap();
    assert!((old_bounds.max[1] - 35.).abs() < 1e-6);
    let mut parameter = session
        .document()
        .entities()
        .find(|e| e.name == "edge_distance")
        .unwrap()
        .clone();
    if let EntityData::Parameter { expression, .. } = &mut parameter.data {
        *expression = "25".into();
    }
    let actor = Actor::local(Channel::CommandLine);
    let p = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: actor.clone(),
            label: "Change drilling distance".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    Compiler::default()
        .compile(p.document(), session.revision())
        .unwrap();
    session.commit(p).unwrap();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let bounds = model
        .solid(model.snapshot().bodies[&fence])
        .unwrap()
        .bounds()
        .unwrap();
    assert!((bounds.max[1] - 40.).abs() < 1e-6);
    for name in ["Printed thumb knob 1", "Printed thumb knob 2"] {
        let id = named(session.document(), name);
        let before = old
            .solid(old.snapshot().bodies[&id])
            .unwrap()
            .bounds()
            .unwrap();
        let after = model
            .solid(model.snapshot().bodies[&id])
            .unwrap()
            .bounds()
            .unwrap();
        assert!((after.min[1] - before.min[1] - 5.).abs() < 1e-6);
    }
    for size in [3, 5, 8] {
        let id = named(
            session.document(),
            &format!("Steel drill bushing {size} mm"),
        );
        let solid = model.solid(model.snapshot().bodies[&id]).unwrap();
        let radius = (f64::from(size) + 0.05) / 2.;
        let expected = std::f64::consts::PI * (36. - radius * radius) * 22.;
        assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-5);
    }
    let request = PrintRequest {
        source: PrintSource::Tag {
            tag: "printed".into(),
        },
        ..Default::default()
    };
    let exporter = Exporter::default();
    let preview = exporter
        .print_preview(
            session.document(),
            session.revision(),
            &request,
            &actor,
            None,
        )
        .unwrap();
    assert!(preview.can_write(), "{:?}", preview.blocked);
    assert_eq!(preview.objects.len(), 4);
    assert!(
        preview
            .objects
            .iter()
            .all(|o| o.name.starts_with("Printed "))
    );
    let dir = tempfile::tempdir().unwrap();
    let receipt = exporter
        .write_print(
            session.document(),
            session.revision(),
            &request,
            &preview.key,
            &dir.path().join("drill-guide.3mf"),
            &actor,
            None,
        )
        .unwrap();
    assert_eq!(receipt.files.len(), 1);
    assert_eq!(receipt.files[0].objects, 4);
    assert!(receipt.files[0].mesh_volume_mm3 > 0.);
}
#[test]
fn generator_and_shipped_guide_have_the_same_active_geometry() {
    fn geometry(d: &Document) -> Vec<(String, [f64; 3], [f64; 3], f64)> {
        let model = Compiler::default().compile(d, 0).unwrap();
        let mut out: Vec<_> = model
            .snapshot()
            .bodies
            .iter()
            .filter_map(|(part, feature)| {
                let entity = d.get(*part)?;
                if !matches!(entity.data, EntityData::Part { .. }) {
                    return None;
                }
                let solid = model.solid(*feature)?;
                let b = solid.bounds().unwrap();
                Some((
                    entity.name.clone(),
                    b.min,
                    b.max,
                    solid.volume_mm3().unwrap(),
                ))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1[0].total_cmp(&b.1[0])));
        out
    }
    let fresh = demo::build().unwrap();
    let original = shipped();
    assert_eq!(fresh.parts.len(), 13);
    assert_eq!(fresh.printed.len(), 4);
    assert_eq!(fresh.bushings.len(), 3);
    let a = geometry(fresh.session.document());
    let b = geometry(&original);
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(a.0, b.0);
        for (x, y) in
            a.1.iter()
                .chain(&a.2)
                .chain(std::iter::once(&a.3))
                .zip(b.1.iter().chain(&b.2).chain(std::iter::once(&b.3)))
        {
            assert!((x - y).abs() < 1e-5, "{}: {x} != {y}", a.0);
        }
    }
}
