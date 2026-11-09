//! Saved print layout identity and transaction tests; native packing validates
//! physical extents and exports the exact placement transforms.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    exchange::*,
    project_file::{self, ProjectFile},
};
use std::io::Cursor;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn part(name: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    )
}

fn instance(name: &str, component: EntityId) -> Entity {
    Entity::new(
        name,
        EntityData::Instance {
            component,
            placement: Default::default(),
            placement_source: None,
        },
    )
}

fn placement(source: PartRef, position_mm: [f64; 2]) -> PrintPlatePlacement {
    PrintPlatePlacement {
        source,
        position_mm,
        rotation_deg: [90.; 3],
    }
}

#[test]
fn saved_plate_keeps_its_exact_sources_and_arrangement_across_revision_reopen_and_undo() {
    let left = part("Handle");
    let right = part("Panel");
    let a = instance("First copy", left.id);
    let b = instance("Second copy", left.id);
    let assembly = Entity::new(
        "Fixture",
        EntityData::Assembly {
            instances: vec![a.id, b.id],
        },
    );
    let doc = Document::new("Print arrangement")
        .unwrap()
        .edited(&[
            Mutation::Insert(left.clone()),
            Mutation::Insert(right.clone()),
            Mutation::Insert(a.clone()),
            Mutation::Insert(b.clone()),
            Mutation::Insert(assembly.clone()),
        ])
        .unwrap();
    let mut s = Session::new(doc.clone()).unwrap();
    let selected = resolve_occurrences(&doc, assembly.id)
        .unwrap()
        .into_iter()
        .find(|part| part.reference.occurrence_path == [b.id])
        .unwrap()
        .reference;
    let world = resolve_occurrences(&doc, right.id)
        .unwrap()
        .remove(0)
        .reference;
    let plate = PrintPlateDefinition {
        bed_mm: [245., 195.],
        placements: vec![placement(selected, [25., 8.]), placement(world, [-2., 42.])],
        stl_per_part: true,
        ..Default::default()
    };
    let id = EntityId::new();
    let prepared = prepare_print_plate(&s, 0, actor(), id, "Prototype plate", &plate).unwrap();
    assert_eq!(s.document(), &doc);
    s.commit(prepared).unwrap();
    assert_eq!(print_plates(s.document()).collect::<Vec<_>>(), [id]);
    assert_eq!(sets(s.document()).count(), 0);
    assert_eq!(read_print_plate(s.document(), id).unwrap(), plate);
    let stored = s.document().get(id).unwrap();
    let refs = stored.references();
    for referenced in [left.id, right.id, b.id, assembly.id] {
        assert!(refs.iter().any(|link| link.target == referenced));
    }
    assert!(!refs.iter().any(|link| link.target == a.id));

    let selected = resolve_print_plate(s.document(), s.revision(), &plate).unwrap();
    assert_eq!(selected.parts.len(), 2);
    assert_eq!(selected.parts[0].occurrence_path, [b.id]);
    assert_eq!(selected.parts[1].part, right.id);
    assert!(
        selected
            .parts
            .iter()
            .all(|part| part.role == PrintRole::Finished)
    );
    assert!(selected.skipped.is_empty());
    let summary = plate.summary_request(s.document());
    let PrintSource::Parts { parts } = summary.source else {
        panic!("expected explicit root sources");
    };
    assert_eq!(parts.len(), 2);
    assert!(parts.contains(&assembly.id) && parts.contains(&right.id));
    assert_eq!(summary.aids, AidPolicy::ExcludeRemovable);
    assert!(summary.stl_per_part);

    let mut renamed = left;
    renamed.name = "Revised handle".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Rename a source Part".into(),
        edits: vec![Mutation::Replace(renamed)],
    })
    .unwrap();
    let refreshed = resolve_print_plate(s.document(), s.revision(), &plate).unwrap();
    assert_ne!(refreshed.stamp, selected.stamp);
    assert_eq!(refreshed.parts[0].name, "Revised handle (Second copy)");
    assert_eq!(read_print_plate(s.document(), id).unwrap(), plate);

    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(PRINT_PLATE_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        read_print_plate(&opened.project.document, id).unwrap(),
        plate
    );

    s.undo(s.revision(), actor()).unwrap();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &doc);
}

#[test]
fn plate_rejects_invalid_or_stale_placements_and_rotation_order_is_explicit() {
    let part = part("Stand");
    let doc = Document::new("Plate checks")
        .unwrap()
        .edited(&[Mutation::Insert(part.clone())])
        .unwrap();
    let source = resolve_occurrences(&doc, part.id)
        .unwrap()
        .remove(0)
        .reference;
    let item = placement(source, [0., 0.]);
    let rotation = item.rotation();
    let point = [2., 3., 5.];
    let rotated = rotation.map(|row| row.into_iter().zip(point).map(|(a, b)| a * b).sum::<f64>());
    for (actual, expected) in rotated.into_iter().zip([5., 3., -2.]) {
        assert!((actual - expected).abs() < 1e-12);
    }

    let mut invalid = PrintPlateDefinition {
        placements: vec![item.clone(), item.clone()],
        ..Default::default()
    };
    assert!(invalid.validate().unwrap_err().message.contains("once"));
    invalid.placements.pop();
    invalid.placements[0].position_mm[1] = f64::INFINITY;
    assert!(invalid.validate().is_err());
    invalid.placements[0] = item;
    invalid.bed_mm[0] = 0.;
    assert!(invalid.validate().is_err());
    invalid.bed_mm = [100., 100.];
    invalid.placements[0].source.occurrence_path = vec![EntityId::new()];
    let error = resolve_print_plate(&doc, 0, &invalid).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Missing);
    assert!(error.message.contains("Placement 1"));
}
