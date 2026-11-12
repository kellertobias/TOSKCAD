use multicad_core::{
    assembly_inspection::*,
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, OccurrencePath, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, ProjectFile},
};
use multicad_model::{
    Compiler, GeometryBackend,
    assembly_inspection::{inspect_assembly, inspect_assembly_interruptible},
};
fn parameters(x: f64) -> PrimitiveParameters {
    PrimitiveParameters::Cylinder {
        radius: length(5.),
        height: length(10.),
        frame: PrimitiveFrame {
            origin: [length(x), length(0.), length(0.)],
            ..Default::default()
        },
    }
}
fn model(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
    .unwrap()
}
fn query(s: &Session, a: &PrimitiveRequest, b: &PrimitiveRequest) -> PairQuery {
    PairQuery {
        expected: InspectionStamp::capture(s.document(), s.revision()).unwrap(),
        pairs: vec![OccurrencePair {
            a: OccurrencePath {
                root: a.body,
                members: vec![],
            },
            b: OccurrencePath {
                root: b.body,
                members: vec![],
            },
        }],
        contact_tolerance_mm: 1e-7,
    }
}
#[test]
fn canonical_pair_queries_remain_read_only_and_follow_edits_history_and_reopen() {
    let actor = || Actor::local(Channel::Ui);
    let mut session = Session::new(Document::new("Cylinder inspection").unwrap()).unwrap();
    let a = PrimitiveRequest::new("A", parameters(0.));
    let b = PrimitiveRequest::new("B", parameters(14.));
    for p in [&a, &b] {
        let edit = model::prepare_primitive(&session, session.revision(), actor(), p).unwrap();
        session.commit(edit.prepared).unwrap();
    }
    let before = session.document().clone();
    let first = model(&session);
    let q = query(&session, &a, &b);
    let report = inspect_assembly(&first, &q).unwrap();
    let values = report.pairs[0].result.as_ref().unwrap();
    assert_eq!(values.relation, PairRelation::Separated);
    assert!((values.minimum_clearance_mm - 4.).abs() < 1e-10);
    assert_eq!(values.total_solutions, None);
    assert!(!values.witnesses_complete);
    assert_eq!(session.document(), &before);
    assert_eq!(
        inspect_assembly_interruptible(&first, &q, Some(&|| true))
            .unwrap_err()
            .kind,
        InspectionFailureKind::Cancelled
    );
    let changed = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        b.feature,
        &parameters(5.),
    )
    .unwrap();
    session.commit(changed.prepared).unwrap();
    let next = model(&session);
    assert_eq!(
        inspect_assembly(&next, &q).unwrap_err().kind,
        InspectionFailureKind::Stale
    );
    let report = inspect_assembly(&next, &query(&session, &a, &b)).unwrap();
    let values = report.pairs[0].result.as_ref().unwrap();
    assert_eq!(values.relation, PairRelation::Overlapping);
    let expected = 10. * (50. * (0.5_f64).acos() - 12.5 * 3_f64.sqrt());
    assert!((values.overlap_volume_mm3 - expected).abs() < 1e-9);
    let saved = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &saved);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap();
    let rebuilt = model(&reopened);
    let report = inspect_assembly(&rebuilt, &query(&reopened, &a, &b)).unwrap();
    assert_eq!(
        report.pairs[0].result.as_ref().unwrap().relation,
        PairRelation::Overlapping
    );
}
