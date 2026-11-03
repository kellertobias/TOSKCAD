use super::*;
use crate::modeling::{body_choices, native_fixture};
use multicad_core::{document::Document, model};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
fn controller() -> ModelToolController {
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
}
fn wait(controller: &mut ModelToolController, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while controller.pending() {
        controller.poll(session, &Default::default());
        assert!(Instant::now() < deadline, "native thread preview timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn native_thread_preview_cancel_apply_undo_redo_and_saved_feature_reopen_share_the_real_gate() {
    let mut session = Session::new(Document::new("Native thread UI").unwrap()).unwrap();
    let before = session.document().clone();
    let mut controller = controller();
    controller
        .begin_thread(
            &session,
            &Default::default(),
            None,
            &BTreeSet::new(),
            Channel::Ui,
        )
        .unwrap();
    assert_eq!(controller.label(), "Thread");
    assert!(!controller.can_apply(&session, &Default::default()));
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    assert_eq!(session.document(), &before);
    controller.cancel();
    assert_eq!(session.document(), &before);
    assert!(!controller.active());
    controller
        .begin_thread(
            &session,
            &Default::default(),
            None,
            &BTreeSet::new(),
            Channel::Ui,
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    let outputs = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(session.revision(), 1);
    let saved = session.document().clone();
    let feature = outputs[0].feature;
    let source = model::read_thread(session.document(), feature).unwrap();
    assert_eq!(
        source.parameters.representation,
        model::ThreadRepresentation::Modeled
    );
    let native = multicad_model::Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &native.snapshot().shapes[&feature];
    assert_eq!(shape.solid_count, 1);
    assert!(
        shape
            .topology
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
            .count()
            > 3
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &saved);
    assert!(ModelToolController::editable(session.document(), feature));
    controller
        .begin_existing(&session, &Default::default(), feature, Channel::Ui)
        .unwrap();
    controller.cancel();
    assert_eq!(session.document(), &saved);
}
#[test]
fn actual_native_receiving_failure_never_enables_apply_or_changes_the_document() {
    let mut session = Session::new(Document::new("Invalid thread receiver").unwrap()).unwrap();
    let stock = model::PrimitiveRequest::new(
        "Thin receiver",
        model::PrimitiveParameters::Cylinder {
            radius: model::length(4.5),
            height: model::length(20.),
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = multicad_model::Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 1).unwrap();
    let choices = body_choices::choices(session.document(), session.revision(), &scene).unwrap();
    let request = model::ThreadRequest::receiving(
        "Thread",
        model::ThreadMode::CutInternal,
        model::BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        model::ThreadParameters {
            length: model::length(8.),
            ..Default::default()
        },
    )
    .unwrap();
    let panel = ThreadPanel::new(
        request,
        session.parameters().clone(),
        choices,
        &BTreeSet::from([stock.body]),
        false,
    )
    .unwrap();
    let before = session.document().clone();
    let mut controller = controller();
    controller
        .begin_form(
            &session,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(panel)),
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(!controller.can_apply(&session, &Default::default()));
    assert!(
        controller
            .error()
            .is_some_and(|e| e.contains("minimum wall")),
        "{:?}",
        controller.error()
    );
    assert!(controller.apply(&mut session, &Default::default()).is_err());
    controller.cancel();
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 1);
}

fn unified_frame(
    ctx: &egui::Context,
    controller: &mut ModelToolController,
    session: &mut Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    controller.poll(session, &Default::default());
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(460., 1800.),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            crate::inspector_actions::scope(
                ui,
                Some(crate::inspector_actions::Owner::Model),
                crate::registry::Mode::Model.color(),
                |ui| {
                    controller.show(ui, session, &Default::default()).unwrap();
                },
            );
        },
    )
}
fn unified_click(ctx: &egui::Context, c: &mut ModelToolController, s: &mut Session, label: &str) {
    unified_frame(ctx, c, s, vec![]).drop_without_applying_deltas();
    let mut out = unified_frame(ctx, c, s, vec![]);
    // Long family menus paint clipped rows. Scroll the real menu before
    // clicking, so a hidden row cannot accidentally target the inspector.
    for _ in 0..4 {
        let painted = out.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some((shape.clip_rect, text.pos + text.galley.size() * 0.5))
            }
            _ => None,
        });
        let target = painted.or_else(|| {
            let clip = out.shapes.iter().rev().find_map(|shape| {
                matches!(shape.shape, egui::Shape::Text(_)).then_some(shape.clip_rect)
            })?;
            let bounds = out
                .platform_output
                .accesskit_update
                .as_ref()?
                .nodes
                .iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })?;
            let center = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.) as f32,
                ((bounds.y0 + bounds.y1) / 2.) as f32,
            );
            Some((clip, center))
        });
        let clipped = target.filter(|(clip, center)| !clip.contains(*center));
        let Some((clip, center)) = clipped else { break };
        out.drop_without_applying_deltas();
        unified_frame(
            ctx,
            c,
            s,
            vec![
                egui::Event::PointerMoved(clip.center()),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., clip.center().y - center.y),
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
        out = unified_frame(ctx, c, s, vec![]);
    }
    let pos = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .or_else(|| {
            out.platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
                .max_by(|a, b| {
                    ((a.x1 - a.x0) * (a.y1 - a.y0)).total_cmp(&((b.x1 - b.x0) * (b.y1 - b.y0)))
                })
                .map(|bounds| {
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) / 2.) as f32,
                        ((bounds.y0 + bounds.y1) / 2.) as f32,
                    )
                })
        })
        .unwrap_or_else(|| {
            let painted: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            panic!("Missing thread control {label}; painted text: {painted:?}")
        });
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        unified_frame(
            ctx,
            c,
            s,
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
fn choose_unified(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    preset: &model::UnifiedThreadPreset,
) {
    unified_click(ctx, c, s, model::ThreadFamily::IsoMetric.label());
    unified_click(ctx, c, s, preset.family.label());
    unified_click(
        ctx,
        c,
        s,
        &format!("Choose a sourced {} preset…", preset.family.label()),
    );
    unified_click(ctx, c, s, &preset.designation);
}
#[test]
fn painted_unc_and_unf_presets_keep_exact_inch_tpi_and_native_cancel_apply_history_and_reopen() {
    for family in [model::ThreadFamily::Unc, model::ThreadFamily::Unf] {
        let preset = model::unified_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut s = Session::new(Document::new("Unified thread controls").unwrap()).unwrap();
        let original = s.document().clone();
        let mut c = controller();
        c.begin_thread(&s, &Default::default(), None, &BTreeSet::new(), Channel::Ui)
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        choose_unified(&ctx, &mut c, &mut s, &preset);
        let request = match c.form.as_ref().unwrap() {
            ModelForm::Thread(panel) => panel.request(s.document(), true).unwrap(),
            _ => panic!("Expected Thread inspector"),
        };
        assert_eq!(request.parameters.family, family);
        assert_eq!(request.parameters.fit, model::ThreadFit::Unified2A);
        assert_eq!(
            request.parameters.major_diameter,
            multicad_core::constraint_sketch::Dimension::Literal {
                value: preset.nominal_diameter_inch,
                unit: "in".into()
            }
        );
        assert_eq!(
            request.parameters.pitch,
            model::length(25.4 / f64::from(preset.threads_per_inch))
        );
        let resolved = request
            .parameters
            .resolve(s.parameters(), request.mode)
            .unwrap();
        assert!(
            (resolved.length_mm - 3. * preset.pitch_mm).abs() < 1e-12,
            "New default length follows its first Unified preset"
        );
        assert_eq!(
            resolved.fit.as_ref().unwrap().class,
            model::ThreadFit::Unified2A
        );
        let out = unified_frame(&ctx, &mut c, &mut s, vec![]);
        assert!(out.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t) if t.galley.job.text.contains(&format!("{} TPI",preset.threads_per_inch)))));
        out.drop_without_applying_deltas();
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &original);
        c.cancel();
        assert_eq!(s.document(), &original);
        let panel = ThreadPanel::new(
            request.clone(),
            s.parameters().clone(),
            vec![],
            &BTreeSet::new(),
            false,
        )
        .unwrap();
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(panel)),
        )
        .unwrap();
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        let output = c.apply(&mut s, &Default::default()).unwrap();
        assert_eq!(output[0].feature, request.feature);
        let saved = s.document().clone();
        assert_eq!(
            model::read_thread(s.document(), request.feature)
                .unwrap()
                .parameters,
            request.parameters
        );
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &original);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &saved);
        let project = multicad_core::project_file::ProjectFile::new(saved.clone()).unwrap();
        let mut bytes = vec![];
        multicad_core::project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let reopened = multicad_core::project_file::read(
            &mut &bytes[..],
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let independent = Session::new(reopened.document).unwrap();
        let mut reopened_controller = controller();
        reopened_controller
            .begin_existing(
                &independent,
                &Default::default(),
                request.feature,
                Channel::Ui,
            )
            .unwrap();
        let restored = match reopened_controller.form.as_ref().unwrap() {
            ModelForm::Thread(panel) => panel.request(independent.document(), true).unwrap(),
            _ => panic!("Expected restored Thread inspector"),
        };
        assert_eq!(restored.parameters, request.parameters);
        reopened_controller
            .preview(&independent, &Default::default(), Channel::Ui)
            .unwrap();
        wait(&mut reopened_controller, &independent);
        assert!(reopened_controller.can_apply(&independent, &Default::default()));
        reopened_controller.cancel();
        assert_eq!(independent.document(), &saved);
    }
}
#[test]
fn nested_hex_nut_unified_controls_use_internal_class_and_keep_authored_height() {
    let preset = model::unified_thread_presets()
        .into_iter()
        .find(|p| p.family == model::ThreadFamily::Unf)
        .unwrap();
    let mut s = Session::new(Document::new("Unified hex nut controls").unwrap()).unwrap();
    let original = s.document().clone();
    let request = model::MechanicalPartRequest::new(
        "Nut",
        model::MechanicalPartParameters::HexNut {
            across_flats: model::length(12.),
            thread: Box::new(model::ThreadParameters {
                major_diameter: model::length(6.),
                pitch: model::length(1.),
                length: model::length(4.2),
                ..Default::default()
            }),
        },
    );
    let panel = MechanicalPartPanel::new(request.clone(), s.parameters().clone(), false).unwrap();
    let mut c = controller();
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::MechanicalPart(Box::new(panel)),
    )
    .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    choose_unified(&ctx, &mut c, &mut s, &preset);
    let candidate = match c.form.as_ref().unwrap() {
        ModelForm::MechanicalPart(panel) => panel.request(s.document(), true).unwrap(),
        _ => panic!("Expected nested hardware controls"),
    };
    let model::MechanicalPartParameters::HexNut { thread, .. } = &candidate.parameters else {
        panic!("Expected hex nut")
    };
    assert_eq!(thread.fit, model::ThreadFit::Unified2B);
    assert_eq!(thread.length, model::length(4.2));
    assert_eq!(
        thread.major_diameter,
        multicad_core::constraint_sketch::Dimension::Literal {
            value: preset.nominal_diameter_inch,
            unit: "in".into()
        }
    );
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    assert_eq!(s.document(), &original);
    c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(
        model::read_mechanical_part(s.document(), request.feature)
            .unwrap()
            .parameters,
        candidate.parameters
    );
}

#[test]
fn reopening_a_default_valued_native_nut_keeps_its_authored_height_when_selecting_unified() {
    let mut source =
        Session::new(Document::new("Existing default-valued M8 nut").unwrap()).unwrap();
    let request = model::MechanicalPartRequest::new(
        "Saved M8 nut",
        model::MechanicalPartParameters::HexNut {
            across_flats: model::length(14.),
            thread: Box::new(model::ThreadParameters::default()),
        },
    );
    let prepared = model::prepare_mechanical_part(
        &source,
        source.revision(),
        Actor::local(Channel::Ui),
        &request,
    )
    .unwrap();
    let native = multicad_model::Compiler::default()
        .compile(prepared.prepared.document(), source.revision() + 1)
        .unwrap();
    assert_eq!(native.snapshot().shapes[&request.feature].solid_count, 1);
    assert!(
        native.snapshot().shapes[&request.feature]
            .topology
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
            .count()
            > 3
    );
    source.commit(prepared.prepared).unwrap();
    source.mark_saved(source.revision()).unwrap();
    let saved = source.document().clone();
    for family in [model::ThreadFamily::Unc, model::ThreadFamily::Unf] {
        let preset = model::unified_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut s = Session::new(saved.clone()).unwrap();
        let mut c = controller();
        c.begin_existing(&s, &Default::default(), request.feature, Channel::Ui)
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        choose_unified(&ctx, &mut c, &mut s, &preset);
        let error = match c.form.as_ref().unwrap() {
            ModelForm::MechanicalPart(panel) => panel.request(s.document(), true).unwrap_err(),
            _ => panic!("Expected reopened hardware inspector"),
        };
        assert!(
            error.contains("1.5"),
            "Retained 12.5 mm height must require explicit correction: {error}"
        );
        assert!(c.preview(&s, &Default::default(), Channel::Ui).is_err());
        assert!(!c.can_apply(&s, &Default::default()));
        assert_eq!(s.document(), &saved);

        let display =
            crate::modeling::dimension::DimensionField::from_dimension(&model::length(12.5), "mm")
                .unwrap()
                .text;
        let out = unified_frame(&ctx, &mut c, &mut s, vec![]);
        let bounds = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                (node.role() == egui::accesskit::Role::TextInput
                    && node.value() == Some(display.as_str()))
                .then(|| node.bounds())
                .flatten()
            })
            .find(|b| b.x1 - b.x0 > 50.)
            .expect("Retained authored nut height must remain visible");
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.) as f32,
            ((bounds.y0 + bounds.y1) / 2.) as f32,
        );
        out.drop_without_applying_deltas();
        for pressed in [true, false] {
            unified_frame(
                &ctx,
                &mut c,
                &mut s,
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
        unified_frame(
            &ctx,
            &mut c,
            &mut s,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("8 mm".into()),
            ],
        )
        .drop_without_applying_deltas();
        let candidate = match c.form.as_ref().unwrap() {
            ModelForm::MechanicalPart(panel) => panel.request(s.document(), true).unwrap(),
            _ => panic!("Expected edited hardware inspector"),
        };
        let model::MechanicalPartParameters::HexNut { thread, .. } = candidate.parameters else {
            panic!("Expected nut")
        };
        assert_eq!(thread.length, model::length(8.));
        assert_eq!(thread.fit, model::ThreadFit::Unified2B);
        c.cancel();
        assert_eq!(s.document(), &saved);
    }
}

fn choose_rounded(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    preset: &model::WhitworthThreadPreset,
) {
    unified_click(ctx, c, s, model::ThreadFamily::IsoMetric.label());
    unified_click(ctx, c, s, preset.family.label());
    unified_click(
        ctx,
        c,
        s,
        &format!("Choose a sourced {} preset…", preset.family.label()),
    );
    unified_click(ctx, c, s, &preset.designation);
}
#[test]
fn painted_bspt_mode_changes_preserve_length_and_native_preview_cancel_apply_history() {
    let mut s = Session::new(Document::new("BSPT inspector").unwrap()).unwrap();
    let before = s.document().clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut c = controller();
    let request = model::ThreadRequest::new(
        "Pipe",
        model::ThreadParameters {
            length: model::length(8.),
            ..Default::default()
        },
    );
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::Thread(Box::new(
            ThreadPanel::new(
                request,
                s.parameters().clone(),
                vec![],
                &BTreeSet::new(),
                false,
            )
            .unwrap(),
        )),
    )
    .unwrap();
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Bspt.label());
    unified_click(&ctx, &mut c, &mut s, "Choose sourced R/Rc size…");
    unified_click(&ctx, &mut c, &mut s, "R 1/8 · 28 TPI");
    let recipe = |c: &ModelToolController, mode| match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel.recipe(s.document(), mode).unwrap(),
        _ => panic!(),
    };
    let original = recipe(&c, model::ThreadMode::NewExternal);
    assert_eq!(original.length, model::length(8.));
    assert_eq!(original.pipe.unwrap().form, model::PipeThreadForm::R);
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Bspt.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Bspt.label());
    let restored = match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel
            .recipe(s.document(), model::ThreadMode::NewExternal)
            .unwrap(),
        _ => panic!(),
    };
    assert_eq!(restored, original);
    unified_click(&ctx, &mut c, &mut s, "New threaded shaft");
    unified_click(&ctx, &mut c, &mut s, "Cut internal thread");
    let internal = match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel
            .recipe(s.document(), model::ThreadMode::CutInternal)
            .unwrap(),
        _ => panic!(),
    };
    assert_eq!(internal.pipe.unwrap().form, model::PipeThreadForm::Rc);
    assert_eq!(
        internal.major_diameter,
        model::length(internal.pipe.unwrap().base_major_mm())
    );
    assert_eq!(internal.length, model::length(8.));
    unified_click(&ctx, &mut c, &mut s, "Cut internal thread");
    unified_click(&ctx, &mut c, &mut s, "New threaded shaft");
    let request = match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel.request(s.document(), true).unwrap(),
        _ => panic!(),
    };
    assert_eq!(request.parameters, original);
    unified_click(&ctx, &mut c, &mut s, "Preview");
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    assert_eq!(s.document(), &before);
    unified_click(&ctx, &mut c, &mut s, "Cancel");
    assert_eq!(s.document(), &before);
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::Thread(Box::new(
            ThreadPanel::new(
                request,
                s.parameters().clone(),
                vec![],
                &BTreeSet::new(),
                false,
            )
            .unwrap(),
        )),
    )
    .unwrap();
    unified_click(&ctx, &mut c, &mut s, "Preview");
    wait(&mut c, &s);
    let outputs = c.apply(&mut s, &Default::default()).unwrap();
    let feature = outputs[0].feature;
    assert_eq!(
        model::read_thread(s.document(), feature)
            .unwrap()
            .parameters,
        original
    );
    let saved = s.document().clone();
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &saved);
    c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
        .unwrap();
    c.cancel();
    assert_eq!(s.document(), &saved);
}
#[test]
fn painted_bsw_bsf_and_bspp_presets_keep_physical_units_fit_rounding_and_native_cancel_apply_history_reopen()
 {
    for family in [
        model::ThreadFamily::Bsw,
        model::ThreadFamily::Bsf,
        model::ThreadFamily::Bspp,
    ] {
        let preset = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut s = Session::new(Document::new("Rounded thread UI").unwrap()).unwrap();
        let before = s.document().clone();
        let mut c = controller();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        c.begin_thread(&s, &Default::default(), None, &BTreeSet::new(), Channel::Ui)
            .unwrap();
        choose_rounded(&ctx, &mut c, &mut s, &preset);
        let request = match c.form.as_ref().unwrap() {
            ModelForm::Thread(panel) => panel.request(s.document(), true).unwrap(),
            _ => panic!("Expected Thread inspector"),
        };
        assert_eq!(request.parameters.family, family);
        assert_eq!(
            request.parameters.fit,
            if family == model::ThreadFamily::Bspp {
                model::ThreadFit::BsppClassBExternal
            } else {
                model::ThreadFit::WhitworthMediumExternal
            }
        );
        assert_eq!(
            request.parameters.major_diameter,
            if let Some(inches) = preset.nominal_diameter_inch {
                multicad_core::constraint_sketch::Dimension::Literal {
                    value: inches,
                    unit: "in".into(),
                }
            } else {
                model::length(preset.diameter_mm)
            }
        );
        assert_eq!(
            request.parameters.pitch,
            model::length(25.4 / f64::from(preset.threads_per_inch))
        );
        assert_eq!(
            request.parameters.length,
            model::length(3. * preset.pitch_mm)
        );
        let resolved = request
            .parameters
            .resolve(s.parameters(), request.mode)
            .unwrap();
        let a = 27.5_f64.to_radians();
        assert!(
            (resolved.profile.rounding_radius_mm.unwrap()
                - preset.pitch_mm * a.cos() / (12. * (1. - a.sin())))
            .abs()
                < 1e-12
        );
        assert_eq!(resolved.profile.crest_width_mm, 0.);
        unified_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        unified_click(&ctx, &mut c, &mut s, "Cancel");
        assert!(!c.active());
        assert_eq!(s.document(), &before);
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(
                ThreadPanel::new(
                    request.clone(),
                    s.parameters().clone(),
                    vec![],
                    &BTreeSet::new(),
                    false,
                )
                .unwrap(),
            )),
        )
        .unwrap();
        unified_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        unified_click(&ctx, &mut c, &mut s, "Apply");
        assert_eq!(s.revision(), 1);
        let source = model::read_thread(s.document(), request.feature).unwrap();
        assert_eq!(source.parameters, request.parameters);
        assert_eq!(source.output.body, request.body);
        let saved = s.document().clone();
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &saved);
        let mut bytes = vec![];
        multicad_core::project_file::write(
            &multicad_core::project_file::ProjectFile::new(saved.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = multicad_core::project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let mut independent = Session::new(reopened.document).unwrap();
        assert_eq!(independent.revision(), 0);
        c.begin_existing(
            &independent,
            &Default::default(),
            request.feature,
            Channel::Ui,
        )
        .unwrap();
        unified_click(&ctx, &mut c, &mut independent, "Preview");
        wait(&mut c, &independent);
        assert!(
            c.can_apply(&independent, &Default::default()),
            "{:?}",
            c.error()
        );
        unified_click(&ctx, &mut c, &mut independent, "Cancel");
        assert_eq!(independent.document(), &saved);
    }
}

#[test]
fn rounded_preset_selection_preserves_explicit_authored_length_and_invalid_recipes_stay_atomic() {
    for family in [
        model::ThreadFamily::Bsw,
        model::ThreadFamily::Bsf,
        model::ThreadFamily::Bspp,
    ] {
        let preset = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut s = Session::new(Document::new("Authored rounded lengths").unwrap()).unwrap();
        let before = s.document().clone();
        let mut c = controller();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let authored = model::ThreadRequest::new(
            "Authored length",
            model::ThreadParameters {
                length: model::length(7.25),
                ..Default::default()
            },
        );
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(
                ThreadPanel::new(
                    authored,
                    s.parameters().clone(),
                    vec![],
                    &BTreeSet::new(),
                    true,
                )
                .unwrap(),
            )),
        )
        .unwrap();
        choose_rounded(&ctx, &mut c, &mut s, &preset);
        let valid = match c.form.as_ref().unwrap() {
            ModelForm::Thread(panel) => panel.request(s.document(), true).unwrap(),
            _ => panic!(),
        };
        assert_eq!(valid.parameters.length, model::length(7.25));
        c.cancel();
        for p in [
            model::ThreadParameters {
                family: model::ThreadFamily::Whitworth,
                ..valid.parameters.clone()
            },
            model::ThreadParameters {
                axial_allowance: model::length(0.001),
                ..valid.parameters.clone()
            },
            model::ThreadParameters {
                fit: model::ThreadFit::NominalBasic,
                ..valid.parameters.clone()
            },
            model::ThreadParameters {
                pitch: model::length(preset.pitch_mm * 1.01),
                ..valid.parameters.clone()
            },
        ] {
            c.begin_form(
                &s,
                &Default::default(),
                Channel::Ui,
                ModelForm::Thread(Box::new(
                    ThreadPanel::new(
                        model::ThreadRequest::new("Invalid rounded", p),
                        s.parameters().clone(),
                        vec![],
                        &BTreeSet::new(),
                        false,
                    )
                    .unwrap(),
                )),
            )
            .unwrap();
            assert!(c.preview(&s, &Default::default(), Channel::Ui).is_err());
            assert!(!c.can_apply(&s, &Default::default()));
            assert!(c.apply(&mut s, &Default::default()).is_err());
            c.cancel();
            assert_eq!(s.document(), &before);
            assert_eq!(s.revision(), 0);
        }
    }
}

#[test]
fn painted_npt_mode_changes_preserve_length_and_native_preview_cancel_apply_history() {
    let mut s = Session::new(Document::new("NPT inspector").unwrap()).unwrap();
    let before = s.document().clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut c = controller();
    let request = model::ThreadRequest::new(
        "Pipe",
        model::ThreadParameters {
            length: model::length(0.4018 * 25.4),
            ..Default::default()
        },
    );
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::Thread(Box::new(
            ThreadPanel::new(
                request,
                s.parameters().clone(),
                vec![],
                &BTreeSet::new(),
                false,
            )
            .unwrap(),
        )),
    )
    .unwrap();
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Npt.label());
    unified_click(&ctx, &mut c, &mut s, "Choose sourced NPT size…");
    unified_click(&ctx, &mut c, &mut s, "NPT 1/4 · 18 TPI");
    let recipe = |c: &ModelToolController, mode| match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel.recipe(s.document(), mode).unwrap(),
        _ => panic!(),
    };
    let original = recipe(&c, model::ThreadMode::NewExternal);
    assert_eq!(original.length, model::length(0.4018 * 25.4));
    assert_eq!(original.npt.unwrap().form, model::NptThreadForm::External);
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Npt.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
    unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Npt.label());
    let restored = match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel
            .recipe(s.document(), model::ThreadMode::NewExternal)
            .unwrap(),
        _ => panic!(),
    };
    assert_eq!(restored, original);
    unified_click(&ctx, &mut c, &mut s, "New threaded shaft");
    unified_click(&ctx, &mut c, &mut s, "Cut internal thread");
    let internal = match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel
            .recipe(s.document(), model::ThreadMode::CutInternal)
            .unwrap(),
        _ => panic!(),
    };
    assert_eq!(internal.npt.unwrap().form, model::NptThreadForm::Internal);
    assert_eq!(
        internal.major_diameter,
        model::length(internal.npt.unwrap().base_major_mm())
    );
    assert_eq!(internal.length, model::length(0.4018 * 25.4));
    unified_click(&ctx, &mut c, &mut s, "Cut internal thread");
    unified_click(&ctx, &mut c, &mut s, "New threaded shaft");
    let request = match c.form.as_ref().unwrap() {
        ModelForm::Thread(panel) => panel.request(s.document(), true).unwrap(),
        _ => panic!(),
    };
    assert_eq!(request.parameters, original);
    unified_click(&ctx, &mut c, &mut s, "Preview");
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    assert_eq!(s.document(), &before);
    unified_click(&ctx, &mut c, &mut s, "Cancel");
    assert_eq!(s.document(), &before);
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::Thread(Box::new(
            ThreadPanel::new(
                request,
                s.parameters().clone(),
                vec![],
                &BTreeSet::new(),
                false,
            )
            .unwrap(),
        )),
    )
    .unwrap();
    unified_click(&ctx, &mut c, &mut s, "Preview");
    wait(&mut c, &s);
    let outputs = c.apply(&mut s, &Default::default()).unwrap();
    let feature = outputs[0].feature;
    assert_eq!(
        model::read_thread(s.document(), feature)
            .unwrap()
            .parameters,
        original
    );
    let saved = s.document().clone();
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &saved);
    c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
        .unwrap();
    c.cancel();
    assert_eq!(s.document(), &saved);
}

#[test]
fn painted_lead_screw_presets_keep_literal_length_left_hand_multiple_starts_and_native_history() {
    for row in model::lead_screw_presets()
        .into_iter()
        .filter(|r| r.designation == "Tr10 × 2" || r.designation == "1/2-10 Acme")
    {
        let mut s = Session::new(Document::new("Lead screw inspector").unwrap()).unwrap();
        let before = s.document().clone();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut c = controller();
        let request = model::ThreadRequest::new(
            "Authored lead length",
            model::ThreadParameters {
                length: model::length(9.5),
                ..Default::default()
            },
        );
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(
                ThreadPanel::new(
                    request,
                    s.parameters().clone(),
                    vec![],
                    &BTreeSet::new(),
                    false,
                )
                .unwrap(),
            )),
        )
        .unwrap();
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
        unified_click(&ctx, &mut c, &mut s, row.family.label());
        unified_click(
            &ctx,
            &mut c,
            &mut s,
            &format!("Choose a sourced {} preset…", row.family.label()),
        );
        unified_click(&ctx, &mut c, &mut s, row.designation);
        unified_click(&ctx, &mut c, &mut s, "Left hand");
        unified_click(&ctx, &mut c, &mut s, "1");
        unified_frame(
            &ctx,
            &mut c,
            &mut s,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("2".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
        let request = match c.form.as_ref().unwrap() {
            ModelForm::Thread(p) => p.request(s.document(), true).unwrap(),
            _ => panic!(),
        };
        assert_eq!(request.parameters.length, model::length(9.5));
        assert_eq!(request.parameters.hand, model::ThreadHand::Left);
        assert_eq!(request.parameters.starts, 2);
        assert_eq!(request.parameters.family, row.family);
        assert_eq!(request.parameters.fit, model::ThreadFit::NominalBasic);
        let r = request
            .parameters
            .resolve(s.parameters(), request.mode)
            .unwrap();
        assert!((r.lead_mm - 2. * row.pitch_mm).abs() < 1e-12);
        assert_eq!(r.lead_screw.unwrap().preset, row);
        unified_click(&ctx, &mut c, &mut s, row.family.label());
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
        unified_click(&ctx, &mut c, &mut s, row.family.label());
        let restored = match c.form.as_ref().unwrap() {
            ModelForm::Thread(p) => p.request(s.document(), true).unwrap(),
            _ => panic!(),
        };
        assert_eq!(restored.parameters, request.parameters);
        unified_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        unified_click(&ctx, &mut c, &mut s, "Cancel");
        assert_eq!(s.document(), &before);
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(
                ThreadPanel::new(
                    request.clone(),
                    s.parameters().clone(),
                    vec![],
                    &BTreeSet::new(),
                    false,
                )
                .unwrap(),
            )),
        )
        .unwrap();
        unified_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        let outputs = c.apply(&mut s, &Default::default()).unwrap();
        let feature = outputs[0].feature;
        assert_eq!(
            model::read_thread(s.document(), feature)
                .unwrap()
                .parameters,
            request.parameters
        );
        let saved = s.document().clone();
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &saved);
        c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
            .unwrap();
        c.cancel();
        assert_eq!(s.document(), &saved);
    }
}

#[test]
fn painted_buttress_preset_keeps_literal_length_and_independent_hand_load_side_with_native_history()
{
    for row in model::buttress_thread_presets() {
        let mut s = Session::new(Document::new("Lead screw inspector").unwrap()).unwrap();
        let before = s.document().clone();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut c = controller();
        let request = model::ThreadRequest::new(
            "Authored lead length",
            model::ThreadParameters {
                length: model::length(9.5),
                ..Default::default()
            },
        );
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(
                ThreadPanel::new(
                    request,
                    s.parameters().clone(),
                    vec![],
                    &BTreeSet::new(),
                    false,
                )
                .unwrap(),
            )),
        )
        .unwrap();
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Buttress.label());
        unified_click(&ctx, &mut c, &mut s, "Choose sourced ST size…");
        unified_click(&ctx, &mut c, &mut s, row.designation);
        unified_click(&ctx, &mut c, &mut s, "Left hand");
        unified_click(&ctx, &mut c, &mut s, "Positive local axis side");
        let request = match c.form.as_ref().unwrap() {
            ModelForm::Thread(p) => p.request(s.document(), true).unwrap(),
            _ => panic!(),
        };
        assert_eq!(request.parameters.length, model::length(9.5));
        assert_eq!(request.parameters.hand, model::ThreadHand::Left);
        assert_eq!(request.parameters.starts, 1);
        assert_eq!(
            request.parameters.buttress.unwrap().load_flank_side,
            model::ButtressLoadFlankSide::PositiveAxisSide
        );
        assert_eq!(request.parameters.family, model::ThreadFamily::Buttress);
        assert_eq!(request.parameters.fit, model::ThreadFit::NominalBasic);
        let r = request
            .parameters
            .resolve(s.parameters(), request.mode)
            .unwrap();
        assert!((r.lead_mm - row.pitch_mm).abs() < 1e-12);
        assert_eq!(r.buttress.unwrap().preset, row);
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Buttress.label());
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::IsoMetric.label());
        unified_click(&ctx, &mut c, &mut s, model::ThreadFamily::Buttress.label());
        let restored = match c.form.as_ref().unwrap() {
            ModelForm::Thread(p) => p.request(s.document(), true).unwrap(),
            _ => panic!(),
        };
        assert_eq!(restored.parameters, request.parameters);
        unified_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        unified_click(&ctx, &mut c, &mut s, "Cancel");
        assert_eq!(s.document(), &before);
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(
                ThreadPanel::new(
                    request.clone(),
                    s.parameters().clone(),
                    vec![],
                    &BTreeSet::new(),
                    false,
                )
                .unwrap(),
            )),
        )
        .unwrap();
        unified_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        let outputs = c.apply(&mut s, &Default::default()).unwrap();
        let feature = outputs[0].feature;
        assert_eq!(
            model::read_thread(s.document(), feature)
                .unwrap()
                .parameters,
            request.parameters
        );
        let saved = s.document().clone();
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &saved);
        c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
            .unwrap();
        c.cancel();
        assert_eq!(s.document(), &saved);
    }
}
