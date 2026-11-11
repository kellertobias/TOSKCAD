use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    parameters::BoundExpression,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use multicad_model::{Compiler, GeometryBackend};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn fixture(count: usize) -> Session {
    let mut document = Document::new("Parameter consumers").unwrap();
    for name in ["width", "other_width"] {
        document
            .insert(Entity::new(
                name,
                EntityData::Parameter {
                    expression: "2".into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
    }
    let mut session = Session::new(document).unwrap();
    for index in 0..count {
        add_box(&mut session, &format!("Box {index}"), "width");
    }
    session
}
fn add_box(session: &mut Session, name: &str, parameter: &str) {
    let request = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: [
                Dimension::Expression {
                    expression: BoundExpression::bind(
                        session.document(),
                        &format!("=${parameter}"),
                        "mm",
                        None,
                    )
                    .unwrap(),
                },
                length(3.),
                length(4.),
            ],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(
                session,
                session.revision(),
                Actor::local(Channel::Ui),
                &request,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
}
fn host(session: Session) -> Headless {
    Headless::with_geometry_backend(
        Workspace::new(session.document().clone()).unwrap(),
        None,
        false,
        GeometryBackend::Rust,
    )
}

#[test]
fn parameter_preflight_rejects_invalid_consumers_in_scoped_and_large_model_paths() {
    for count in [1, 129] {
        let host = host(fixture(count));
        let before = host.document();
        let error = dispatch(
            &host,
            &call(false),
            "set_parameter",
            json!({"expected_revision":0,"name":"width","expression":"-1","unit":"mm"}),
        )
        .unwrap_err();
        assert_eq!(error.current_revision, Some(0));
        assert_eq!(host.document(), before);
        let reply = dispatch(
            &host,
            &call(false),
            "set_parameter",
            json!({"expected_revision":0,"name":"width","expression":"3","unit":"mm"}),
        )
        .unwrap();
        assert_eq!(reply["revision"], 1);
        let rebuilt = Compiler::default()
            .compile_with_backend(&host.document(), 1, GeometryBackend::Rust, None)
            .unwrap();
        assert_eq!(rebuilt.snapshot().bodies.len(), count);
        for part in rebuilt.snapshot().bodies.keys() {
            assert!((rebuilt.solid(*part).unwrap().volume_mm3().unwrap() - 36.).abs() < 1e-7);
        }
    }
}

#[test]
fn parameter_preflight_skips_unrelated_invalid_parts_and_preserves_cancelled_edits() {
    let mut session = fixture(129);
    add_box(&mut session, "Unrelated unfinished part", "other_width");
    let mut other = session
        .document()
        .entities()
        .find(|e| e.name == "other_width")
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut other.data else {
        unreachable!()
    };
    *expression = "-1".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Unfinished unrelated source".into(),
            edits: vec![Mutation::Replace(other)],
        })
        .unwrap();
    assert!(
        Compiler::default()
            .compile_with_backend(
                session.document(),
                session.revision(),
                GeometryBackend::Rust,
                None
            )
            .is_err()
    );
    let host = host(session);
    let before = host.document();
    let error = dispatch(
        &host,
        &call(true),
        "set_parameter",
        json!({"expected_revision":0,"name":"width","expression":"3","unit":"mm"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.document(), before);
    let reply = dispatch(
        &host,
        &call(false),
        "set_parameter",
        json!({"expected_revision":0,"name":"width","expression":"3","unit":"mm"}),
    )
    .unwrap();
    assert_eq!(reply["revision"], 1);
    let reply = dispatch(
        &host,
        &call(false),
        "set_parameter",
        json!({"expected_revision":1,"name":"unused","expression":"12","unit":"mm"}),
    )
    .unwrap();
    assert_eq!(reply["revision"], 2);
}
