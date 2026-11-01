//! Application Settings window (CAD-187): default length unit and workbench
//! visibility/order. Settings are user-local preferences, never project data.
use super::*;
use crate::preferences::{
    ALWAYS_VISIBLE, LengthUnit, PreferenceStore, PreferenceWorker, Preferences,
};

impl Application {
    /// Load persisted preferences now (a small bounded file) and write later
    /// changes through a background worker. A load failure keeps defaults
    /// and reports the reason in Settings.
    pub fn set_preference_store(&mut self, mut store: Box<dyn PreferenceStore>) {
        match store.load() {
            Ok(Some(mut preferences)) => match preferences.validate() {
                Ok(()) => {
                    preferences.normalize(&self.registry);
                    self.install_preferences(preferences);
                }
                Err(error) => self.preference_error = Some(error),
            },
            Ok(None) => {}
            Err(error) => self.preference_error = Some(error),
        }
        match PreferenceWorker::new(store) {
            Ok(worker) => self.preference_worker = Some(worker),
            Err(error) => self.preference_error = Some(error),
        }
    }
    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }
    /// Apply a changed preference set to navigation and display and persist it.
    pub fn apply_preferences(&mut self, mut preferences: Preferences) {
        preferences.normalize(&self.registry);
        if preferences == self.preferences {
            return;
        }
        self.install_preferences(preferences);
        if let Some(worker) = &self.preference_worker
            && let Err(error) = worker.save(&self.preferences)
        {
            self.preference_error = Some(error);
        }
    }
    fn install_preferences(&mut self, preferences: Preferences) {
        let hidden_active = !preferences.is_visible(&self.registry, self.mode, self.tab)
            && self
                .preferences
                .is_visible(&self.registry, self.mode, self.tab);
        self.preferences = preferences;
        crate::preferences::set_length_unit(self.preferences.length_unit);
        crate::preferences::set_title_logo_default(self.preferences.title_logo);
        for project in &mut self.projects {
            project
                .editor
                .adopt_length_unit(self.preferences.length_unit);
        }
        if hidden_active {
            let title = self.registry.workspace(self.mode).tabs[self.tab].title;
            self.tab = self.first_visible_tab(self.mode);
            self.message = format!(
                "{title} hidden · showing {}",
                self.registry.workspace(self.mode).tabs[self.tab].title
            );
            self.message_error = false;
        }
    }
    /// First visible workbench of `mode` usable in the current context.
    pub(super) fn first_visible_tab(&self, mode: Mode) -> usize {
        let editing = self.current().sketch.editing;
        let tabs = &self.registry.workspace(mode).tabs;
        self.preferences
            .visible_tabs(&self.registry, mode)
            .into_iter()
            .find(|i| editing || !tabs[*i].requires_active_sketch)
            .unwrap_or(0)
    }
    /// Ribbon workbenches in display order. A hidden workbench that a tool
    /// routed to stays reachable while it is active (marked `true`).
    pub(super) fn ribbon_tabs(&self) -> Vec<(usize, bool)> {
        let mut tabs: Vec<(usize, bool)> = self
            .preferences
            .visible_tabs(&self.registry, self.mode)
            .into_iter()
            .map(|i| (i, false))
            .collect();
        if !tabs.iter().any(|(i, _)| *i == self.tab) {
            tabs.push((self.tab, true));
        }
        let editing = self.current().sketch.editing;
        let workspace = self.registry.workspace(self.mode);
        tabs.retain(|(i, _)| editing || !workspace.tabs[*i].requires_active_sketch);
        tabs
    }
    pub(super) fn poll_preferences(&mut self) {
        if let Some(result) = self.preference_worker.as_ref().and_then(|w| w.poll()) {
            self.preference_error = result.err();
        }
    }
    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut draft = self.preferences.clone();
        let mut error = None;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(true)
            .default_width(420.)
            .show(ctx, |ui| {
                ui.small("Application preferences for this user. They never change project data and apply to every open document.");
                if let Some(problem) = &self.preference_error {
                    ui.colored_label(Color32::LIGHT_RED, format!("Preferences: {problem}"));
                }
                ui.separator();
                ui.heading("Measurement units");
                ui.horizontal(|ui| {
                    for unit in LengthUnit::ALL {
                        ui.radio_value(&mut draft.length_unit, unit, unit.label())
                            .on_hover_text(format!("Show lengths and read unqualified length input in {}", unit.symbol()));
                    }
                });
                ui.small("Values typed with a unit, such as 25 mm or 1 in, keep that unit. Switching never changes model dimensions or stored formulas; parameters keep their own declared units, and drawing pages and export formats keep their explicit units.");
                ui.separator();
                ui.heading("Workbenches");
                ui.small(format!("Show, hide and reorder each mode's workbenches. {ALWAYS_VISIBLE} always stays visible; tools of hidden workbenches stay available from Find tool and the command line."));
                egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
                    for mode in Mode::ALL {
                        egui::CollapsingHeader::new(mode.title())
                            .default_open(mode == self.mode)
                            .show(ui, |ui| {
                            let order = draft.order(&self.registry, mode);
                            let last = order.len().saturating_sub(1);
                            for (position, (index, visible)) in order.into_iter().enumerate() {
                                let title = self.registry.workspace(mode).tabs[index].title;
                                ui.horizontal(|ui| {
                                    let mut shown = visible;
                                    let locked = title == ALWAYS_VISIBLE;
                                    let response = ui
                                        .add_enabled(!locked, egui::Checkbox::new(&mut shown, title))
                                        .on_hover_text(format!("Show {} › {title} in the ribbon", mode.title()))
                                        .on_disabled_hover_text(format!("{title} always stays visible"));
                                    if response.changed()
                                        && let Err(e) = draft.set_visible(mode, title, shown)
                                    {
                                        error = Some(e);
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let later = format!("Move {title} later in {}", mode.title());
                                        let earlier = format!("Move {title} earlier in {}", mode.title());
                                        let down = ui
                                            .add_enabled_ui(position < last, |ui| crate::icons::compact_button(ui, crate::icons::Icon::ArrowDown, &later))
                                            .inner;
                                        let up = ui
                                            .add_enabled_ui(position > 0, |ui| crate::icons::compact_button(ui, crate::icons::Icon::ArrowUp, &earlier))
                                            .inner;
                                        let delta = if up.clicked() { -1 } else if down.clicked() { 1 } else { 0 };
                                        if delta != 0
                                            && let Err(e) = draft.move_workbench(mode, title, delta)
                                        {
                                            error = Some(e);
                                        }
                                    });
                                });
                            }
                        });
                    }
                });
                ui.separator();
                ui.heading("Drawings");
                ui.checkbox(&mut draft.title_logo, "Show app logo in title block on new pages")
                    .on_hover_text("Default for new drawing pages. Each page keeps its own option in Page setup; existing pages are unchanged.");
                ui.separator();
                if ui
                    .button("Restore defaults")
                    .on_hover_text("Millimeters and every workbench visible in the standard order")
                    .clicked()
                {
                    draft = Preferences::defaults(&self.registry);
                }
            });
        if let Some(error) = error {
            self.message = error;
            self.message_error = true;
        }
        self.apply_preferences(draft);
        self.settings_open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            Ok(())
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    /// Shared in-memory stand-in for the user-local preference file.
    #[derive(Clone, Default)]
    struct Memory(Arc<Mutex<Option<Preferences>>>);
    impl PreferenceStore for Memory {
        fn load(&mut self) -> Result<Option<Preferences>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&mut self, preferences: &Preferences) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(preferences.clone());
            Ok(())
        }
    }
    struct Backend;
    impl crate::viewport::SceneBackend for Backend {
        fn request(&mut self, _: &Document, _: crate::viewport::SceneStamp) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
            None
        }
    }
    fn app(store: &Memory) -> Application {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        app.set_model_backends(std::sync::Arc::new(|| Box::new(Backend)));
        app.set_preference_store(Box::new(store.clone()));
        app
    }
    fn wait_saved(app: &mut Application) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = app.preference_worker.as_ref().unwrap().poll() {
                result.unwrap();
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "preference write timed out"
            );
            std::thread::yield_now();
        }
    }
    fn tab(app: &Application, mode: Mode, title: &str) -> usize {
        app.registry
            .workspace(mode)
            .tabs
            .iter()
            .position(|t| t.title == title)
            .unwrap()
    }
    fn titles(app: &Application) -> Vec<&'static str> {
        app.ribbon_tabs()
            .into_iter()
            .map(|(i, _)| app.registry.workspace(app.mode).tabs[i].title)
            .collect()
    }
    #[test]
    fn preferences_persist_across_restart_and_restore_to_defaults() {
        let store = Memory::default();
        let mut first = app(&store);
        let defaults = first.preferences().clone();
        let mut changed = defaults.clone();
        changed.length_unit = LengthUnit::Inch;
        changed.title_logo = false;
        changed.set_visible(Mode::Model, "Weld", false).unwrap();
        changed
            .set_visible(Mode::Model, "Mechanical", false)
            .unwrap();
        changed
            .move_workbench(Mode::Model, "Mechanical", -1)
            .unwrap();
        let import = changed.workbenches[&Mode::Model]
            .iter()
            .position(|entry| entry.title == "Import")
            .unwrap();
        changed
            .move_workbench(Mode::Model, "Import", -(import as isize))
            .unwrap();
        first.apply_preferences(changed.clone());
        wait_saved(&mut first);
        drop(first);
        // A restarted application reads the same preferences back.
        let mut second = app(&store);
        assert_eq!(second.preferences(), &changed);
        assert_eq!(crate::preferences::length_unit(), LengthUnit::Inch);
        assert!(!crate::preferences::title_logo_default());
        second.mode = Mode::Model;
        let model = titles(&second);
        assert_eq!(model[0], "Import");
        assert!(!model.contains(&"Weld"));
        assert!(!model.contains(&"Mechanical"));
        // Restore defaults is persisted the same way.
        second.apply_preferences(Preferences::defaults(&second.registry));
        wait_saved(&mut second);
        assert_eq!(store.0.lock().unwrap().as_ref(), Some(&defaults));
        assert_eq!(crate::preferences::length_unit(), LengthUnit::Millimeter);
        assert!(crate::preferences::title_logo_default());
        // The preference file is independent of the project document.
        assert_eq!(second.current().session.revision(), 0);
    }
    #[test]
    fn hiding_the_active_workbench_moves_to_a_visible_one_and_showing_restores_it() {
        let store = Memory::default();
        let mut app = app(&store);
        app.mode = Mode::Model;
        let wood = tab(&app, Mode::Model, "Wood");
        app.tab = wood;
        let mut hidden = app.preferences().clone();
        hidden.move_workbench(Mode::Model, "Common", 3).unwrap();
        hidden
            .set_visible(Mode::Model, "Architecture", false)
            .unwrap();
        hidden.set_visible(Mode::Model, "Wood", false).unwrap();
        app.apply_preferences(hidden);
        // The first visible workbench in the chosen order, not a hidden one.
        assert_eq!(app.tab, tab(&app, Mode::Model, "Sheet metal"));
        assert!(app.message.contains("Wood hidden"), "{}", app.message);
        assert!(!titles(&app).contains(&"Wood"));
        // Routing a Wood tool through command entry still works and shows
        // the hidden workbench while it is active.
        let panel = app.registry.tool("panel").unwrap().action;
        app.invoke(panel, &[], Channel::CommandLine).unwrap();
        assert_eq!(app.tab, wood);
        assert!(app.ribbon_tabs().contains(&(wood, true)));
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        // Showing it again restores it in the chosen position.
        let mut shown = app.preferences().clone();
        shown.set_visible(Mode::Model, "Wood", true).unwrap();
        app.apply_preferences(shown);
        assert!(app.ribbon_tabs().contains(&(wood, false)));
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn invalid_stored_preferences_keep_defaults_and_report_the_problem() {
        let store = Memory::default();
        let mut bad = Preferences::defaults(&Registry::default());
        bad.workbenches.get_mut(&Mode::Model).unwrap()[1].title = "\u{7}".into();
        *store.0.lock().unwrap() = Some(bad);
        let app = app(&store);
        assert_eq!(app.preferences(), &Preferences::defaults(&app.registry));
        assert!(app.preference_error.is_some());
    }
    #[test]
    fn settings_command_opens_the_settings_window_from_any_mode() {
        let store = Memory::default();
        let mut app = app(&store);
        for mode in Mode::ALL {
            app.settings_open = false;
            app.mode = mode;
            let settings = app.registry.resolve("settings").unwrap().action;
            assert!(app.enabled(settings));
            app.invoke(settings, &[], Channel::Ui).unwrap();
            assert!(app.settings_open, "{mode:?}");
        }
        assert_eq!(
            app.registry.search("inches")[0].action,
            Command::Extension("settings")
        );
    }
    /// Labels and bounds of the rendered accessibility tree.
    fn frame(
        app: &mut Application,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Pos2, bool)> {
        let output = ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360., 860.),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        let nodes = output
            .platform_output
            .accesskit_update
            .as_ref()
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let label = node.label()?.to_string();
                        let b = node.bounds()?;
                        Some((
                            label,
                            egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32),
                            node.is_disabled(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        output.drop_without_applying_deltas();
        nodes
    }
    fn click(app: &mut Application, ctx: &egui::Context, at: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    fn find(nodes: &[(String, egui::Pos2, bool)], label: &str) -> (egui::Pos2, bool) {
        nodes
            .iter()
            .find(|(l, ..)| l == label)
            .map(|(_, at, disabled)| (*at, *disabled))
            .unwrap_or_else(|| panic!("{label} not rendered"))
    }
    #[test]
    fn rendered_settings_offer_labeled_unit_and_workbench_controls() {
        let store = Memory::default();
        let mut app = app(&store);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.mode = Mode::Model;
        // The header Settings button opens the window.
        frame(&mut app, &ctx, vec![]);
        let nodes = frame(&mut app, &ctx, vec![]);
        let (gear, _) = find(&nodes, "Settings · measurement units and workbenches");
        click(&mut app, &ctx, gear);
        assert!(app.settings_open);
        frame(&mut app, &ctx, vec![]);
        let nodes = frame(&mut app, &ctx, vec![]);
        for label in [
            "Millimeters (mm)",
            "Inches (in)",
            "Wood",
            "Move Wood earlier in Model",
            "Move Wood later in Model",
            "Restore defaults",
        ] {
            find(&nodes, label);
        }
        // Common cannot be hidden and cannot move earlier than first.
        // (The ribbon also has an enabled Common tab; the Settings checkbox is disabled.)
        assert!(
            nodes
                .iter()
                .any(|(l, _, disabled)| l == "Common" && *disabled)
        );
        assert!(find(&nodes, "Move Common earlier in Model").1);
        let (inches, _) = find(&nodes, "Inches (in)");
        click(&mut app, &ctx, inches);
        assert_eq!(app.preferences().length_unit, LengthUnit::Inch);
        let nodes = frame(&mut app, &ctx, vec![]);
        let (wood, _) = find(&nodes, "Wood");
        click(&mut app, &ctx, wood);
        let nodes = frame(&mut app, &ctx, vec![]);
        let (earlier, _) = find(&nodes, "Move Import earlier in Model");
        click(&mut app, &ctx, earlier);
        app.mode = Mode::Model;
        let ribbon = titles(&app);
        assert!(!ribbon.contains(&"Wood"));
        let import = ribbon.iter().position(|t| *t == "Import").unwrap();
        assert_eq!(ribbon[import + 1], "Materials");
        let nodes = frame(&mut app, &ctx, vec![]);
        let (restore, _) = find(&nodes, "Restore defaults");
        click(&mut app, &ctx, restore);
        assert_eq!(app.preferences(), &Preferences::defaults(&app.registry));
        crate::preferences::set_length_unit(LengthUnit::Millimeter);
    }
    #[test]
    fn switching_units_changes_no_document_value_and_only_blank_rows_adopt_it() {
        let store = Memory::default();
        let mut app = app(&store);
        app.example().unwrap();
        let document = app.current().session.document().clone();
        let revision = app.current().session.revision();
        let values: Vec<_> = app
            .current()
            .session
            .parameters()
            .values
            .values()
            .map(|v| (v.quantity, v.unit))
            .collect();
        let mut inches = app.preferences().clone();
        inches.length_unit = LengthUnit::Inch;
        app.apply_preferences(inches);
        let p = app.current();
        assert_eq!(p.session.document(), &document);
        assert_eq!(p.session.revision(), revision);
        assert!(!p.editor.dirty(), "a unit switch is not a pending edit");
        let after: Vec<_> = p
            .session
            .parameters()
            .values
            .values()
            .map(|v| (v.quantity, v.unit))
            .collect();
        assert_eq!(after, values, "stored units and quantities are unchanged");
        let blank = p.editor.rows.last().unwrap();
        assert!(blank.name.is_empty());
        assert_eq!(blank.unit, "in");
        // Authored rows keep their declared units.
        assert!(
            p.editor.rows[..p.editor.rows.len() - 1]
                .iter()
                .all(|row| row.unit != "in")
        );
        crate::preferences::set_length_unit(LengthUnit::Millimeter);
    }
    #[test]
    fn title_logo_default_seeds_only_new_drawing_pages() {
        use multicad_core::drawings;
        let store = Memory::default();
        let mut app = app(&store);
        let new_page = app.registry.tool("drawing-new").unwrap().action;
        app.invoke(Command::Mode(Mode::Drawings), &[], Channel::Ui)
            .unwrap();
        app.invoke(new_page, &[], Channel::Ui).unwrap();
        let mut hidden = app.preferences().clone();
        hidden.title_logo = false;
        app.apply_preferences(hidden);
        let add_page = app.registry.tool("sheet-page").unwrap().action;
        app.invoke(add_page, &[], Channel::Ui).unwrap();
        let document = app.current().session.document();
        let book = drawings::books(document).next().unwrap();
        let pages: Vec<bool> = drawings::read(document, book)
            .unwrap()
            .pages
            .iter()
            .map(|page| page.settings.app_logo)
            .collect();
        // The existing page keeps its logo; the new page follows Settings.
        assert_eq!(pages, [true, false]);
        crate::preferences::set_title_logo_default(true);
    }
}
