//! Standard sketch tools on the native developed chart, independent of the
//! folded assembly scene used to validate the eventual cut.
use super::*;
use multicad_core::model::{self, CutoutFace, DevelopedSheetSketch};
use std::sync::Arc;

pub(super) struct DevelopedViewport {
    pub(super) pattern: Arc<crate::sheet_pattern::Pattern>,
    folded_camera: Camera,
}

pub(super) fn chart(document: &Document, sketch: EntityId) -> Option<DevelopedSheetSketch> {
    model::developed_sheet_sketch(document, sketch)
        .ok()
        .flatten()
}

impl SketchEditor {
    pub fn begin_developed_sketch(
        &mut self,
        session: &mut Session,
        part: EntityId,
        face: CutoutFace,
        channel: Channel,
    ) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before creating a developed sketch".into());
        }
        let count = session
            .document()
            .entities()
            .filter(|entity| matches!(entity.data, EntityData::Sketch { .. }))
            .count();
        let created = sketch::prepare_new(
            session,
            session.revision(),
            Actor::local(channel),
            &format!("Developed sketch {}", count + 1),
            sketch::CreationPlane::DevelopedSheet { part, face },
            Default::default(),
        )
        .map_err(|error| error.to_string())?;
        session
            .commit(created.prepared)
            .map_err(|error| error.to_string())?;
        self.activate_sketch(created.sketch)?;
        self.occurrence = None;
        self.creation_evaluation = None;
        self.message =
            "Developed sketch · draw a closed profile inside one flat area, clear of bends".into();
        Ok(())
    }

    pub(crate) fn developed_sketch_part(&self, document: &Document) -> Option<EntityId> {
        chart(document, self.selected?).map(|chart| chart.part)
    }

    pub fn set_developed_pattern(&mut self, pattern: Option<Arc<crate::sheet_pattern::Pattern>>) {
        match (pattern, &mut self.developed_viewport) {
            (Some(pattern), Some(view)) => view.pattern = pattern,
            (Some(pattern), None) => {
                self.developed_viewport = Some(DevelopedViewport {
                    pattern,
                    folded_camera: self.camera,
                });
                self.fit_requested = true;
                self.solid.clear();
                self.cancel_move();
                self.moving = false;
            }
            (None, view) => {
                if let Some(view) = view.take() {
                    self.camera = view.folded_camera;
                    self.fit_requested = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        modeling::native_fixture::{NativeBackend, scene},
        sheet_pattern::{Chart, Pattern},
    };
    use multicad_core::{
        project_file,
        sketch_draw::{DrawInput, DrawRequest, PointInput},
    };
    use std::time::{Duration, Instant};

    #[test]
    fn finished_developed_circle_picks_on_blank_and_uses_exact_through_sheet_extrude() {
        let mut bytes = &include_bytes!("../../../../examples/front-panel.tcd")[..];
        let file = project_file::read(
            &mut bytes,
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let mut session = Session::new(file.document).unwrap();
        let part = session
            .document()
            .entities()
            .find_map(|entity| {
                model::sheet_feature(session.document(), entity.id).map(|_| entity.id)
            })
            .unwrap();
        let compiler = multicad_model::Compiler::default();
        let native = compiler
            .compile(session.document(), session.revision())
            .unwrap();
        let flat = native.unfold(session.document(), part).unwrap();
        let before = flat.cut.area_mm2();
        let mut editor = SketchEditor::default();
        let folded_camera = editor.camera;
        editor.set_query_factory(Arc::new(|| {
            Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
        }));
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.set_developed_pattern(Some(Arc::new(Pattern {
            name: flat.name,
            cut: flat.cut,
            size_mm: flat.size_mm,
            thickness_mm: flat.thickness_mm,
            bends: vec![],
            drills: vec![],
            chart: Chart {
                base: flat.chart.base,
                legs: flat
                    .chart
                    .legs
                    .into_iter()
                    .map(|leg| (leg.index, leg.quad))
                    .collect(),
                bend_zones: flat.chart.bend_zones,
            },
        })));
        editor
            .begin_developed_sketch(&mut session, part, CutoutFace::Base, Channel::Ui)
            .unwrap();
        let sketch = editor.selected.unwrap();
        super::super::tests::settle(&mut editor, &session);
        editor
            .draw(
                &mut session,
                &DrawRequest {
                    input: DrawInput::Circle {
                        center: PointInput::Position([25., 25.]),
                        radius_mm: 3.,
                    },
                    construction: false,
                },
                Channel::Ui,
            )
            .unwrap();
        super::super::tests::settle(&mut editor, &session);
        editor.finish().unwrap();

        let native = compiler
            .compile(session.document(), session.revision())
            .unwrap();
        let canonical = scene(session.document(), native.snapshot(), 1).unwrap();
        editor.display_scene = Some(canonical.clone());
        let ctx = egui::Context::default();
        let run = |editor: &mut SketchEditor, session: &mut Session, events| {
            ctx.run_ui(super::super::tests::input(events), |ui| {
                editor.canvas(ui, session, false, Some(canonical.clone()), true)
            })
            .drop_without_applying_deltas();
        };
        run(&mut editor, &mut session, vec![]);
        let viewport = editor.viewport_rect.unwrap();
        let [x, y, _] = editor.camera.project([25., 25., 0.], viewport);
        let pos = egui::pos2(x, y);
        assert!(viewport.contains(pos));
        assert!(editor.camera.project([25., 26., 0.], viewport)[1] < y);
        run(
            &mut editor,
            &mut session,
            vec![egui::Event::PointerMoved(pos)],
        );
        for pressed in [true, false] {
            run(
                &mut editor,
                &mut session,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let wait = Instant::now() + Duration::from_secs(5);
        while editor.pending_profile_pick.is_some() {
            editor.sketch_evaluation.poll();
            editor.poll_profile_pick(session.document(), session.revision());
            assert!(Instant::now() < wait, "Exact profile click timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        let (owner, selected) = editor
            .selected_profiles(session.document(), session.revision())
            .unwrap();
        assert_eq!(owner, sketch);
        assert_eq!(selected.len(), 1);
        assert!(editor.solid.selected_id().is_none());
        editor
            .begin_extrude(&session, &file.resources, Channel::Ui)
            .unwrap();
        assert!(!editor.model.operation_gizmo_active());
        editor
            .preview_model(&session, &file.resources, Channel::Ui)
            .unwrap();
        let wait = Instant::now() + Duration::from_secs(15);
        while !editor.model.can_apply(&session, &file.resources) {
            editor.poll_model(&session, &file.resources);
            assert!(editor.model.error().is_none(), "{:?}", editor.model.error());
            assert!(Instant::now() < wait, "Native cut preview timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        editor.apply_model(&mut session, &file.resources).unwrap();
        let native = compiler
            .compile(session.document(), session.revision())
            .unwrap();
        let after = native
            .unfold(session.document(), part)
            .unwrap()
            .cut
            .area_mm2();
        assert!((before - after - 9. * std::f64::consts::PI).abs() < 1e-5);
        editor.set_developed_pattern(None);
        assert_eq!(editor.camera, folded_camera);
    }
}
