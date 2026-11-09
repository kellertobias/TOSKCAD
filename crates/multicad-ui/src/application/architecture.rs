use super::*;
use crate::{architecture_navigation::Category, modeling::ModelingTool};
impl Application {
    pub(super) fn active_architecture_category(&self) -> Option<Category> {
        (self.mode == Mode::Model
            && self
                .registry
                .workspace(self.mode)
                .tabs
                .get(self.tab)
                .is_some_and(|tab| tab.title == "Architecture"))
        .then_some(self.current().architecture_category)
        .flatten()
    }
    pub(super) fn architecture_category_tabs(
        &self,
        ui: &mut egui::Ui,
        category: Category,
        colors: TabColors,
    ) -> Option<Command> {
        let mut action =
            controls::tab_with_icon(ui, Some(Icon::Back), "Architecture", false, colors)
                .on_hover_text("Return to Architecture tools and the full Model tab row")
                .clicked()
                .then_some(Command::ArchitectureParent);
        for entry in Category::ALL {
            let selected = entry == category;
            if controls::tab_with_icon(ui, Some(entry.icon()), entry.label(), selected, colors)
                .clicked()
                && !selected
            {
                action = Some(Command::ArchitectureCategory(entry));
            }
        }
        action
    }

    pub(super) fn cycle_architecture_category(&mut self, category: Category, forward: bool) {
        let command = category
            .adjacent(forward)
            .map_or(Command::ArchitectureParent, Command::ArchitectureCategory);
        self.execute(command, &[], Channel::Ui);
    }

    pub(super) fn navigate_architecture(
        &mut self,
        category: Option<Category>,
    ) -> Result<(), String> {
        if self.welcome || self.mode != Mode::Model {
            return Err("Open Model before browsing Architecture tools".into());
        }
        let tab = self
            .registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .position(|tab| tab.title == "Architecture")
            .ok_or("Architecture is unavailable")?;
        self.tab = tab;
        self.current_mut().architecture_category = category;
        self.cancel_pending_view_restore();
        let label = category.map_or("Architecture", Category::label);
        self.message = if self.current().sketch.model_interaction_active() {
            format!("{label} tools · the current operation remains open in the Inspector")
        } else {
            format!("{label} tools · choose a tool to begin")
        };
        Ok(())
    }
    pub(super) fn route_architecture_tool(
        &mut self,
        tool: ModelingTool,
        preset: Option<multicad_core::equipment::Kind>,
    ) {
        let category = if let Some(kind) = preset {
            Some(
                self.current()
                    .architecture_category
                    .filter(|category| category.contains(kind))
                    .unwrap_or_else(|| Category::for_equipment(kind)),
            )
        } else {
            match tool {
                ModelingTool::Cabinet | ModelingTool::Panel | ModelingTool::WoodBox => {
                    if self.registry.workspace(Mode::Model).tabs[self.tab].title != "Architecture" {
                        return;
                    }
                    Some(Category::Furniture)
                }
                ModelingTool::Equipment => Some(Category::Electrical),
                ModelingTool::Plant | ModelingTool::Fence => Some(Category::Landscaping),
                ModelingTool::Wall
                | ModelingTool::Door
                | ModelingTool::Window
                | ModelingTool::Cutout
                | ModelingTool::Level
                | ModelingTool::Stair => None,
                _ => return,
            }
        };
        if let Some(tab) = self
            .registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .position(|tab| tab.title == "Architecture")
        {
            self.tab = tab;
            self.current_mut().architecture_category = category;
        }
    }
}
