use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Session},
    constraint_sketch::{CurveKind, source::Source},
    document::{Document, EntityData, EntityId},
    sketch::Datum,
    sketch_draw::{DrawInput, DrawRequest, PointInput},
    sketch_import::{self, Format, ImportOptions, UnitChoice, plan::plan, provenance},
};
use multicad_solver::{
    EditError,
    draw::prepare_draw,
    import::{ImportSource, ImportTarget, prepare_import},
};
use std::collections::BTreeSet;

const DXF: &[u8] = include_bytes!("../../multicad-core/tests/fixtures/sketch_import/plate.dxf");
const SVG: &[u8] = include_bytes!("../../multicad-core/tests/fixtures/sketch_import/badge.svg");

fn actor() -> Actor {
    Actor {
        id: "import-client".into(),
        channel: Channel::Mcp,
    }
}
fn empty() -> Session {
    Session::new(Document::new("Import").unwrap()).unwrap()
}
fn dxf_plan(layers: &[&str]) -> sketch_import::ImportPlan {
    let art = sketch_import::parse(Format::Dxf, DXF, UnitChoice::FromFile).unwrap();
    plan(
        &art,
        &ImportOptions {
            layers: Some(layers.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        },
    )
    .unwrap()
}
fn file(bytes: &'static [u8], name: &'static str) -> ImportSource<'static> {
    ImportSource {
        file_name: name,
        bytes,
    }
}
fn curves(session: &Session, sketch: EntityId) -> usize {
    Source::read_for_evaluation(session.document(), sketch, session.revision(), None)
        .unwrap()
        .0
        .sketch
        .curves
        .len()
}

#[test]
fn new_sketch_import_is_one_undoable_transaction_with_provenance() {
    let mut session = empty();
    let before = session.document().clone();
    let plan = dxf_plan(&["CUT", "ENGRAVE"]);
    let target = ImportTarget::New {
        datum: Datum::Xz,
        name: "Plate".into(),
    };
    let edit = prepare_import(
        &session,
        session.revision(),
        actor(),
        &target,
        None,
        &plan,
        &file(DXF, "/private/fixtures/plate.dxf"),
    )
    .unwrap();
    assert_eq!(session.document(), &before, "preparing changes nothing");
    let sketch = edit.sketch;
    session.commit(edit.edit.preview.unwrap()).unwrap();
    assert_eq!(session.revision(), 1);
    assert_eq!(curves(&session, sketch), 11);
    let (record, linked) =
        provenance::Provenance::read(session.document(), edit.provenance).unwrap();
    assert_eq!(linked, sketch);
    assert_eq!(record.file_name, "plate.dxf");
    assert_eq!(record.sha256, provenance::sha256_hex(DXF));
    assert_eq!(record.layers, ["CUT", "ENGRAVE"]);
    assert_eq!(record.curves, edit.curves);
    assert!(record.unsupported.iter().any(|d| d.kind == "dxf.insert"));
    // Splines make the sketch schema 3; imported curves are ordinary IDs.
    let Some(EntityData::Sketch { definition, .. }) =
        session.document().get(sketch).map(|e| &e.data)
    else {
        panic!()
    };
    assert_eq!(definition.schema_version, 3);
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, actor()).unwrap();
    assert_eq!(curves(&session, sketch), 11);
    assert!(session.document().get(edit.provenance).is_some());
}

#[test]
fn existing_sketch_import_preserves_prior_geometry_and_allows_further_editing() {
    let mut session = empty();
    let (sketch, edits) = Source::create(Datum::Xy, "Artwork").unwrap();
    let document = session.document().edited(&edits).unwrap();
    session = Session::new(document).unwrap();
    let draw = prepare_draw(
        &session,
        0,
        actor(),
        sketch,
        None,
        &DrawRequest {
            input: DrawInput::Line {
                start: PointInput::Position([-50., -50.]),
                end: PointInput::Position([-40., -50.]),
            },
            construction: false,
        },
    )
    .unwrap();
    let original = draw.created_curves[0];
    session.commit(draw.edit.preview.unwrap()).unwrap();
    let art = sketch_import::parse(Format::Svg, SVG, UnitChoice::FromFile).unwrap();
    let options = ImportOptions {
        layers: Some(BTreeSet::from(["Outline".into(), "holes".into()])),
        insertion_mm: [100., 0.],
        ..Default::default()
    };
    let plan = plan(&art, &options).unwrap();
    let edit = prepare_import(
        &session,
        1,
        actor(),
        &ImportTarget::Existing(sketch),
        None,
        &plan,
        &file(SVG, "badge.svg"),
    )
    .unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let (source, _, _) = Source::read_for_evaluation(session.document(), sketch, 2, None).unwrap();
    assert!(source.sketch.curves.iter().any(|c| c.id == original));
    assert_eq!(source.sketch.curves.len(), 1 + 8 + 1 + 1 + 2);
    // The imported circle is a canonical curve that later draws can reference.
    let circle = source
        .sketch
        .curves
        .iter()
        .find_map(|c| match c.kind {
            CurveKind::Circle { center, radius_mm } => Some((center, radius_mm)),
            _ => None,
        })
        .unwrap();
    assert!((circle.1 - 4.).abs() < 1e-9);
    let further = prepare_draw(
        &session,
        2,
        actor(),
        sketch,
        None,
        &DrawRequest {
            input: DrawInput::Line {
                start: PointInput::Existing(circle.0),
                end: PointInput::Position([0., 0.]),
            },
            construction: true,
        },
    )
    .unwrap();
    assert!(further.edit.preview.is_some());
    assert_eq!(provenance::for_sketch(session.document(), sketch).len(), 1);
}

#[test]
fn stale_failed_and_cancelled_imports_leave_the_document_unchanged() {
    let mut session = empty();
    let plan = dxf_plan(&["CUT"]);
    let target = ImportTarget::New {
        datum: Datum::Xy,
        name: "Plate".into(),
    };
    let edit = prepare_import(
        &session,
        0,
        actor(),
        &target,
        None,
        &plan,
        &file(DXF, "plate.dxf"),
    )
    .unwrap();
    // Cancel: dropping the prepared edit commits nothing.
    drop(edit);
    assert_eq!(session.revision(), 0);
    assert_eq!(session.document().entities().count(), 0);
    let stale = prepare_import(
        &session,
        7,
        actor(),
        &target,
        None,
        &plan,
        &file(DXF, "plate.dxf"),
    );
    assert!(matches!(stale, Err(EditError::Command(e)) if e.code == ErrorCode::StaleRevision));
    // A plan whose selected layers produce no curves cannot prepare a commit.
    let notes = dxf_plan(&["NOTES"]);
    assert!(
        prepare_import(
            &session,
            0,
            actor(),
            &target,
            None,
            &notes,
            &file(DXF, "plate.dxf")
        )
        .is_err()
    );
    // A receipt prepared before another commit cannot apply afterwards.
    let first = prepare_import(
        &session,
        0,
        actor(),
        &target,
        None,
        &plan,
        &file(DXF, "plate.dxf"),
    )
    .unwrap();
    let second = prepare_import(
        &session,
        0,
        actor(),
        &target,
        None,
        &plan,
        &file(DXF, "plate.dxf"),
    )
    .unwrap();
    session.commit(first.edit.preview.unwrap()).unwrap();
    let snapshot = session.document().clone();
    assert!(session.commit(second.edit.preview.unwrap()).is_err());
    assert_eq!(session.document(), &snapshot);
}
