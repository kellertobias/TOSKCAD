use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn s() -> Session {
    Session::new(Document::new("Authored custom connection hardware").unwrap()).unwrap()
}
#[test]
fn actual_custom_geometry_has_explicit_datums_and_distinct_metric_ends() {
    let s = s();
    for p in [
        default_custom_connection_wood_screw(),
        default_custom_connection_wood_insert(),
        default_custom_connection_hanger_bolt(),
    ] {
        let wire = serde_json::to_value(&p).unwrap();
        assert_eq!(wire["kind"], "custom_connection");
        assert!(wire["parameters"]["connection_kind"].is_string());
        assert_eq!(
            serde_json::from_value::<MechanicalPartParameters>(wire).unwrap(),
            p
        );
        let ResolvedMechanicalPart::CustomConnection { parameters } =
            p.resolve(s.parameters()).unwrap()
        else {
            panic!()
        };
        match *parameters {
            ResolvedCustomConnectionHardware::WoodScrew {
                wood_thread,
                wood_thread_start,
                underhead_frame,
                plain_length_mm,
                wood_complete_interval_mm,
                ..
            } => {
                assert_eq!(wood_thread_start.origin_mm, [0.; 3]);
                assert_eq!(underhead_frame.origin_mm, [0., 0., -plain_length_mm]);
                assert_eq!(wood_complete_interval_mm, [0., wood_thread.length_mm]);
                assert!(wood_thread.metric_fit.is_none());
            }
            ResolvedCustomConnectionHardware::WoodInsert {
                outer_thread,
                inner_thread,
                inner_thread_start,
                inner_complete_interval_mm,
                ..
            } => {
                assert_eq!(inner_thread_start.origin_mm, [0.; 3]);
                assert!(inner_thread.length_mm <= outer_thread.length_mm);
                assert!(inner_thread.metric_fit.is_some());
                assert_eq!(inner_complete_interval_mm, [0., inner_thread.length_mm]);
            }
            ResolvedCustomConnectionHardware::HangerBolt {
                wood_thread,
                metric_thread,
                metric_thread_start,
                plain_length_mm,
                metric_complete_interval_mm,
                ..
            } => {
                assert_eq!(
                    metric_thread_start.origin_mm,
                    [0., 0., -plain_length_mm - metric_thread.length_mm]
                );
                assert_eq!(metric_thread.frame.origin, metric_thread_start.origin_mm);
                assert!(wood_thread.metric_fit.is_none());
                assert!(metric_thread.metric_fit.is_some());
                assert_eq!(metric_complete_interval_mm, [0., metric_thread.length_mm]);
            }
        }
    }
}
#[test]
fn fake_metric_wood_cosmetic_unknown_spans_and_insufficient_insert_wall_refuse() {
    let s = s();
    let mut p = default_custom_connection_wood_screw();
    let MechanicalPartParameters::CustomConnection { parameters } = &mut p else {
        panic!()
    };
    let CustomConnectionHardwareParameters::WoodScrew { wood_thread, .. } = parameters.as_mut()
    else {
        panic!()
    };
    wood_thread.family = ThreadFamily::IsoMetric;
    wood_thread.custom = None;
    assert!(p.resolve(s.parameters()).is_err());
    let mut p = default_custom_connection_hanger_bolt();
    let MechanicalPartParameters::CustomConnection { parameters } = &mut p else {
        panic!()
    };
    let CustomConnectionHardwareParameters::HangerBolt { wood_thread, .. } = parameters.as_mut()
    else {
        panic!()
    };
    wood_thread.representation = ThreadRepresentation::CosmeticEnvelope;
    assert!(p.resolve(s.parameters()).is_err());
    let mut p = default_custom_connection_wood_insert();
    let MechanicalPartParameters::CustomConnection { parameters } = &mut p else {
        panic!()
    };
    let CustomConnectionHardwareParameters::WoodInsert { outer_thread, .. } = parameters.as_mut()
    else {
        panic!()
    };
    outer_thread.major_diameter = length(4.);
    assert!(p.resolve(s.parameters()).is_err());
}
#[test]
fn expressions_reader_caps_template_closure_and_history_are_retained() {
    let mut doc = Document::new("Custom source").unwrap();
    let e = Entity::new(
        "plain",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let pid = e.id;
    doc.insert(e).unwrap();
    let mut s = Session::new(doc).unwrap();
    let mut p = default_custom_connection_hanger_bolt();
    let MechanicalPartParameters::CustomConnection { parameters } = &mut p else {
        panic!()
    };
    let CustomConnectionHardwareParameters::HangerBolt { plain_length, .. } = parameters.as_mut()
    else {
        panic!()
    };
    *plain_length = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$plain", "mm", None).unwrap(),
    };
    let r = MechanicalPartRequest::new("Custom hanger", p);
    let before = s.document().clone();
    let q = prepare_mechanical_part(&s, 0, actor(), &r).unwrap();
    assert_eq!(*s.document(), before);
    s.commit(q.prepared).unwrap();
    assert!(
        read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .inputs()
            .unwrap()
            .iter()
            .any(|l| l.target == pid)
    );
    let after = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(*s.document(), before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(*s.document(), after);
    let author = SymbolAuthorRequest::new(
        "Reusable custom hanger",
        r.body,
        SymbolKind::Dynamic,
        vec![pid],
        None,
    );
    let q = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(q.prepared).unwrap();
    let def = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut other = self::s();
    let q = symbols::prepare_import(&other, 0, actor(), &def).unwrap();
    other.commit(q.prepared).unwrap();
    assert!(
        required_capabilities(other.document()).contains(CUSTOM_CONNECTION_HARDWARE_CAPABILITY)
    );
    assert!(required_capabilities(other.document()).contains(METRIC_FIT_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(other.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(CUSTOM_CONNECTION_HARDWARE_CAPABILITY);
    assert!(
        project_file::read(&mut std::io::Cursor::new(bytes), Default::default(), &old).is_err()
    );
}
