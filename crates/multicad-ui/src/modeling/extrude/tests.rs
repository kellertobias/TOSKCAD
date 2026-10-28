use super::*;
use crate::egui::{Pos2, Rect, Vec2};
use multicad_core::{
    document::{Entity, EntityData},
    parameters::{self, BoundExpression, Unit},
    sketch_regions::BoundaryIntent,
};
use std::collections::BTreeSet;

fn region() -> RegionIntent {
    RegionIntent {
        outer: BoundaryIntent {
            curves: BTreeSet::from([EntityId::new()]),
        },
        holes: BTreeSet::new(),
    }
}
fn panel() -> ExtrudePanel {
    let mut panel = ExtrudePanel::default();
    panel
        .begin(EntityId::new(), vec![region(), region()])
        .unwrap();
    panel
}
fn document() -> Document {
    Document::new("Source-neutral extrusion form").unwrap()
}

#[test]
fn profiles_have_independent_stable_outputs_across_edits_and_requests() {
    let document = document();
    let original_document = document.clone();
    let mut panel = panel();
    let original = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert_ne!(original.outputs[0].body, original.outputs[1].body);
    assert_ne!(original.outputs[0].feature, original.outputs[1].feature);
    assert_eq!(original.parameters, ExtrudeParameters::default());
    for index in 1..=50 {
        panel.draft.as_mut().unwrap().forward.distance.text = index.to_string();
        assert!(matches!(
            panel.finish_frame(
                true,
                None,
                &document,
                ExtrudeCapabilities::distance_bodies(),
                PreviewState::Idle
            ),
            Some(ExtrudeEvent::Changed(_))
        ));
        let next = panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap();
        assert_eq!(next.outputs, original.outputs);
        assert_eq!(next.sketch, original.sketch);
    }
    assert_eq!(document, original_document);
}

#[test]
fn edit_and_preview_in_one_frame_uses_new_stamp_and_current_values() {
    let document = document();
    let mut panel = panel();
    let old = panel.stamp().unwrap();
    panel.draft.as_mut().unwrap().forward.distance.text = "37".into();
    let event = panel.finish_frame(
        true,
        Some(Action::Preview),
        &document,
        ExtrudeCapabilities::distance_bodies(),
        PreviewState::Ready { stamp: old },
    );
    let Some(ExtrudeEvent::Preview { stamp, request }) = event else {
        panic!("Preview should accept the edited value in the same frame")
    };
    assert_eq!(stamp.tool, old.tool);
    assert_eq!(stamp.revision, old.revision + 1);
    assert_eq!(
        request.parameters.extent,
        Extent::OneSided {
            end: Termination::Distance {
                distance: millimeters(37.)
            }
        }
    );
    assert!(
        panel
            .finish_frame(
                false,
                Some(Action::Apply),
                &document,
                ExtrudeCapabilities::distance_bodies(),
                PreviewState::Ready { stamp: old }
            )
            .is_none()
    );
    assert!(
        matches!(panel.finish_frame(false,Some(Action::Apply),&document,ExtrudeCapabilities::distance_bodies(),PreviewState::Ready{stamp}),Some(ExtrudeEvent::Apply(s)) if s==stamp)
    );
}

#[test]
fn stale_apply_cannot_win_over_an_edit_and_cancel_wins_over_every_action() {
    let document = document();
    let mut panel = panel();
    let old = panel.stamp().unwrap();
    panel.draft.as_mut().unwrap().reverse = true;
    assert!(
        matches!(panel.finish_frame(true,Some(Action::Apply),&document,ExtrudeCapabilities::distance_bodies(),PreviewState::Ready{stamp:old}),Some(ExtrudeEvent::Changed(stamp)) if stamp!=old)
    );
    for state in [
        PreviewState::Idle,
        PreviewState::Pending {
            stamp: panel.stamp().unwrap(),
        },
        PreviewState::Failed {
            stamp: panel.stamp().unwrap(),
            message: "Native solid failed",
        },
    ] {
        assert!(
            panel
                .finish_frame(
                    false,
                    Some(Action::Apply),
                    &document,
                    ExtrudeCapabilities::distance_bodies(),
                    state
                )
                .is_none()
        );
    }
    assert!(matches!(
        panel.finish_frame(
            true,
            Some(Action::Cancel),
            &document,
            ExtrudeCapabilities::distance_bodies(),
            PreviewState::Ready { stamp: old }
        ),
        Some(ExtrudeEvent::Cancel)
    ));
    assert!(!panel.active());
    panel.begin(EntityId::new(), vec![region()]).unwrap();
    assert_ne!(panel.stamp().unwrap().tool, old.tool);
}

#[test]
fn offset_direction_reverse_and_extent_remain_separate_canonical_controls() {
    let document = document();
    let parameters = parameters::evaluate(&document).unwrap();
    let mut panel = panel();
    let draft = panel.draft.as_mut().unwrap();
    draft.elevation.text = "45".into();
    draft.azimuth.text = "90".into();
    draft.reverse = true;
    draft.offset.text = "10".into();
    draft.forward.distance.text = "100".into();
    draft.backward.distance.text = "25".into();
    draft.extent = ExtentChoice::TwoSided;
    let request = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert_eq!(request.parameters.taper, degrees(0.));
    let resolved = request.parameters.resolve(&parameters).unwrap();
    assert_eq!(resolved.start_offset_mm, 10.);
    assert_eq!((resolved.forward_mm, resolved.backward_mm), (100., 25.));
    assert!(resolved.direction[0].abs() < 1e-12);
    assert!((resolved.direction[1] + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    assert!((resolved.direction[2] + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    let draft = panel.draft.as_mut().unwrap();
    draft.extent = ExtentChoice::Symmetric;
    draft.total.text = "80".into();
    let request = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert_eq!(
        request.parameters.extent,
        Extent::Symmetric {
            total: millimeters(80.)
        }
    );
    let resolved = request.parameters.resolve(&parameters).unwrap();
    assert_eq!((resolved.forward_mm, resolved.backward_mm), (40., 40.));
}

#[test]
fn unsupported_requests_remain_unchanged_and_cannot_be_presented_as_new_body() {
    let document = document();
    let mut panel = panel();
    let original = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    for operation in [
        Operation::Join,
        Operation::Cut,
        Operation::Intersect,
        Operation::Split,
    ] {
        let mut request = original.clone();
        request.operation = operation;
        let unchanged = request.clone();
        assert!(
            panel
                .begin_request(request.clone())
                .unwrap_err()
                .contains("new-body")
        );
        assert_eq!(request, unchanged);
        assert_eq!(
            panel
                .request(&document, ExtrudeCapabilities::distance_bodies())
                .unwrap(),
            original
        );
    }
    let mut invalid_unit = original;
    invalid_unit.parameters.start_offset = degrees(10.);
    assert!(
        panel
            .begin_request(invalid_unit)
            .unwrap_err()
            .contains("units")
    );
}

#[test]
fn unavailable_capabilities_and_invalid_input_prevent_apply_and_report_errors() {
    let document = document();
    let mut panel = panel();
    let stamp = panel.stamp().unwrap();
    assert!(
        panel
            .request(&document, ExtrudeCapabilities::default())
            .is_err()
    );
    assert!(
        panel
            .finish_frame(
                false,
                Some(Action::Apply),
                &document,
                ExtrudeCapabilities::default(),
                PreviewState::Ready { stamp }
            )
            .is_none()
    );
    for text in ["NaN", "inf", "-inf", "1e30", "not a number", "=$missing"] {
        panel.draft.as_mut().unwrap().forward.distance.text = text.into();
        assert!(
            panel
                .finish_frame(
                    true,
                    Some(Action::Preview),
                    &document,
                    ExtrudeCapabilities::distance_bodies(),
                    PreviewState::Idle
                )
                .is_none()
        );
        assert!(panel.error.is_some());
    }
    panel.draft.as_mut().unwrap().forward.distance.text = "10".into();
    panel.draft.as_mut().unwrap().taper.text = "3".into();
    assert!(
        panel
            .request(
                &document,
                ExtrudeCapabilities {
                    taper: false,
                    ..ExtrudeCapabilities::distance_bodies()
                }
            )
            .unwrap_err()
            .contains("Taper")
    );
}

#[test]
fn reopening_a_bound_expression_preserves_identity_after_parameter_rename() {
    let mut document = document();
    let mut parameter = Entity::new(
        "height",
        EntityData::Parameter {
            expression: "20".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document.insert(parameter.clone()).unwrap();
    let expression = BoundExpression::bind(&document, "=$height * 2", "mm", None).unwrap();
    let mut request = panel()
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    request.parameters.extent = Extent::Symmetric {
        total: Dimension::Expression { expression },
    };
    let outputs = request.outputs.clone();
    let mut panel = ExtrudePanel::default();
    panel.begin_request(request).unwrap();
    parameter.name = "renamed_height".into();
    document.replace(parameter.clone()).unwrap();
    let request = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert_eq!(request.outputs, outputs);
    let Extent::Symmetric { total } = request.parameters.extent else {
        panic!()
    };
    assert_eq!(total.expression().unwrap().bindings[0].target, parameter.id);
    assert_eq!(
        total
            .value_in(
                Unit::Millimeter,
                Some(&parameters::evaluate(&document).unwrap())
            )
            .unwrap(),
        40.
    );
}

#[test]
fn reopened_formula_keeps_its_units_and_implicit_constants() {
    let mut document = document();
    let parameter = Entity::new(
        "height",
        EntityData::Parameter {
            expression: "20".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document.insert(parameter).unwrap();
    let expression = BoundExpression::bind(&document, "=$height + 2", "cm", None).unwrap();
    let mut request = panel()
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    request.parameters.extent = Extent::Symmetric {
        total: Dimension::Expression {
            expression: expression.clone(),
        },
    };
    let mut panel = ExtrudePanel::default();
    panel.begin_request(request).unwrap();
    let request = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    let Extent::Symmetric { total } = request.parameters.extent else {
        panic!()
    };
    assert_eq!(total.expression(), Some(&expression));
    assert_eq!(
        total
            .value_in(
                Unit::Millimeter,
                Some(&parameters::evaluate(&document).unwrap())
            )
            .unwrap(),
        40.
    );
}

#[test]
fn many_frames_fit_a_narrow_inspector_without_allocating_output_identities() {
    let document = document();
    let original_document = document.clone();
    let mut panel = panel();
    let original = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    for width in [180., 250.] {
        let context = egui::Context::default();
        for frame in 0..120 {
            panel.draft.as_mut().unwrap().extent = match frame % 3 {
                0 => ExtentChoice::OneSided,
                1 => ExtentChoice::TwoSided,
                _ => ExtentChoice::Symmetric,
            };
            context
                .run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 1000.))),
                        ..Default::default()
                    },
                    |ui| {
                        let inspector = egui::Panel::right("test-extrude-inspector")
                            .default_size(width)
                            .min_size(0.)
                            .resizable(true)
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    assert!(
                                        panel
                                            .show(
                                                ui,
                                                &document,
                                                ExtrudeCapabilities::distance_bodies(),
                                                PreviewState::Ready {
                                                    stamp: panel.stamp().unwrap()
                                                }
                                            )
                                            .is_none()
                                    );
                                });
                            });
                        assert!(
                            inspector.response.rect.width() <= width + 1.,
                            "Inspector grew from {width} to {}",
                            inspector.response.rect.width()
                        );
                        assert!(ui.available_width() >= 900.);
                    },
                )
                .drop_without_applying_deltas();
            assert_eq!(
                panel
                    .request(&document, ExtrudeCapabilities::distance_bodies())
                    .unwrap()
                    .outputs,
                original.outputs
            );
        }
    }
    assert_eq!(document, original_document);
}

#[test]
fn empty_selection_is_rejected_before_allocating_an_active_tool() {
    let mut panel = ExtrudePanel::default();
    assert!(panel.begin(EntityId::new(), vec![]).is_err());
    assert!(!panel.active());
}

#[test]
fn associative_plane_controls_reopen_without_replacing_the_reference_or_formula() {
    let mut document = document();
    let plane = Entity::new(
        "Target plane",
        EntityData::Plane {
            origin_mm: [0., 0., 30.],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    document.insert(plane.clone()).unwrap();
    let mut p = Entity::new(
        "clearance",
        EntityData::Parameter {
            expression: "5".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document.insert(p.clone()).unwrap();
    let offset = Dimension::Expression {
        expression: BoundExpression::bind(&document, "=$clearance + 2", "mm", None).unwrap(),
    };
    let mut request = panel()
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    request.parameters.extent = Extent::TwoSided {
        forward: Termination::ToPlane {
            plane: plane.id,
            offset,
        },
        backward: Termination::Distance {
            distance: millimeters(7.),
        },
    };
    let original = request.clone();
    let mut panel = ExtrudePanel::default();
    panel.begin_request(request).unwrap();
    p.name = "renamed_clearance".into();
    document.replace(p).unwrap();
    assert_eq!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap(),
        original
    );
    assert!(
        panel
            .request(
                &document,
                ExtrudeCapabilities {
                    to_plane: false,
                    ..ExtrudeCapabilities::distance_bodies()
                }
            )
            .is_err()
    );
    panel.draft.as_mut().unwrap().taper.text = "2".into();
    assert!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap_err()
            .contains("Nonzero taper")
    );
}
#[test]
fn face_links_remain_sole_preserved_selectors_and_shared_ends_are_deduplicated() {
    use multicad_core::{
        document::Definition,
        model::FaceInput,
        references::{Attachment, GeometryIntent, Origin, TopologyKind},
    };
    let mut document = document();
    let producer = Entity::new(
        "Named source",
        EntityData::Feature {
            definition: Definition {
                type_id: "test.face-source".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    document.insert(producer.clone()).unwrap();
    let make = |role: &str| {
        let origin = Origin {
            source: producer.id,
            role: role.into(),
        };
        FaceInput {
            source: producer.id,
            intent: GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([origin.clone()]),
                boundary: None,
                attachment: Some(Attachment {
                    anchor: origin,
                    offset_mm: [2., 3., 0.],
                    angle_rad: 0.5,
                }),
            },
        }
    };
    let a = make("cap.start");
    let b = make("cap.end");
    let mut request = panel()
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    request.parameters.extent = Extent::TwoSided {
        forward: Termination::ToFace {
            reference: a.intent.id,
            offset: millimeters(2.),
        },
        backward: Termination::ToFace {
            reference: b.intent.id,
            offset: millimeters(-3.),
        },
    };
    request.faces = vec![b.clone(), a.clone()];
    let original = request.clone();
    let mut panel = ExtrudePanel::default();
    panel.begin_request(request.clone()).unwrap();
    assert_eq!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap(),
        original
    );
    request.parameters.extent = Extent::TwoSided {
        forward: Termination::ToFace {
            reference: a.intent.id,
            offset: millimeters(2.),
        },
        backward: Termination::ToFace {
            reference: a.intent.id,
            offset: millimeters(-3.),
        },
    };
    request.faces = vec![a.clone()];
    panel.begin_request(request.clone()).unwrap();
    assert_eq!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap(),
        request
    );
    let mut missing = request.clone();
    missing.faces.clear();
    assert!(panel.begin_request(missing).is_err());
    assert_eq!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap(),
        request
    );
}
#[test]
fn external_operation_changes_share_form_stamp_and_picking_blocks_old_ready_apply() {
    let document = document();
    let mut panel = panel();
    let old = panel.stamp().unwrap();
    let outputs = panel
        .request(&document, ExtrudeCapabilities::distance_bodies())
        .unwrap()
        .outputs;
    panel.changed();
    let stamp = panel.stamp().unwrap();
    assert_eq!(stamp.tool, old.tool);
    assert_eq!(stamp.revision, old.revision + 1);
    assert_eq!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap()
            .outputs,
        outputs
    );
    assert!(
        panel
            .finish_frame(
                false,
                Some(Action::Apply),
                &document,
                ExtrudeCapabilities::distance_bodies(),
                PreviewState::Ready { stamp: old }
            )
            .is_none()
    );
    panel
        .draft
        .as_mut()
        .unwrap()
        .picker
        .begin(EndSide::Forward, super::super::termination::EndKind::Face);
    assert!(
        panel
            .request(&document, ExtrudeCapabilities::distance_bodies())
            .unwrap_err()
            .contains("selection")
    );
    assert!(
        panel
            .finish_frame(
                false,
                Some(Action::Apply),
                &document,
                ExtrudeCapabilities::distance_bodies(),
                PreviewState::Ready { stamp }
            )
            .is_none()
    );
    panel.cancel();
    assert!(!panel.termination_picking());
}
