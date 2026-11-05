use super::*;
use crate::modeling::native_fixture;
use multicad_core::{document::Document, model};
use std::time::{Duration, Instant};
fn controller() -> ModelToolController {
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c
}
fn wait(c: &mut ModelToolController, s: &Session) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < deadline, "native gear preview timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn ring_frame(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    c.poll(s, &Default::default());
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(650., 2400.),
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
                    c.show(ui, s, &Default::default()).unwrap();
                },
            );
        },
    )
}
fn ring_click(ctx: &egui::Context, c: &mut ModelToolController, s: &mut Session, label: &str) {
    ring_frame(ctx, c, s, vec![]).drop_without_applying_deltas();
    let out = ring_frame(ctx, c, s, vec![]);
    let pos = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.job.text == label => {
                let p = t.pos + t.galley.size() * 0.5;
                shape.clip_rect.contains(p).then_some(p)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing visible gear control {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        ring_frame(
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
#[test]
fn painted_internal_pair_roles_feed_native_preview_cancel_apply_history_and_saved_reopen() {
    for ring_first in [false, true] {
        let mut s = Session::new(Document::new("Painted internal pair").unwrap()).unwrap();
        let before = s.document().clone();
        let mut c = controller();
        let request = model::GearRequest::new(
            "Pair",
            model::GearParameters {
                module: model::length(1.),
                face_width: model::length(2.),
                ..Default::default()
            },
        );
        let panel = GearPanel::new(request, s.parameters().clone(), false).unwrap();
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Gear(Box::new(panel)),
        )
        .unwrap();
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        if ring_first {
            ring_click(&ctx, &mut c, &mut s, "Spur");
            ring_click(&ctx, &mut c, &mut s, "Internal ring");
        }
        ring_click(&ctx, &mut c, &mut s, "Create mating pair");
        if !ring_first {
            ring_click(&ctx, &mut c, &mut s, "Internal ring as second member");
        }
        let ModelForm::Gear(panel) = c.form.as_ref().unwrap() else {
            panic!()
        };
        let saved = panel.clone();
        let ModelRequest::GearPair(command) = panel.model_request(s.document(), true).unwrap()
        else {
            panic!()
        };
        let resolved = command.resolved(s.document(), s.parameters()).unwrap();
        assert_eq!(resolved.center_distance_mm, 12.);
        assert_eq!(
            resolved.gears[usize::from(!ring_first)].family(),
            model::GearFamily::InternalRing
        );
        let pair = command.pair();
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        c.cancel();
        assert_eq!(s.document(), &before);
        c.begin_form(&s, &Default::default(), Channel::Ui, ModelForm::Gear(saved))
            .unwrap();
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        let output = c.apply(&mut s, &Default::default()).unwrap();
        assert_eq!(output.len(), 2);
        let committed = s.document().clone();
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &committed);
        let mut bytes = vec![];
        multicad_core::project_file::write(
            &multicad_core::project_file::ProjectFile::new(committed.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let opened = multicad_core::project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = Session::new(opened.project.document).unwrap();
        assert_eq!(reopened.document(), &committed);
        c.begin_existing(&reopened, &Default::default(), pair, Channel::Ui)
            .unwrap();
        assert_eq!(c.label(), "Gear pair");
        c.cancel();
    }
}
#[test]
fn native_gear_preview_cancel_apply_history_and_historical_edit_use_the_real_gate() {
    let mut s = Session::new(Document::new("Native gear UI").unwrap()).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    for cancel in [true, false] {
        c.begin_gear(&s, &Default::default(), Channel::Ui).unwrap();
        assert_eq!(c.label(), "Gear");
        assert!(!c.can_apply(&s, &Default::default()));
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        if cancel {
            c.cancel();
        } else {
            let output = c.apply(&mut s, &Default::default()).unwrap();
            let feature = output[0].feature;
            assert_eq!(
                model::read_gear(s.document(), feature).unwrap().output.body,
                output[0].body
            );
            let committed = s.document().clone();
            s.undo(1, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &before);
            s.redo(2, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &committed);
            assert!(ModelToolController::editable(s.document(), feature));
            c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
                .unwrap();
            c.cancel();
            assert_eq!(s.document(), &committed);
        }
    }
}
#[test]
fn native_unresolvable_width_never_enables_apply_or_changes_the_document() {
    let mut s = Session::new(Document::new("Invalid native gear").unwrap()).unwrap();
    let request = model::GearRequest::new(
        "Too thin",
        model::GearParameters {
            face_width: model::length(0.00000001),
            ..Default::default()
        },
    );
    let panel = GearPanel::new(request, s.parameters().clone(), false).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::Gear(Box::new(panel)),
    )
    .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(!c.can_apply(&s, &Default::default()));
    assert!(c.error().is_some());
    assert!(c.apply(&mut s, &Default::default()).is_err());
    c.cancel();
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), 0);
}

#[test]
fn native_pair_preview_requires_both_placed_members_and_preserves_atomic_history() {
    let mut s = Session::new(Document::new("Native pair UI").unwrap()).unwrap();
    let before = s.document().clone();
    let request = model::GearRequest::new(
        "Pair",
        model::GearParameters {
            face_width: model::length(4.),
            ..Default::default()
        },
    );
    let mut panel = GearPanel::new(request, s.parameters().clone(), false).unwrap();
    panel.enable_pair();
    let ModelRequest::GearPair(command) = panel.model_request(s.document(), true).unwrap() else {
        panic!("pair conversion must use the pair transaction");
    };
    let pair = command.pair();
    let root = command.occurrence().unwrap();
    let mut c = controller();
    for cancel in [true, false] {
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Gear(Box::new(panel.clone())),
        )
        .unwrap();
        assert_eq!(c.label(), "Gear pair");
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        let candidate = c.candidate.as_ref().unwrap();
        let scene = candidate.scene.as_ref().unwrap();
        let source = candidate.prepared.document();
        let leaves = scene.assembly_evaluation().unwrap();
        assert_eq!(
            leaves.leaves.iter().filter(|l| l.path.root == root).count(),
            2
        );
        let aliases = candidate
            .outputs
            .iter()
            .map(|o| (o.body, o.feature))
            .collect();
        let missing_placement = Scene::new(
            scene.stamp(),
            scene.geometries().values().cloned().collect(),
            scene.reference_instances().to_vec(),
        )
        .unwrap()
        .with_reference_aliases(aliases)
        .unwrap()
        .with_model_evaluation(scene.model_evaluation().unwrap().clone())
        .unwrap();
        assert!(
            candidate
                .proof
                .validate(source, &missing_placement, &candidate.outputs)
                .unwrap_err()
                .contains("physical occurrence evidence")
        );
        let duplicated = [candidate.outputs[0].clone(), candidate.outputs[0].clone()];
        assert!(
            candidate
                .proof
                .validate(source, scene, &duplicated)
                .unwrap_err()
                .contains("member identities")
        );
        if cancel {
            c.cancel();
        } else {
            let output = c.apply(&mut s, &Default::default()).unwrap();
            assert_eq!(output.len(), 2);
            let committed = s.document().clone();
            let definition = model::read_gear_pair(s.document(), pair).unwrap();
            for selected in [
                root,
                pair,
                definition.outputs.assembly,
                definition.outputs.instances[0],
                definition.outputs.joints[1],
                output[0].body,
                output[1].feature,
            ] {
                assert_eq!(
                    ModelToolController::gear_pair_for(s.document(), selected),
                    Some(pair)
                );
            }
            s.undo(1, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &before);
            s.redo(2, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &committed);
            c.begin_existing(&s, &Default::default(), pair, Channel::Ui)
                .unwrap();
            assert_eq!(c.label(), "Gear pair");
            c.cancel();
            assert_eq!(s.document(), &committed);
        }
    }
}

fn painted_pair_drive(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    value: &str,
) {
    let output = ring_frame(ctx, c, s, vec![]);
    let label_y = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Pinion drive angle (deg)" => {
                Some(text.pos.y)
            }
            _ => None,
        })
        .expect("painted pinion drive label");
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            (node.role() == egui::accesskit::Role::TextInput)
                .then(|| node.bounds())
                .flatten()
        })
        .filter(|bounds| bounds.y0 > f64::from(label_y))
        .min_by(|a, b| a.y0.total_cmp(&b.y0))
        .expect("drive input follows its label");
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    );
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        ring_frame(
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
    ring_frame(
        ctx,
        c,
        s,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text(value.into()),
        ],
    )
    .drop_without_applying_deltas();
}

#[test]
fn painted_rack_roles_use_native_preview_cancel_apply_edit_history_and_independent_reopen() {
    for rack_first in [true, false] {
        let mut s = Session::new(Document::new("Painted rack pair").unwrap()).unwrap();
        let before = s.document().clone();
        let mut c = controller();
        let panel = GearPanel::new(
            model::GearRequest::new(
                "Rack pair",
                model::GearParameters {
                    module: model::length(1.),
                    face_width: model::length(2.),
                    ..Default::default()
                },
            ),
            s.parameters().clone(),
            false,
        )
        .unwrap();
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Gear(Box::new(panel)),
        )
        .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        ring_click(&ctx, &mut c, &mut s, "Spur");
        ring_click(&ctx, &mut c, &mut s, model::GearFamily::Rack.label());
        let ModelForm::Gear(panel) = c.form.as_ref().unwrap() else {
            panic!("gear")
        };
        let single = panel.request(s.document(), true).unwrap();
        assert_eq!(single.parameters.family, model::GearFamily::Rack);
        assert!(single.parameters.rack.is_some());
        assert_eq!(single.parameters.bore_diameter, model::length(0.));
        assert_eq!(s.document(), &before);
        ring_click(&ctx, &mut c, &mut s, "Create mating pair");
        let ModelForm::Gear(panel) = c.form.as_ref().unwrap() else {
            panic!("pair")
        };
        let ModelRequest::GearPair(original) = panel.model_request(s.document(), true).unwrap()
        else {
            panic!("pair request")
        };
        if !rack_first {
            ring_click(&ctx, &mut c, &mut s, "Pinion A / Rack B");
        }
        let ModelForm::Gear(panel) = c.form.as_ref().unwrap() else {
            panic!("pair")
        };
        let saved = panel.clone();
        let ModelRequest::GearPair(command) = panel.model_request(s.document(), true).unwrap()
        else {
            panic!("pair request")
        };
        assert_eq!(original.pair(), command.pair());
        let resolved = command.resolved(s.document(), s.parameters()).unwrap();
        let rack = usize::from(!rack_first);
        assert_eq!(resolved.gears[rack].family(), model::GearFamily::Rack);
        assert_eq!(resolved.gears[1 - rack].family(), model::GearFamily::Spur);
        let travel = resolved.rack_travel.as_ref().unwrap();
        assert_eq!(travel.rack_member.index(), rack);
        assert!(travel.min_drive_angle_rad < 0. && travel.max_drive_angle_rad > 0.);
        assert_eq!(s.document(), &before);
        ring_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        ring_click(&ctx, &mut c, &mut s, "Cancel");
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
        c.begin_form(&s, &Default::default(), Channel::Ui, ModelForm::Gear(saved))
            .unwrap();
        ring_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        ring_click(&ctx, &mut c, &mut s, "Apply");
        assert_eq!(s.revision(), 1);
        let pair = command.pair();
        let source = model::read_gear_pair(s.document(), pair).unwrap();
        let committed = s.document().clone();
        s.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(2, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &committed);
        let mut bytes = vec![];
        multicad_core::project_file::write(
            &multicad_core::project_file::ProjectFile::new(committed.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let opened = multicad_core::project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap();
        let mut reopened = Session::new(opened.project.document).unwrap();
        assert_eq!(reopened.document(), &committed);
        c.begin_existing(&reopened, &Default::default(), pair, Channel::Ui)
            .unwrap();
        painted_pair_drive(&ctx, &mut c, &mut reopened, "5");
        assert_eq!(reopened.revision(), 0);
        assert_eq!(reopened.document(), &committed);
        ring_click(&ctx, &mut c, &mut reopened, "Preview");
        wait(&mut c, &reopened);
        assert!(
            c.can_apply(&reopened, &Default::default()),
            "{:?}",
            c.error()
        );
        ring_click(&ctx, &mut c, &mut reopened, "Apply");
        assert_eq!(reopened.revision(), 1);
        let edited = model::read_gear_pair(reopened.document(), pair).unwrap();
        assert_eq!(edited.outputs, source.outputs);
        assert_eq!(edited.parameters.gears, source.parameters.gears);
        let resolved = model::resolve_gear_pair_parameters(
            reopened.document(),
            reopened.parameters(),
            &edited.parameters,
        )
        .unwrap();
        let radius = resolved.gears[1 - rack].rotary().unwrap().pitch_radius_mm;
        assert!(
            (resolved.rack_travel.as_ref().unwrap().translation_mm - radius * 5_f64.to_radians())
                .abs()
                < 1e-10
        );
        c.begin_existing(&reopened, &Default::default(), pair, Channel::Ui)
            .unwrap();
        painted_pair_drive(&ctx, &mut c, &mut reopened, "1000");
        let edited_document = reopened.document().clone();
        assert!(
            c.preview(&reopened, &Default::default(), Channel::Ui)
                .is_err()
        );
        assert!(!c.can_apply(&reopened, &Default::default()));
        assert!(c.apply(&mut reopened, &Default::default()).is_err());
        c.cancel();
        assert_eq!(reopened.document(), &edited_document);
        assert_eq!(reopened.revision(), 1);
    }
}
