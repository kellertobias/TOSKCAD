//! Debounced automatic previews for the auxiliary Model inspectors.
use crate::egui;

#[derive(Default)]
pub(crate) struct Schedule {
    automatic: bool,
    draft: Option<(u64, Option<f64>)>,
}

impl Schedule {
    pub(crate) fn automatic(&self) -> bool {
        self.automatic
    }

    pub(crate) fn reset(&mut self) {
        self.draft = None;
    }

    pub(crate) fn attempted(&mut self, version: u64) {
        self.draft = Some((version, None));
    }

    pub(crate) fn ready(
        &mut self,
        ui: &egui::Ui,
        automatic: bool,
        version: Option<u64>,
        blocked: bool,
    ) -> bool {
        self.automatic = automatic;
        let Some(version) = version.filter(|_| automatic && !blocked) else {
            self.reset();
            return false;
        };
        let now = ui.input(|input| input.time);
        if self.draft.is_none_or(|(old, _)| old != version) {
            self.draft = Some((version, Some(now + 0.15)));
        }
        let Some((_, Some(deadline))) = self.draft else {
            return false;
        };
        let remaining = deadline - now;
        if remaining > 0. {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
            return false;
        }
        self.attempted(version);
        true
    }
}
