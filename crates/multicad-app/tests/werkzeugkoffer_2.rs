//! Checks that the shipped case and its native generator still agree.
#[path = "../examples/werkzeugkoffer_2.rs"]
#[allow(dead_code)]
mod demo;

use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{EntityData, EntityId, Mutation},
    drawings,
    exchange::read_print_plate,
    model::{prepare_apply_pose, read_saved_motion_study},
    project_file,
};
use multicad_model::{Compiler, DrawingProviders, EvaluationCache, Model};
use std::path::Path;

fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}

fn named(session: &Session, name: &str) -> EntityId {
    session
        .document()
        .entities()
        .find(|entity| entity.name == name)
        .unwrap_or_else(|| panic!("Missing {name}"))
        .id
}

fn named_geometry(session: &Session, model: &Model) -> Vec<(String, [i64; 6])> {
    let mut leaves: Vec<_> = model
        .snapshot()
        .leaf_occurrences
        .values()
        .map(|leaf| {
            let name = session.document().get(leaf.component).unwrap().name.clone();
            let bounds = std::array::from_fn(|i| {
                let position = if i < 3 {
                    leaf.bounds.min[i]
                } else {
                    leaf.bounds.max[i - 3]
                };
                (position * 1000.).round() as i64
            });
            (name, bounds)
        })
        .collect();
    leaves.sort();
    leaves
}

fn volume(compiler: &Compiler, session: &Session, body: EntityId) -> f64 {
    let model = compiler
        .compile_sources(session.document(), session.revision(), &[body])
        .unwrap();
    let shape = model.snapshot().body_sources[&body];
    model.snapshot().shapes[&shape].volume_mm3
}

fn shipped() -> Session {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/werkzeugkoffer-2.tcd");
    let loaded = project_file::read(
        &mut std::fs::File::open(path).unwrap(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    Session::new(loaded.project.document).unwrap()
}

#[test]
fn bundled_plywood_drawing_stays_inside_its_printable_area() {
    let session = shipped();
    let document = session.document();
    let revision = session.revision();
    let book = drawings::books(document).next().unwrap();
    let source = drawings::read(document, book).unwrap();
    let [page] = source.pages.as_slice() else {
        panic!("Expected one fabrication page");
    };
    let roots = multicad_model::drawing_preview_sources(document, page.id).unwrap();
    assert_eq!(roots.len(), 2);
    let scoped = compiler()
        .compile_sources(document, revision, &roots)
        .unwrap();
    let preview = scoped
        .drawing_page_preview(document, revision, &DrawingProviders::default(), page.id)
        .unwrap();
    assert!(
        scoped
            .drawing_page(document, revision, &DrawingProviders::default(), page.id)
            .is_err()
    );
    let started = std::time::Instant::now();
    let model = compiler().compile(document, revision).unwrap();
    let drawing = model
        .drawing_page(document, revision, &DrawingProviders::default(), page.id)
        .unwrap();
    eprintln!(
        "Full native WK-101 compile and projection: {:.3?}, {} feature shapes",
        started.elapsed(),
        model.snapshot().shapes.len()
    );
    assert_eq!(preview, drawing);
    assert_eq!(
        drawing
            .checked_page(document, revision, book, page.id)
            .unwrap()
            .len(),
        2
    );
    let conflicts = drawings::page_conflicts(page, 0, 1, &drawing).unwrap();
    assert!(conflicts.is_empty(), "{conflicts:#?}");
}

#[test]
fn bundled_case_matches_generator_and_keeps_export_sources() {
    let shipped = shipped();
    let fresh = demo::build().unwrap();
    for session in [&shipped, &fresh.session] {
        let info = project_file::info::read(session.document()).unwrap();
        assert_eq!(info.name, demo::NAME);
        assert!(info.description.contains("eight small drawers"));
        assert!(info.preview.is_some());
    }

    let compiler = compiler();
    let compile = |session: &Session| {
        compiler
            .compile(session.document(), session.revision())
            .unwrap()
    };
    let closed = compile(&shipped);
    let regenerated = compile(&fresh.session);
    assert_eq!(
        named_geometry(&shipped, &closed),
        named_geometry(&fresh.session, &regenerated)
    );
    let mut panels: Vec<_> = closed
        .cut_list(shipped.document())
        .unwrap()
        .into_iter()
        .map(|panel| {
            assert!(panel.fits_stock && panel.issues.is_empty());
            assert!((panel.cut.thickness_mm - 6.).abs() < 1e-6);
            panel.cut.finished_mm.map(|value| value.round() as u32)
        })
        .collect();
    panels.sort();
    assert_eq!(
        panels,
        [
            [306, 231],
            [306, 231],
            [341, 123],
            [341, 199],
            [341, 231],
            [341, 231],
            [341, 306]
        ]
    );
    for id in &fresh.print_plates {
        let name = &fresh.session.document().get(*id).unwrap().name;
        let existing = read_print_plate(shipped.document(), named(&shipped, name)).unwrap();
        let generated = read_print_plate(fresh.session.document(), *id).unwrap();
        let layout = |plate: multicad_core::exchange::PrintPlateDefinition| {
            plate
                .placements
                .into_iter()
                .map(|placement| (placement.position_mm, placement.rotation_deg))
                .collect::<Vec<_>>()
        };
        assert_eq!(layout(existing), layout(generated), "{name}");
    }
    assert_eq!(fresh.print_plates.len(), 13);
    for id in &fresh.studies {
        let name = &fresh.session.document().get(*id).unwrap().name;
        let existing = read_saved_motion_study(shipped.document(), named(&shipped, name))
            .unwrap()
            .query(&shipped)
            .unwrap();
        let generated = read_saved_motion_study(fresh.session.document(), *id)
            .unwrap()
            .query(&fresh.session)
            .unwrap();
        assert_eq!(
            existing.sample_values().unwrap(),
            generated.sample_values().unwrap(),
            "{name}"
        );
        assert_eq!(existing.pairs.len(), generated.pairs.len(), "{name}");
    }
}

#[test]
fn bundled_case_drawers_move_and_rebuild_from_edited_dimensions() {
    let mut shipped = shipped();
    let compiler = compiler();
    let mut cache = EvaluationCache::default();
    let closed = compiler
        .compile_cached(&mut cache, shipped.document(), shipped.revision(), None)
        .unwrap();
    let actor = Actor::local(Channel::CommandLine);
    let lower = read_saved_motion_study(
        shipped.document(),
        named(&shipped, "Lower drawer clearance"),
    )
    .unwrap();
    let drawer = &lower.pairs[0].a;
    let pose = named(&shipped, "Lower drawer extended");
    let prepared = prepare_apply_pose(&shipped, shipped.revision(), actor.clone(), pose).unwrap();
    let opened = compiler
        .compile_cached(
            &mut cache,
            prepared.prepared.document(),
            shipped.revision(),
            None,
        )
        .unwrap();
    let initial = closed.leaf_snapshot(drawer).unwrap();
    let extended = opened.leaf_snapshot(drawer).unwrap();
    assert!((initial.bounds.min[1] - extended.bounds.min[1] - 185.).abs() < 1e-5);
    for side in ["left", "right"] {
        for (kind, travel) in [("fixed channel", 0.), ("moving member", 185.)] {
            let rail = format!("Drawer 1 · {side} 200 mm slide · {kind}");
            let closed_rail = closed
                .snapshot()
                .leaf_occurrences
                .values()
                .find(|leaf| {
                    shipped
                        .document()
                        .get(leaf.component)
                        .is_some_and(|part| part.name == rail)
                })
                .unwrap_or_else(|| panic!("Missing physical rail {rail}"));
            let open_rail = opened.leaf_snapshot(&closed_rail.path).unwrap();
            assert!((closed_rail.bounds.min[1] - open_rail.bounds.min[1] - travel).abs() < 1e-5);
        }
    }

    let before = volume(&compiler, &shipped, initial.component);
    let rack = closed
        .snapshot()
        .leaf_occurrences
        .values()
        .find(|leaf| {
            shipped
                .document()
                .get(leaf.component)
                .is_some_and(|part| part.name == "Small-drawer rack · 8 bays")
        })
        .expect("Missing physical rack")
        .component;
    let rack_before = volume(&compiler, &shipped, rack);
    let edits = [("drawer_disk_cell", "135"), ("dovetail_fit", "0.20")]
        .into_iter()
        .map(|(name, value)| {
            let mut parameter = shipped
                .document()
                .get(named(&shipped, name))
                .unwrap()
                .clone();
            let EntityData::Parameter { expression, .. } = &mut parameter.data else {
                panic!("Missing editable parameter {name}")
            };
            *expression = value.into();
            Mutation::Replace(parameter)
        })
        .collect();
    let prepared = shipped
        .prepare(Request {
            expected_revision: shipped.revision(),
            actor,
            label: "Resize cutting-disc compartment and adjust dovetail fit".into(),
            edits,
        })
        .unwrap();
    shipped.commit(prepared).unwrap();
    assert!((volume(&compiler, &shipped, initial.component) - before).abs() > 1.);
    assert!((volume(&compiler, &shipped, rack) - rack_before).abs() > 1.);
}
