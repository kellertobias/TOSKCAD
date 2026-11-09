//! Route Sheet Metal tools and keep the developed view local to each project.
use super::*;
use crate::sheet_pattern::{BackendFactory, Tool};

impl Application {
    pub fn set_sheet_pattern_backends(&mut self, factory: BackendFactory) {
        self.sheet_pattern_backends = Some(factory);
    }

    pub(super) fn sheet_metal_tab(&self) -> bool {
        self.mode == Mode::Model
            && self
                .registry
                .workspace(Mode::Model)
                .tabs
                .get(self.tab)
                .is_some_and(|tab| tab.title == crate::registry::METAL_TITLE)
    }

    pub(super) fn sheet_action_ready(&self, command: Command) -> Result<(), String> {
        let project = self.current();
        if self.mode != Mode::Model {
            return Err("Open Model to use Sheet Metal tools".into());
        }
        if command == Command::SheetPattern(Tool::Fold) {
            return project.sheet_pattern.active().then_some(()).ok_or_else(|| {
                "Unfold a sheet to switch between the flat pattern and the folded model".into()
            });
        }
        if project.pending_edits()
            || project.sketch.model_active()
            || project.sketch.measurement.active()
        {
            return Err("Finish the active tool before opening a Sheet Metal tool".into());
        }
        let document = project.session.document();
        let part = project
            .sketch
            .selected_sheet_part(&project.session)
            .ok_or("Select one sheet-metal part in the model tree or viewport")?;
        match command {
            Command::SheetFlange | Command::SheetFasteners => {
                if !project.sketch.modeling_available() {
                    return Err("Finish the active Model tool first".into());
                }
                if command == Command::SheetFlange {
                    crate::modeling::ModelToolController::sheet_flange_availability(
                        document, part,
                    )?;
                }
            }
            Command::SheetPattern(_) if self.sheet_pattern_backends.is_none() => {
                return Err("Native flat-pattern evaluation is unavailable".into());
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn invoke_sheet_action(
        &mut self,
        command: Command,
        channel: Channel,
    ) -> Result<(), String> {
        self.sheet_action_ready(command)?;
        if command == Command::SheetPattern(Tool::Fold) {
            self.current_mut().sheet_pattern.close();
            self.message = "Folded model".into();
            return Ok(());
        }
        let tab = self.workbench_tab(Mode::Model, command);
        let factory = self.sheet_pattern_backends.clone();
        let p = self.current_mut();
        let document = p.session.document();
        let part = p
            .sketch
            .selected_sheet_part(&p.session)
            .expect("selection checked");
        match command {
            Command::SheetFlange => {
                p.sketch
                    .begin_sheet_flange(&p.session, &p.resources, part, channel)?;
                p.sheet_pattern.close();
                self.active_tool = Some((self.active, command));
                self.message = "Add flange · select a sheet edge in the viewport".into();
            }
            Command::SheetFasteners => {
                p.sketch
                    .begin_sheet_fasteners(&p.session, &p.resources, part, channel)?;
                p.sheet_pattern.close();
                self.active_tool = Some((self.active, command));
                self.message = "Attached fasteners · place a metric screw on the sheet".into();
            }
            Command::SheetPattern(tool) => {
                p.sheet_pattern.open(
                    &factory.expect("backend checked"),
                    document,
                    p.session.revision(),
                    part,
                    tool == Tool::BendSchedule,
                )?;
            }
            _ => unreachable!("sheet command routing"),
        }
        self.tab = tab;
        Ok(())
    }
}
