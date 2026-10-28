use super::*;
use crate::{inspector_actions, modeling::native_fixture};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::source::Source,
    document::{Entity, EntityData},
    model::{PrimitiveParameters, PrimitiveRequest},
    parameters::BoundExpression,
    sketch::Datum,
    sketch_regions,
};
use multicad_model::Compiler;
use std::sync::Arc;
fn source() -> (Session, Arc<Scene>, EntityId, Vec<GeometryIntent>) {
    let mut document = Document::new("Advanced Inspector source").unwrap();
    document
        .insert(Entity::new(
            "copies",
            EntityData::Parameter {
                expression: "3".into(),
                unit: "count".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let request = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [model::length(60.), model::length(40.), model::length(20.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                &request,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 0).unwrap();
    let topology = scene.geometry_snapshot(request.feature).unwrap();
    let faces = topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .take(2)
        .map(|e| GeometryIntent::capture(&topology, e.key).unwrap())
        .collect();
    (session, scene, request.body, faces)
}
fn request(command: AdvancedCommand) -> model::AdvancedRequest {
    match command {
        AdvancedCommand::Create(r) | AdvancedCommand::Edit(r) => r,
    }
}
fn section(document: Document, name: &str) -> (Document, ProfileSource, EntityId) {
    let (sketch, edits) = Source::create(Datum::Xy, name).unwrap();
    let document = document.edited(&edits).unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let first = source.add_rectangle([0., 0.], [10., 8.]).unwrap();
    let profile = sketch_regions::extract(&source).regions[0].intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    (
        document,
        ProfileSource {
            sketch,
            region: profile,
        },
        first,
    )
}
fn frame(
    ctx: &egui::Context,
    panel: &mut AdvancedPanel,
    document: &Document,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360., 1000.),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            inspector_actions::scope(
                ui,
                Some(inspector_actions::Owner::Model),
                crate::registry::Mode::Model.color(),
                |ui| {
                    panel.show(ui, document, true, PreviewState::Idle);
                },
            );
        },
    )
}
fn click(
    ctx: &egui::Context,
    panel: &mut AdvancedPanel,
    document: &Document,
    label: &str,
    nth: usize,
) {
    frame(ctx, panel, document, vec![]).drop_without_applying_deltas();
    let out = frame(ctx, panel, document, vec![]);
    let pos = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .nth(nth)
        .or_else(|| {
            out.platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .and_then(|(_, n)| n.bounds())
                .map(|b| egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32))
        })
        .unwrap_or_else(|| panic!("Missing {label} #{nth}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            panel,
            document,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
#[test]
fn native_part_forms_freeze_source_validate_faces_and_keep_stable_output_ids() {
    let (session, scene, body, faces) = source();
    let before = session.document().clone();
    for kind in [
        AdvancedKind::Shell,
        AdvancedKind::Thickness,
        AdvancedKind::Draft,
        AdvancedKind::Mirror,
        AdvancedKind::LinearPattern,
        AdvancedKind::CircularPattern,
    ] {
        let mut p = AdvancedPanel::new(
            session.document(),
            session.revision(),
            Some(&scene),
            kind,
            AdvancedSelection {
                body: Some(body),
                ..Default::default()
            },
        )
        .unwrap();
        if kind.face_tool() {
            assert!(p.request(session.document(), true).is_err());
            assert!(p.set_captures(&scene, faces[..1].to_vec()).unwrap());
            assert!(!p.set_captures(&scene, faces[..1].to_vec()).unwrap());
            assert!(
                p.set_captures(&scene, vec![faces[0].clone(), faces[0].clone()])
                    .is_err()
            );
        }
        let first = request(p.request(session.document(), true).unwrap());
        let second = request(p.request(session.document(), true).unwrap());
        assert_eq!(first, second);
        assert_eq!(first.body, body);
        assert_eq!(first.input.as_ref().unwrap().body, body);
        assert!(
            AdvancedPanel::new(
                session.document(),
                session.revision(),
                Some(
                    &scene
                        .isolated_source(first.input.as_ref().unwrap().producer)
                        .unwrap()
                ),
                kind,
                AdvancedSelection {
                    body: Some(body),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    assert_eq!(session.document(), &before);
}
#[test]
fn pattern_count_keeps_bound_expression_and_typed_invalid_inputs_fail_without_source_edits() {
    let (session, scene, body, _) = source();
    let before = session.document().clone();
    let mut p = AdvancedPanel::new(
        session.document(),
        session.revision(),
        Some(&scene),
        AdvancedKind::LinearPattern,
        AdvancedSelection {
            body: Some(body),
            ..Default::default()
        },
    )
    .unwrap();
    let fields::ParameterFields::Linear { count, .. } = &mut p.fields else {
        panic!()
    };
    count.text = "=$copies".into();
    let r = request(p.request(session.document(), true).unwrap());
    let model::AdvancedParameters::LinearPattern(parameters) = r.parameters else {
        panic!()
    };
    let expected = BoundExpression::bind(session.document(), "=$copies", "count", None).unwrap();
    assert_eq!(
        parameters.count,
        multicad_core::constraint_sketch::Dimension::Expression {
            expression: expected
        }
    );
    for invalid in ["NaN", "infinity", "=$missing"] {
        let fields::ParameterFields::Linear { count, .. } = &mut p.fields else {
            panic!()
        };
        count.text = invalid.into();
        assert!(p.request(session.document(), true).is_err());
    }
    assert_eq!(session.document(), &before);
}
#[test]
fn loft_authors_section_order_and_seams_by_pointer_without_uuid_correspondence() {
    let (document, a, seam_a) =
        section(Document::new("Ordered sections").unwrap(), "First section");
    let (document, b, seam_b) = section(document, "Second section");
    let mut p = AdvancedPanel::new(
        &document,
        0,
        None,
        AdvancedKind::Loft,
        AdvancedSelection {
            profile: Some(a.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    p.set_sketch_context(vec![a.clone(), b.clone()], vec![]);
    assert!(p.request(&document, true).is_err());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    click(&ctx, &mut p, &document, "Add section (ordered)", 0);
    click(
        &ctx,
        &mut p,
        &document,
        &inputs::profile_label(&document, &b, &[a.clone(), b.clone()]),
        0,
    );
    let r = request(p.request(&document, true).unwrap());
    assert_eq!(r.profiles, vec![a.clone(), b.clone()]);
    let model::AdvancedParameters::Loft(parameters) = r.parameters else {
        panic!()
    };
    assert_eq!(parameters.seams, vec![seam_a, seam_b]);
    click(&ctx, &mut p, &document, "Move section later", 0);
    let r = request(p.request(&document, true).unwrap());
    assert_eq!(r.profiles, vec![b, a]);
    let model::AdvancedParameters::Loft(parameters) = r.parameters else {
        panic!()
    };
    assert_eq!(parameters.seams, vec![seam_b, seam_a]);
    // Clearing stale choices never rewrites a previously authored section list.
    p.set_sketch_context(vec![], vec![]);
    assert_eq!(
        request(p.request(&document, true).unwrap()).profiles,
        r.profiles
    );
}
