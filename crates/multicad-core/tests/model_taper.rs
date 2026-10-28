use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, Mutation},
    model::{self, ExtrudeParameters, ExtrudeRequest, angle, length},
    parameters::BoundExpression,
    sketch::Datum,
    sketch_regions,
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn taper_controls_resolve_signed_units_and_reject_bounds_without_lossy_legacy_sweep() {
    let session = Session::new(Document::new("Taper controls").unwrap()).unwrap();
    for degrees in [-89., -5., 0., 5., 89.] {
        let mut p = ExtrudeParameters {
            taper: angle(degrees),
            ..Default::default()
        };
        let expected = degrees.to_radians();
        assert!(
            (p.resolve_controls(session.parameters()).unwrap().taper_rad - expected).abs() < 1e-12
        );
        p.taper = Dimension::Literal {
            value: expected,
            unit: "rad".into(),
        };
        assert!(
            (p.resolve_controls(session.parameters()).unwrap().taper_rad - expected).abs() < 1e-12
        );
        assert_eq!(p.resolve(session.parameters()).is_ok(), degrees == 0.);
    }
    for degrees in [-180., -90., 90., 180., f64::NAN, f64::INFINITY] {
        let p = ExtrudeParameters {
            taper: angle(degrees),
            ..Default::default()
        };
        assert!(p.resolve_controls(session.parameters()).is_err());
    }
    let p = ExtrudeParameters {
        taper: length(1.),
        ..Default::default()
    };
    assert!(p.resolve_controls(session.parameters()).is_err());
}
#[test]
fn bound_taper_keeps_parameter_identity_through_rename_and_value_edit() {
    let parameter = Entity::new(
        "draft_angle",
        EntityData::Parameter {
            expression: "5".into(),
            unit: "deg".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let document = Document::new("Bound taper")
        .unwrap()
        .edited(&[Mutation::Insert(parameter.clone())])
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let expression =
        BoundExpression::bind(session.document(), "=$draft_angle", "deg", None).unwrap();
    let p = ExtrudeParameters {
        taper: Dimension::Expression { expression },
        ..Default::default()
    };
    let wire = serde_json::to_vec(&p).unwrap();
    assert!(
        (p.resolve_controls(session.parameters()).unwrap().taper_rad - 5_f64.to_radians()).abs()
            < 1e-12
    );
    let mut changed = parameter.clone();
    changed.name = "renamed".into();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        panic!()
    };
    *expression = "-12".into();
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Edit angle".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    assert!(
        (p.resolve_controls(session.parameters()).unwrap().taper_rad + 12_f64.to_radians()).abs()
            < 1e-12
    );
    assert_eq!(serde_json::to_vec(&p).unwrap(), wire);
    session
        .apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Remove angle".into(),
            edits: vec![Mutation::Remove(parameter.id)],
        })
        .unwrap();
    assert!(p.resolve_controls(session.parameters()).is_err());
}
#[test]
fn tapered_source_preparation_is_neutral_until_one_undoable_commit() {
    let (sketch, edits) = Source::create(Datum::Xy, "Profile").unwrap();
    let mut document = Document::new("Tapered source")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([0., 0.], [10., 6.]).unwrap();
    document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let original = session.document().clone();
    let request = ExtrudeRequest::new(
        sketch,
        sketch_regions::extract(&source)
            .regions
            .iter()
            .map(|r| r.intent())
            .collect(),
        ExtrudeParameters {
            taper: angle(-5.),
            ..Default::default()
        },
    );
    let preview = model::prepare_extrude(&session, 0, actor(), &request).unwrap();
    assert_eq!(session.document(), &original);
    session.commit(preview.prepared).unwrap();
    assert_eq!(
        model::read_extrude(session.document(), request.outputs[0].feature)
            .unwrap()
            .parameters,
        request.parameters
    );
    assert_eq!(session.document().get(sketch), original.get(sketch));
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &original);
}
