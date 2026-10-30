//! Save-time preview capture. Save first asks the window for a screenshot of
//! the visible 3D view, stores it as the file preview without an Undo entry
//! (`Session::replace_file_preview`) and then submits the ordinary save, so
//! the saved file, home screen and library show it and the tab ends clean.
//! Without a visible 3D view (Parameters, Drawings, Export, headless) the
//! previous preview is kept. A capture failure never blocks the save.
use super::*;
use crate::{preview_capture, storage::SaveMode};
use multicad_core::project_file::info;

/// Identifies this application's screenshot requests.
#[derive(Debug)]
struct CaptureTag(EntityId);
pub(super) struct PendingCapture {
    tag: EntityId,
    local: EntityId,
    path: PathBuf,
    mode: SaveMode,
    rect: egui::Rect,
    requested: bool,
    passes: u32,
}
const MAX_CAPTURE_PASSES: u32 = 120;

impl Application {
    fn capture_enabled(&self) -> bool {
        let p = self.current();
        p.editor.info_locked.is_none()
            && info::read(p.session.document()).is_ok_and(|i| i.capture_on_save)
    }
    /// Start a save, capturing the preview first when that is possible.
    pub(super) fn begin_save(&mut self, path: PathBuf, mode: SaveMode) -> Result<(), String> {
        self.library.capture_note = None;
        if !self.capture_enabled() {
            return self.submit_save(path, mode);
        }
        let viewport = matches!(self.mode, Mode::Sketch | Mode::Model)
            .then_some(self.library.viewport)
            .flatten();
        let Some(rect) = viewport else {
            self.library.capture_note =
                Some("preview unchanged (no 3D view visible to capture)".into());
            return self.submit_save(path, mode);
        };
        self.library.capture = Some(PendingCapture {
            tag: EntityId::new(),
            local: self.current().local_id,
            path,
            mode,
            rect,
            requested: false,
            passes: 0,
        });
        self.message = "Capturing the 3D view as the file preview…".into();
        self.message_error = false;
        Ok(())
    }
    /// Per frame: remember the drawn viewport and drive a pending capture.
    pub(super) fn drive_capture(&mut self, ctx: &egui::Context) {
        self.library.viewport = preview_capture::visible_viewport(ctx);
        let Some(capture) = &mut self.library.capture else {
            return;
        };
        let tag = capture.tag;
        let shot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot {
                    user_data, image, ..
                } if user_data
                    .data
                    .as_ref()
                    .and_then(|d| d.downcast_ref::<CaptureTag>())
                    .is_some_and(|t| t.0 == tag) =>
                {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        if let Some(image) = shot {
            let rect = capture.rect;
            let result = preview_capture::crop(&image, rect, ctx.pixels_per_point());
            self.finish_capture(result);
            return;
        }
        if !capture.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                CaptureTag(tag),
            )));
            capture.requested = true;
        }
        capture.passes += 1;
        if capture.passes > MAX_CAPTURE_PASSES {
            self.finish_capture(Err("the window did not deliver a screenshot".into()));
            return;
        }
        ctx.request_repaint();
    }
    /// Store the captured view (or report why not) and submit the save.
    pub(super) fn finish_capture(&mut self, result: Result<egui::ColorImage, String>) {
        let Some(capture) = self.library.capture.take() else {
            return;
        };
        if self.current().local_id != capture.local {
            self.message =
                "Save cancelled: the tab changed while capturing its preview. Save again.".into();
            self.message_error = true;
            return;
        }
        let stored = result
            .and_then(|image| preview_capture::preview_from(&image))
            .and_then(|preview| {
                let p = self.current_mut();
                p.session
                    .replace_file_preview(Some(preview), Actor::local(Channel::Ui))
                    .map_err(|e| e.to_string())?;
                if !p.editor.dirty() {
                    p.editor.reload(&p.session);
                }
                Ok(())
            });
        self.library.capture_note = match stored {
            Ok(()) => Some("preview updated from the 3D view".into()),
            Err(error) => Some(format!("preview not updated ({error})")),
        };
        if let Err(error) = self.submit_save(capture.path, capture.mode) {
            self.message = error;
            self.message_error = true;
        }
    }
    pub(super) fn capture_pending(&self) -> bool {
        self.library.capture.is_some()
    }
}
