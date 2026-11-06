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
    let end = Instant::now() + Duration::from_secs(90);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < end, "native hardware preview timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn wood_hardware_opens_the_shared_metric_screw_recipe_source_neutrally() {
    let session = Session::new(Document::new("Wood hardware").unwrap()).unwrap();
    let before = session.document().clone();
    let mut controller = controller();
    controller
        .begin_wood_preset(
            &session,
            &Default::default(),
            vec![],
            Channel::Ui,
            crate::wood_navigation::Preset::Hardware,
        )
        .unwrap();
    let ModelRequest::MechanicalPart(request) = controller
        .form
        .as_ref()
        .unwrap()
        .request(session.document(), controller.capabilities())
        .unwrap()
    else {
        panic!("Shared hardware recipe")
    };
    assert_eq!(request.parameters, model::default_machine_screw());
    assert!(matches!(
        request.parameters,
        model::MechanicalPartParameters::MachineScrew { .. }
    ));
    assert_eq!(session.document(), &before);
    controller.cancel();
    assert_eq!(session.document(), &before);
}
#[test]
fn all_three_hardware_forms_use_real_native_preview_cancel_apply_history_and_reopen() {
    let recipes = [
        model::MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.5),
            frame: Default::default(),
        },
        model::MechanicalPartParameters::HexNut {
            across_flats: length(5.5),
            thread: Box::new(model::ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(2.4),
                ..Default::default()
            }),
        },
        model::MechanicalPartParameters::BearingEnvelope {
            bore_diameter: length(10.),
            outside_diameter: length(30.),
            width: length(9.),
            frame: Default::default(),
        },
    ];
    for parameters in recipes {
        let mut s = Session::new(Document::new("Native hardware UI").unwrap()).unwrap();
        let before = s.document().clone();
        let r = model::MechanicalPartRequest::new("Hardware", parameters.clone());
        let mut c = controller();
        for cancel in [true, false] {
            let panel = MechanicalPartPanel::new(r.clone(), s.parameters().clone(), false).unwrap();
            c.begin_form(
                &s,
                &Default::default(),
                Channel::Ui,
                ModelForm::MechanicalPart(Box::new(panel)),
            )
            .unwrap();
            assert_eq!(c.label(), "Hardware");
            assert!(!c.can_apply(&s, &Default::default()));
            c.preview(&s, &Default::default(), Channel::Ui).unwrap();
            wait(&mut c, &s);
            assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
            assert_eq!(s.document(), &before);
            let candidate = c.candidate.as_ref().unwrap();
            assert!(candidate.requires_info);
            assert!(
                candidate
                    .scene
                    .as_ref()
                    .unwrap()
                    .model_evaluation()
                    .is_some()
            );
            if cancel {
                c.cancel();
                assert_eq!(s.document(), &before);
            } else {
                let outputs = c.apply(&mut s, &Default::default()).unwrap();
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].feature, r.feature);
                assert_eq!(outputs[0].body, r.body);
                let source = model::read_mechanical_part(s.document(), r.feature).unwrap();
                assert_eq!(source.parameters, parameters);
                let saved = s.document().clone();
                s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
                assert_eq!(s.document(), &before);
                s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
                assert_eq!(s.document(), &saved);
                assert!(ModelToolController::editable(s.document(), r.feature));
                c.begin_existing(&s, &Default::default(), r.feature, Channel::Ui)
                    .unwrap();
                assert_eq!(c.label(), "Feature edit");
                c.preview(&s, &Default::default(), Channel::Ui).unwrap();
                wait(&mut c, &s);
                assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
                c.cancel();
                assert_eq!(s.document(), &saved);
            }
        }
    }
}
#[test]
fn below_native_resolution_hardware_never_enables_apply_or_mutates_source() {
    let mut s = Session::new(Document::new("Invalid hardware UI").unwrap()).unwrap();
    let before = s.document().clone();
    let r = model::MechanicalPartRequest::new(
        "Too thin",
        model::MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(1e-10),
            frame: Default::default(),
        },
    );
    let panel = MechanicalPartPanel::new(r, s.parameters().clone(), false).unwrap();
    let mut c = controller();
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::MechanicalPart(Box::new(panel)),
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

fn screw_frame(
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
                egui::vec2(460., 2600.),
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
fn screw_click(ctx: &egui::Context, c: &mut ModelToolController, s: &mut Session, label: &str) {
    screw_frame(ctx, c, s, vec![]).drop_without_applying_deltas();
    let out = screw_frame(ctx, c, s, vec![]);
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
        .unwrap_or_else(|| panic!("Missing screw control {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        screw_frame(
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
fn painted_machine_screw_heads_share_native_preview_cancel_apply_history_and_saved_reopen() {
    for external_hex in [false, true] {
        let mut s = Session::new(Document::new("Native machine screw UI").unwrap()).unwrap();
        let before = s.document().clone();
        let mut parameters = model::default_machine_screw();
        let model::MechanicalPartParameters::MachineScrew {
            underhead_length,
            head,
            drive,
            thread,
            ..
        } = &mut parameters
        else {
            unreachable!()
        };
        // Deliberately partial: opening the form must retain both lengths.
        *underhead_length = Some(length(6.));
        thread.length = length(3.);
        if external_hex {
            *head = model::ScrewHead::ExternalHex {
                across_flats: length(5.5),
                height: length(2.),
            };
            *drive = model::ScrewDrive::ExternalHex;
        }
        let request = model::MechanicalPartRequest::new("Machine screw", parameters.clone());
        let mut c = controller();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for cancel in [true, false] {
            c.begin_form(
                &s,
                &Default::default(),
                Channel::Ui,
                ModelForm::MechanicalPart(Box::new(
                    MechanicalPartPanel::new(request.clone(), s.parameters().clone(), false)
                        .unwrap(),
                )),
            )
            .unwrap();
            assert!(!c.can_apply(&s, &Default::default()));
            let output = screw_frame(&ctx, &mut c, &mut s, vec![]);
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains("Underhead length") && text.contains("Thread length"));
            assert!(text.contains(if external_hex {
                "Head across flats"
            } else {
                "Socket depth"
            }));
            output.drop_without_applying_deltas();
            screw_click(&ctx, &mut c, &mut s, "Preview");
            wait(&mut c, &s);
            assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
            assert_eq!(s.document(), &before);
            if cancel {
                screw_click(&ctx, &mut c, &mut s, "Cancel");
                assert!(!c.active());
                assert_eq!(s.document(), &before);
            } else {
                screw_click(&ctx, &mut c, &mut s, "Apply");
                assert_eq!(s.revision(), 1);
                assert_eq!(
                    model::read_mechanical_part(s.document(), request.feature)
                        .unwrap()
                        .parameters,
                    parameters
                );
            }
        }
        let saved = s.document().clone();
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &saved);
        let project = multicad_core::project_file::ProjectFile::new(saved.clone()).unwrap();
        let mut bytes = vec![];
        multicad_core::project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let reopened = multicad_core::project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let mut s = Session::new(reopened.document).unwrap();
        c.begin_existing(&s, &Default::default(), request.feature, Channel::Ui)
            .unwrap();
        screw_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &saved);
        screw_click(&ctx, &mut c, &mut s, "Cancel");
        assert_eq!(s.document(), &saved);
        c.begin_existing(&s, &Default::default(), request.feature, Channel::Ui)
            .unwrap();
        let old = crate::modeling::dimension::DimensionField::from_dimension(&length(6.), "mm")
            .unwrap()
            .text;
        screw_replace_value(&ctx, &mut c, &mut s, &old, "7 mm");
        assert!(!c.can_apply(&s, &Default::default()));
        assert_eq!(s.document(), &saved);
        screw_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        screw_click(&ctx, &mut c, &mut s, "Apply");
        let mut edited_parameters = parameters.clone();
        let model::MechanicalPartParameters::MachineScrew {
            underhead_length, ..
        } = &mut edited_parameters
        else {
            unreachable!()
        };
        *underhead_length = Some(length(7.));
        assert_eq!(
            model::read_mechanical_part(s.document(), request.feature)
                .unwrap()
                .parameters,
            edited_parameters
        );
        let edited = s.document().clone();
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &saved);
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &edited);
        let project = multicad_core::project_file::ProjectFile::new(edited.clone()).unwrap();
        let mut bytes = vec![];
        multicad_core::project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let reopened = multicad_core::project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(reopened.document, edited);
    }
}

#[test]
fn invalid_machine_screw_lengths_socket_walls_and_depths_keep_source_unchanged() {
    for case in 0..4 {
        let mut s = Session::new(Document::new("Invalid screw UI").unwrap()).unwrap();
        let before = s.document().clone();
        let mut parameters = model::default_machine_screw();
        let model::MechanicalPartParameters::MachineScrew {
            underhead_length,
            drive,
            thread,
            ..
        } = &mut parameters
        else {
            unreachable!()
        };
        match case {
            0 => *underhead_length = Some(length(0.)),
            1 => thread.length = length(11.),
            2 => {
                *drive = model::ScrewDrive::HexSocket {
                    across_flats: length(5.5),
                    depth: length(1.3),
                }
            }
            _ => {
                *drive = model::ScrewDrive::HexSocket {
                    across_flats: length(2.55),
                    depth: length(3.),
                }
            }
        }
        let request = model::MechanicalPartRequest::new("Invalid screw", parameters);
        let panel = MechanicalPartPanel::new(request, s.parameters().clone(), false).unwrap();
        assert!(panel.request(s.document(), true).is_err());
        let mut c = controller();
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::MechanicalPart(Box::new(panel)),
        )
        .unwrap();
        assert!(!c.can_apply(&s, &Default::default()));
        assert!(c.preview(&s, &Default::default(), Channel::Ui).is_err());
        assert!(c.apply(&mut s, &Default::default()).is_err());
        c.cancel();
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
}

fn screw_replace_value(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    old: &str,
    new: &str,
) {
    let out = screw_frame(ctx, c, s, vec![]);
    let pos = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|(_, n)| n.value() == Some(old))
        .filter_map(|(_, n)| n.bounds())
        .find(|r| r.x1 - r.x0 > 50.)
        .map(|b| egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32))
        .expect("dimension input");
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        screw_frame(
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
    screw_frame(
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
            egui::Event::Text(new.into()),
        ],
    )
    .drop_without_applying_deltas();
}

#[test]
fn painted_new_screw_selection_and_head_drive_choices_use_the_canonical_default() {
    let mut s = Session::new(Document::new("Screw head choices").unwrap()).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    let request = model::MechanicalPartRequest::new(
        "Hardware",
        model::MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.5),
            frame: Default::default(),
        },
    );
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::MechanicalPart(Box::new(
            MechanicalPartPanel::new(request, s.parameters().clone(), false).unwrap(),
        )),
    )
    .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    screw_click(&ctx, &mut c, &mut s, "Plain washer");
    screw_click(&ctx, &mut c, &mut s, "Machine screw");
    fn read(c: &ModelToolController, s: &Session) -> model::MechanicalPartParameters {
        match c.form.as_ref().unwrap() {
            ModelForm::MechanicalPart(panel) => {
                panel.request(s.document(), true).unwrap().parameters
            }
            _ => panic!("Expected hardware inspector"),
        }
    }
    assert_eq!(read(&c, &s), model::default_machine_screw());
    screw_click(&ctx, &mut c, &mut s, "Socket cap");
    screw_click(&ctx, &mut c, &mut s, "External hex head");
    if let ModelForm::MechanicalPart(panel) = c.form.as_ref().unwrap() {
        assert!(
            panel.request(s.document(), true).is_err(),
            "Head selection must not silently convert the drive"
        );
    }
    screw_click(&ctx, &mut c, &mut s, "Hex socket");
    screw_click(&ctx, &mut c, &mut s, "External hex flats");
    let model::MechanicalPartParameters::MachineScrew { head, drive, .. } = read(&c, &s) else {
        unreachable!()
    };
    assert!(matches!(head, model::ScrewHead::ExternalHex { .. }));
    assert_eq!(drive, model::ScrewDrive::ExternalHex);
    screw_click(&ctx, &mut c, &mut s, "External hex head");
    screw_click(&ctx, &mut c, &mut s, "Socket cap");
    screw_click(&ctx, &mut c, &mut s, "External hex flats");
    screw_click(&ctx, &mut c, &mut s, "Hex socket");
    assert_eq!(read(&c, &s), model::default_machine_screw());
    c.cancel();
    assert_eq!(s.document(), &before);
}

#[test]
fn painted_phillips_pan_retains_overall_expression_through_native_preview_cancel_apply_and_reopen()
{
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};
    let mut s = Session::new(Document::new("Configured Phillips pan").unwrap()).unwrap();
    let before = s.document().clone();
    let angle = |value| Dimension::Literal {
        value,
        unit: "deg".into(),
    };
    let mut parameters = model::default_machine_screw();
    let model::MechanicalPartParameters::MachineScrew {
        underhead_length,
        length_input,
        head,
        drive,
        thread,
        ..
    } = &mut parameters
    else {
        unreachable!()
    };
    *underhead_length = None;
    *length_input = Some(model::ScrewLengthInput::Overall {
        value: Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=14", "mm", None).unwrap(),
        },
    });
    *head = model::ScrewHead::Pan {
        profile: Box::new(model::CrownedScrewHeadParameters {
            diameter: length(5.6),
            height: length(2.4),
            crown_radius: length(5.),
            top_land_diameter: length(4.),
            neck_radius: length(0.1),
        }),
    };
    *drive = model::ScrewDrive::PhillipsH {
        profile: Box::new(model::PhillipsHParameters {
            size: model::PhillipsHSize::H1,
            mouth_diameter: length(3.1),
            mouth_radius: length(0.5),
            b_width: length(0.96),
            g_diameter: length(1.27),
            tilt_angle: angle(7.),
            outer_flank_angle: angle(26.5),
            floor_angle: angle(28.),
            outer_corner_angle: angle(92.),
            inner: model::PhillipsInnerForm::ShallowV {
                angle: angle(138.),
                e_width: length(0.435),
            },
        }),
    };
    thread.representation = model::ThreadRepresentation::Modeled;
    let request = model::MechanicalPartRequest::new("Configured H1 pan", parameters.clone());
    let mut c = controller();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for cancel in [true, false] {
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::MechanicalPart(Box::new(
                MechanicalPartPanel::new(request.clone(), s.parameters().clone(), false).unwrap(),
            )),
        )
        .unwrap();
        let output = screw_frame(&ctx, &mut c, &mut s, vec![]);
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("Pan head")
                && text.contains("Phillips H recess")
                && text.contains("Outer corner included angle (deg)")
        );
        output.drop_without_applying_deltas();
        screw_click(&ctx, &mut c, &mut s, "Preview");
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        screw_click(
            &ctx,
            &mut c,
            &mut s,
            if cancel { "Cancel" } else { "Apply" },
        );
        if cancel {
            assert_eq!(s.revision(), 0);
            assert_eq!(s.document(), &before);
        } else {
            assert_eq!(s.revision(), 1);
            assert_eq!(
                model::read_mechanical_part(s.document(), request.feature)
                    .unwrap()
                    .parameters,
                parameters
            );
        }
    }
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
    screw_click(&ctx, &mut c, &mut independent, "Preview");
    wait(&mut c, &independent);
    assert!(
        c.can_apply(&independent, &Default::default()),
        "{:?}",
        c.error()
    );
    screw_click(&ctx, &mut c, &mut independent, "Cancel");
    assert_eq!(independent.document(), &saved);
}
