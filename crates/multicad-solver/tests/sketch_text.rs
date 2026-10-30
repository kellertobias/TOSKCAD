//! Sketch text (CAD-142): one undoable transaction per placement or edit,
//! refusals never change the document, and project files round-trip the text
//! exactly behind their own capability.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{
        source::Source,
        text::{CAPABILITY, TextAlignment, TextAnchor, TextFont, TextOperation, TextStyle},
    },
    document::*,
    project_file::{self, ProjectFile},
    sketch::{Datum, Geometry},
};
use multicad_solver::{evaluate_document_sketch, prepare_text};

fn actor() -> Actor {
    Actor {
        id: "text".into(),
        channel: Channel::Ui,
    }
}
fn setup() -> (Session, EntityId) {
    let (id, edits) = Source::create(Datum::Xy, "Lettering").unwrap();
    let document = Document::new("Lettering").unwrap().edited(&edits).unwrap();
    (Session::new(document).unwrap(), id)
}
fn apply(session: &mut Session, sketch: EntityId, operation: TextOperation) -> EntityId {
    let prepared = prepare_text(
        session,
        session.revision(),
        actor(),
        sketch,
        None,
        &operation,
    )
    .unwrap();
    session.commit(prepared.edit.preview.unwrap()).unwrap();
    prepared.text
}
fn texts(
    session: &Session,
    sketch: EntityId,
) -> Vec<multicad_core::constraint_sketch::text::SketchText> {
    Source::read_for_evaluation(session.document(), sketch, session.revision(), None)
        .unwrap()
        .0
        .sketch
        .texts
}

#[test]
fn placement_and_edit_are_single_undo_steps_and_display_exact_contours() {
    let (mut session, sketch) = setup();
    let empty = session.document().clone();
    let id = apply(
        &mut session,
        sketch,
        TextOperation::Add {
            style: TextStyle::new("ABO", 10., [0., 0.]),
        },
    );
    let placed = session.document().clone();
    assert_eq!(texts(&session, sketch)[0].style.text, "ABO");
    let evaluated =
        evaluate_document_sketch(session.document(), session.revision(), sketch, None).unwrap();
    let geometry = evaluated.geometry().unwrap();
    assert_eq!(geometry.len(), 7, "seven exact closed glyph contours");
    assert!(
        geometry
            .iter()
            .all(|g| matches!(g, Geometry::BSpline { poles, .. } if poles.first() == poles.last()))
    );
    // Glyph contours are derived, never solver points or source curves.
    assert!(evaluated.solved().unwrap().curves.is_empty());

    let mut style = TextStyle::new("ABD", 12., [5., 5.]);
    style.rotation_deg = 15.;
    apply(
        &mut session,
        sketch,
        TextOperation::Edit {
            text: id,
            style: style.clone(),
        },
    );
    assert_eq!(texts(&session, sketch)[0].style, style);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(
        session.document(),
        &placed,
        "one undo restores the placement"
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &empty, "one undo removes the placement");
    session.redo(session.revision(), actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(texts(&session, sketch)[0].style, style);
    apply(&mut session, sketch, TextOperation::Remove { text: id });
    assert!(texts(&session, sketch).is_empty());
}

#[test]
fn refused_text_names_the_problem_and_changes_nothing() {
    let (mut session, sketch) = setup();
    let id = apply(
        &mut session,
        sketch,
        TextOperation::Add {
            style: TextStyle::new("OK", 5., [0., 0.]),
        },
    );
    let before = session.document().clone();
    let revision = session.revision();
    let mut zero = TextStyle::new("AB", 0., [0., 0.]);
    let mut negative = zero.clone();
    negative.height_mm = -2.;
    let mut huge = zero.clone();
    huge.height_mm = 1e6;
    zero.height_mm = 0.;
    for (operation, needle) in [
        (
            TextOperation::Add {
                style: TextStyle::new("", 5., [0., 0.]),
            },
            "empty",
        ),
        (TextOperation::Add { style: zero }, "height"),
        (TextOperation::Add { style: negative }, "height"),
        (TextOperation::Add { style: huge }, "height"),
        (
            TextOperation::Add {
                style: TextStyle::new("A😀", 5., [0., 0.]),
            },
            "not supported",
        ),
        (
            TextOperation::Add {
                style: TextStyle::new("B".repeat(30), 5., [0., 0.]),
            },
            "budget",
        ),
        (
            TextOperation::Edit {
                text: EntityId::new(),
                style: TextStyle::new("X", 5., [0., 0.]),
            },
            "does not exist",
        ),
        (
            TextOperation::Edit {
                text: id,
                style: TextStyle::new("OK", 5., [0., 0.]),
            },
            "unchanged",
        ),
    ] {
        let error = match prepare_text(&session, revision, actor(), sketch, None, &operation) {
            Ok(_) => panic!("{operation:?} was accepted"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains(needle), "{operation:?}: {error}");
        assert_eq!(session.document(), &before);
    }
    let stale = prepare_text(
        &session,
        revision - 1,
        actor(),
        sketch,
        None,
        &TextOperation::Remove { text: id },
    );
    assert!(stale.is_err());
    assert_eq!(session.revision(), revision);
}

#[test]
fn project_file_round_trips_text_and_requires_its_capability() {
    let (mut session, sketch) = setup();
    let mut style = TextStyle::new("Lid 2", 7.5, [-3.25, 1.]);
    style.rotation_deg = 33.;
    apply(&mut session, sketch, TextOperation::Add { style });
    // CAD-149 placement: multi-line, anchor, alignment and the second font.
    let mut style = TextStyle::new("Top\nside B", 4., [20., -8.5]);
    style.anchor = TextAnchor::RightCenter;
    style.alignment = TextAlignment::Right;
    style.font = TextFont::HackRegular;
    apply(&mut session, sketch, TextOperation::Add { style });
    let document = session.document().clone();
    let project = ProjectFile::new(document.clone()).unwrap();
    assert!(project.capabilities().contains(CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, document, "text reopens exactly");
    let reopened = Session::new(opened.project.document).unwrap();
    assert_eq!(texts(&reopened, sketch), texts(&session, sketch));
    let mut older = project_file::supported_capabilities();
    older.remove(CAPABILITY);
    let refused = project_file::read(&mut &bytes[..], Default::default(), &older);
    assert!(
        refused.is_err(),
        "readers without text support refuse clearly"
    );
}
