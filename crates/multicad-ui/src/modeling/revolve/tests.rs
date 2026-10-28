use super::*;
use multicad_core::{
    document::{Entity, EntityData, EntityId},
    model, parameters,
    sketch_regions::{BoundaryIntent, RegionIntent},
};
use std::collections::BTreeSet;
fn panel() -> RevolvePanel {
    let region = RegionIntent {
        outer: BoundaryIntent {
            curves: BTreeSet::from([EntityId::new()]),
        },
        holes: BTreeSet::new(),
    };
    RevolvePanel::new(RevolveRequest::new(
        EntityId::new(),
        vec![region],
        Default::default(),
    ))
    .unwrap()
}
#[test]
fn axis_and_signed_sweep_use_formula_dimensions_in_sketch_local_coordinates() {
    let mut document = Document::new("Local revolution axis").unwrap();
    document
        .insert(Entity::new(
            "shift",
            EntityData::Parameter {
                expression: "12".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let before = document.clone();
    let mut panel = panel();
    let ids = panel.request(&document, true).unwrap().outputs;
    panel.origin[0].text = "=$shift + 3".into();
    panel.origin[1].text = "20".into();
    panel.bearing.text = "0".into();
    panel.sweep.text = "-180".into();
    let request = panel.request(&document, true).unwrap();
    assert_eq!(request.outputs, ids);
    let resolved = request
        .parameters
        .resolve(&parameters::evaluate(&document).unwrap())
        .unwrap();
    assert_eq!(resolved.axis_origin_mm, [15., 20.]);
    assert_eq!(resolved.axis_direction, [1., 0.]);
    assert!((resolved.angle_rad + std::f64::consts::PI).abs() < 1e-12);
    assert_eq!(document, before);
}
#[test]
fn revolve_uses_new_form_stamp_for_same_frame_preview_and_rejects_stale_apply() {
    let document = Document::new("Revolve events").unwrap();
    let mut panel = panel();
    let old = panel.stamp();
    panel.sweep.text = "270".into();
    let Some(ModelEvent::Preview {
        stamp,
        request: ModelRequest::Revolve(request),
    }) = panel.finish_frame(
        true,
        Some(FormAction::Preview),
        &document,
        true,
        PreviewState::Ready { stamp: old },
    )
    else {
        panic!()
    };
    assert_ne!(stamp, old);
    assert_eq!(request.parameters.angle, model::angle(270.));
    assert!(
        panel
            .finish_frame(
                false,
                Some(FormAction::Apply),
                &document,
                true,
                PreviewState::Ready { stamp: old }
            )
            .is_none()
    );
    assert!(matches!(
        panel.finish_frame(
            false,
            Some(FormAction::Apply),
            &document,
            true,
            PreviewState::Ready { stamp }
        ),
        Some(ModelEvent::Apply)
    ));
    assert!(matches!(
        panel.finish_frame(
            true,
            Some(FormAction::Cancel),
            &document,
            true,
            PreviewState::Ready { stamp }
        ),
        Some(ModelEvent::Cancel)
    ));
    panel.sweep.text = "NaN".into();
    assert!(panel.request(&document, true).is_err());
    assert!(panel.request(&document, false).is_err());
}
#[test]
fn revolve_inspector_stays_narrow_without_creating_canonical_entities() {
    let document = Document::new("Narrow revolve form").unwrap();
    let before = document.clone();
    let mut panel = panel();
    let context = egui::Context::default();
    let request = panel.request(&document, true).unwrap();
    for _ in 0..90 {
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1000.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let inspector = egui::Panel::right("revolve-inspector")
                        .default_size(180.)
                        .min_size(0.)
                        .resizable(true)
                        .show(ui, |ui| {
                            assert!(
                                panel
                                    .show(ui, &document, true, PreviewState::Idle)
                                    .is_none()
                            );
                        });
                    assert!(inspector.response.rect.width() <= 181.);
                },
            )
            .drop_without_applying_deltas();
    }
    assert_eq!(
        panel.request(&document, true).unwrap().outputs,
        request.outputs
    );
    assert_eq!(document, before);
}
