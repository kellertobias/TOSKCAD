//! Route Sheet Metal tools and keep the developed view local to each project.
use super::*;
use crate::sheet_pattern::BackendFactory;

impl Application {
    pub fn set_sheet_pattern_backends(&mut self, factory: BackendFactory) {
        self.sheet_pattern_backends = Some(factory);
    }

    pub(super) fn sheet_action_ready(&self, command: Command) -> Result<(), String> {
        let project = self.current();
        if self.mode != Mode::Model {
            return Err("Open Model to use Sheet Metal tools".into());
        }
        if command == Command::SheetView && project.sheet_pattern.active() {
            return Ok(());
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
            Command::SheetFlange | Command::SheetCutout | Command::SheetFasteners => {
                if !project.sketch.modeling_available() {
                    return Err("Finish the active Model tool first".into());
                }
                match command {
                    Command::SheetFlange => {
                        crate::modeling::ModelToolController::sheet_flange_availability(
                            document, part,
                        )?
                    }
                    Command::SheetCutout => {
                        crate::modeling::ModelToolController::sheet_cutout_availability(
                            document, part,
                        )?
                    }
                    _ => {}
                }
            }
            Command::SheetView => {
                if self.sheet_pattern_backends.is_none() {
                    return Err("Native flat-pattern evaluation is unavailable".into());
                }
                crate::sheet_pattern::current_bends(&project.scene, &project.session, part)?;
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
        if command == Command::SheetView && self.current().sheet_pattern.active() {
            self.current_mut().sheet_pattern.close();
            self.message = "Folded model".into();
            return Ok(());
        }
        let tab = self.workbench_tab(Mode::Model, command);
        let factory = self.sheet_pattern_backends.clone();
        let p = self.current_mut();
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
            Command::SheetCutout => {
                p.sketch
                    .begin_sheet_cutout(&p.session, &p.resources, part, channel)?;
                p.sheet_pattern.close();
                self.active_tool = Some((self.active, command));
                self.message = "Add cutout · pick a planar face on the folded sheet".into();
            }
            Command::SheetFasteners => {
                p.sketch
                    .begin_sheet_fasteners(&p.session, &p.resources, part, channel)?;
                p.sheet_pattern.close();
                self.active_tool = Some((self.active, command));
                self.message = "Attached fasteners · place a metric screw on the sheet".into();
            }
            Command::SheetView => {
                p.sheet_pattern.open(
                    &factory.expect("backend checked"),
                    p.session.document(),
                    p.session.revision(),
                    part,
                )?;
                self.message = "Flat pattern".into();
                return Ok(());
            }
            _ => unreachable!("sheet command routing"),
        }
        self.tab = tab;
        Ok(())
    }
}
