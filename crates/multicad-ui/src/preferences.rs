//! User-local application preferences (CAD-187): the default length unit and
//! Scale drag step and each mode's ribbon workbenches. Preferences
//! are never part of a project, Undo or MCP document state; they change
//! presentation and input defaults only.
use crate::registry::{Mode, Registry};
pub use multicad_core::parameters::LengthUnit;
use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
};

/// Every mode's first workbench. It stays visible so no mode can lose its
/// whole navigation, and it is the fallback when the active one is hidden.
pub const ALWAYS_VISIBLE: &str = "Common";
const MAX_TITLE: usize = 80;
pub const DEFAULT_SCALE_STEP_PERCENT: u8 = 10;
pub(crate) const SCALE_STEP_PERCENT_RANGE: std::ops::RangeInclusive<u8> = 1..=100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbenchPreference {
    pub title: String,
    pub visible: bool,
}
/// Select the single saved slot for the combined Model workshop.
fn metal_workbench(entries: &[WorkbenchPreference]) -> Option<(usize, bool)> {
    let named = |title| {
        entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.title == title)
    };
    if let Some((index, metal)) = named(crate::registry::METAL_TITLE) {
        return Some((index, metal.visible));
    }
    let sheet = named("Sheet metal");
    let weld = named("Weld");
    let (index, _) = sheet.or(weld)?;
    // An omitted workbench would have been visible under the old defaults.
    let visible =
        sheet.is_none_or(|(_, entry)| entry.visible) || weld.is_none_or(|(_, entry)| entry.visible);
    Some((index, visible))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub length_unit: LengthUnit,
    /// Factor increment while holding Shift during a Scale drag.
    pub scale_step_percent: u8,
    /// Per mode, workbench titles in display order. Missing modes or titles
    /// fall back to the established registry order, visible.
    pub workbenches: BTreeMap<Mode, Vec<WorkbenchPreference>>,
    /// Default of the per-page "Show app logo in title block" option for
    /// new drawing pages (CAD-177). Existing pages keep their own setting.
    pub title_logo: bool,
}
impl Preferences {
    /// Millimetres and every workbench visible in the established order.
    pub fn defaults(registry: &Registry) -> Self {
        let mut preferences = Self {
            length_unit: LengthUnit::default(),
            scale_step_percent: DEFAULT_SCALE_STEP_PERCENT,
            workbenches: BTreeMap::new(),
            title_logo: true,
        };
        preferences.normalize(registry);
        preferences
    }
    /// Reconcile stored entries with the installed workbenches: unknown or
    /// duplicate titles are dropped, newly installed ones are appended visible
    /// and the Common workbench is forced visible.
    pub fn normalize(&mut self, registry: &Registry) {
        self.scale_step_percent = self.scale_step_percent.clamp(
            *SCALE_STEP_PERCENT_RANGE.start(),
            *SCALE_STEP_PERCENT_RANGE.end(),
        );
        let stored = std::mem::take(&mut self.workbenches);
        for mode in Mode::WORKSPACES {
            let installed: Vec<&str> = registry
                .workspace(mode)
                .tabs
                .iter()
                .map(|tab| tab.title)
                .collect();
            let mut entries: Vec<WorkbenchPreference> = Vec::new();
            let saved = stored.get(&mode).map_or(&[][..], Vec::as_slice);
            let metal = if mode == Mode::Model {
                metal_workbench(saved)
            } else {
                None
            };
            let old_model_order = mode == Mode::Model
                && saved.iter().map(|entry| entry.title.as_str()).eq([
                    "Common",
                    "Architecture",
                    "Wood",
                    "Sheet metal",
                    "Weld",
                    "Other",
                    "Relationships",
                    "Mechanical",
                ]);
            let legacy_fdm =
                mode == Mode::Model && saved.iter().any(|entry| entry.title == "FDM aids");
            for (index, stored_entry) in saved.iter().enumerate() {
                let title = crate::registry::canonical_workbench_title(mode, &stored_entry.title);
                let visible = if mode == Mode::Model && title == crate::registry::METAL_TITLE {
                    let Some((slot, visible)) = metal else {
                        continue;
                    };
                    if slot != index {
                        continue;
                    }
                    visible
                } else {
                    stored_entry.visible
                };
                if installed.contains(&title) {
                    let entry = WorkbenchPreference {
                        title: title.into(),
                        visible,
                    };
                    if let Some(previous) = entries.iter_mut().find(|e| e.title == entry.title) {
                        if entry.title == "Other" {
                            previous.visible |= entry.visible;
                        }
                    } else {
                        entries.push(entry);
                    }
                }
            }
            for &title in &installed {
                if !entries.iter().any(|e| e.title == title) {
                    entries.push(WorkbenchPreference {
                        title: title.into(),
                        visible: true,
                    });
                }
            }
            if old_model_order {
                entries
                    .sort_by_key(|entry| installed.iter().position(|title| *title == entry.title));
            }
            if legacy_fdm
                && let Some(index) = entries.iter().position(|entry| entry.title == "Other")
            {
                let fdm = entries.remove(index);
                let before = entries
                    .iter()
                    .position(|entry| entry.title == "Relations")
                    .unwrap_or(entries.len());
                entries.insert(before, fdm);
            }
            for entry in &mut entries {
                if entry.title == ALWAYS_VISIBLE {
                    entry.visible = true;
                }
            }
            self.workbenches.insert(mode, entries);
        }
    }
    /// Validate untrusted stored data before it is normalized.
    pub fn validate(&self) -> Result<(), String> {
        if !SCALE_STEP_PERCENT_RANGE.contains(&self.scale_step_percent) {
            return Err("Scale step must be between 1% and 100%".into());
        }
        for entries in self.workbenches.values() {
            if entries.len() > 64 {
                return Err("Too many workbench entries".into());
            }
            for entry in entries {
                if entry.title.trim().is_empty()
                    || entry.title.chars().count() > MAX_TITLE
                    || entry.title.chars().any(char::is_control)
                {
                    return Err("Invalid workbench title".into());
                }
            }
        }
        Ok(())
    }
    fn entries(&self, mode: Mode) -> &[WorkbenchPreference] {
        self.workbenches.get(&mode).map_or(&[], Vec::as_slice)
    }
    /// Registry tab indices of `mode` in display order, with visibility.
    pub fn order(&self, registry: &Registry, mode: Mode) -> Vec<(usize, bool)> {
        let tabs = &registry.workspace(mode).tabs;
        self.entries(mode)
            .iter()
            .filter_map(|entry| {
                tabs.iter()
                    .position(|tab| tab.title == entry.title)
                    .map(|index| (index, entry.visible))
            })
            .collect()
    }
    /// Visible registry tab indices of `mode` in display order. Never empty
    /// for a normalized preference set.
    pub fn visible_tabs(&self, registry: &Registry, mode: Mode) -> Vec<usize> {
        let visible: Vec<usize> = self
            .order(registry, mode)
            .into_iter()
            .filter_map(|(index, visible)| visible.then_some(index))
            .collect();
        if visible.is_empty() { vec![0] } else { visible }
    }
    pub fn is_visible(&self, registry: &Registry, mode: Mode, tab: usize) -> bool {
        self.visible_tabs(registry, mode).contains(&tab)
    }
    pub fn set_visible(&mut self, mode: Mode, title: &str, visible: bool) -> Result<(), String> {
        if !visible && title == ALWAYS_VISIBLE {
            return Err(format!(
                "{} › {ALWAYS_VISIBLE} stays visible so the mode keeps a workbench",
                mode.title()
            ));
        }
        let entry = self
            .workbenches
            .get_mut(&mode)
            .and_then(|entries| entries.iter_mut().find(|e| e.title == title))
            .ok_or_else(|| format!("Unknown workbench {} › {title}", mode.title()))?;
        entry.visible = visible;
        Ok(())
    }
    /// Move a workbench one position earlier (`-1`) or later (`+1`).
    pub fn move_workbench(&mut self, mode: Mode, title: &str, delta: isize) -> Result<(), String> {
        let entries = self
            .workbenches
            .get_mut(&mode)
            .ok_or_else(|| format!("Unknown mode {}", mode.title()))?;
        let from = entries
            .iter()
            .position(|e| e.title == title)
            .ok_or_else(|| format!("Unknown workbench {} › {title}", mode.title()))?;
        let to = from
            .checked_add_signed(delta)
            .filter(|to| *to < entries.len())
            .ok_or("The workbench is already at that end")?;
        let entry = entries.remove(from);
        entries.insert(to, entry);
        Ok(())
    }
}

thread_local! {
    static SCALE_STEP_PERCENT: Cell<u8> = const { Cell::new(DEFAULT_SCALE_STEP_PERCENT) };
    static LENGTH_UNIT: Cell<LengthUnit> = const { Cell::new(LengthUnit::Millimeter) };
    static TITLE_LOGO: Cell<bool> = const { Cell::new(true) };
}
/// Seed for the app-logo option of new drawing pages on the UI thread.
pub fn title_logo_default() -> bool {
    TITLE_LOGO.with(Cell::get)
}
pub fn set_title_logo_default(shown: bool) {
    TITLE_LOGO.with(|cell| cell.set(shown));
}
/// Display/input default for unqualified lengths on the UI thread. Document
/// evaluation never reads it; values are stored with an explicit unit.
pub fn length_unit() -> LengthUnit {
    LENGTH_UNIT.with(Cell::get)
}
pub fn set_length_unit(unit: LengthUnit) {
    LENGTH_UNIT.with(|cell| cell.set(unit));
}
/// Factor increment for Shift-dragging a Scale handle on the UI thread.
pub fn scale_step() -> f64 {
    f64::from(SCALE_STEP_PERCENT.with(Cell::get)) / 100.
}
pub(crate) fn set_scale_step_percent(percent: u8) {
    SCALE_STEP_PERCENT.with(|cell| cell.set(percent));
}
/// Format a millimetre quantity in the preferred unit with its label.
pub fn format_mm(millimeters: f64) -> String {
    length_unit().format_mm(millimeters)
}

/// Persistent user-local preference storage.
pub trait PreferenceStore: Send + 'static {
    /// `Ok(None)` when nothing has been saved yet.
    fn load(&mut self) -> Result<Option<Preferences>, String>;
    fn save(&mut self, preferences: &Preferences) -> Result<(), String>;
}
/// Writes preferences on a dedicated thread, coalescing bursts so only the
/// latest state is written.
pub struct PreferenceWorker {
    tx: Sender<Preferences>,
    rx: Receiver<Result<(), String>>,
}
impl PreferenceWorker {
    pub fn new(mut store: Box<dyn PreferenceStore>) -> Result<Self, String> {
        let (tx, jobs) = mpsc::channel::<Preferences>();
        let (results, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("multicad-preferences".into())
            .spawn(move || {
                while let Ok(mut latest) = jobs.recv() {
                    while let Ok(newer) = jobs.try_recv() {
                        latest = newer;
                    }
                    if results.send(store.save(&latest)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { tx, rx })
    }
    pub fn save(&self, preferences: &Preferences) -> Result<(), String> {
        self.tx
            .send(preferences.clone())
            .map_err(|_| "Preference writer stopped".to_string())
    }
    /// The most recent completed write result, if any arrived.
    pub fn poll(&self) -> Option<Result<(), String>> {
        let mut last = None;
        loop {
            match self.rx.try_recv() {
                Ok(result) => last = Some(result),
                Err(TryRecvError::Empty) => return last,
                Err(TryRecvError::Disconnected) => {
                    return Some(Err("Preference writer stopped".into()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn titles(preferences: &Preferences, registry: &Registry, mode: Mode) -> Vec<&'static str> {
        preferences
            .visible_tabs(registry, mode)
            .into_iter()
            .map(|i| registry.workspace(mode).tabs[i].title)
            .collect()
    }
    #[test]
    fn defaults_keep_every_workbench_visible_in_the_established_order() {
        let registry = Registry::default();
        let preferences = Preferences::defaults(&registry);
        assert_eq!(preferences.length_unit, LengthUnit::Millimeter);
        for mode in Mode::WORKSPACES {
            let tabs = &registry.workspace(mode).tabs;
            let first = if mode == Mode::Parameters {
                "General"
            } else {
                ALWAYS_VISIBLE
            };
            assert_eq!(tabs[0].title, first, "{mode:?}");
            assert_eq!(
                preferences.visible_tabs(&registry, mode),
                (0..tabs.len()).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn existing_preferences_gain_mechanical_without_changing_other_workbenches() {
        let registry = Registry::default();
        let mut preferences = Preferences::defaults(&registry);
        preferences
            .set_visible(Mode::Model, "Metal", false)
            .unwrap();
        preferences
            .workbenches
            .get_mut(&Mode::Model)
            .unwrap()
            .retain(|entry| entry.title != "Mechanical");
        let previous = preferences.workbenches[&Mode::Model].clone();
        preferences.normalize(&registry);
        let model = &preferences.workbenches[&Mode::Model];
        assert_eq!(&model[..previous.len()], previous);
        assert_eq!(model.last().unwrap().title, "Mechanical");
        assert!(model.last().unwrap().visible);
        let normalized = preferences.clone();
        preferences.normalize(&registry);
        assert_eq!(preferences, normalized);
    }
    fn model_preferences(registry: &Registry, saved: &[(&str, bool)]) -> Preferences {
        let mut preferences = Preferences::defaults(registry);
        preferences.workbenches.insert(
            Mode::Model,
            saved
                .iter()
                .map(|&(title, visible)| WorkbenchPreference {
                    title: title.into(),
                    visible,
                })
                .collect(),
        );
        preferences
    }

    #[test]
    fn old_default_model_order_migrates_without_unhiding_tabs() {
        let registry = Registry::default();
        let mut preferences = model_preferences(
            &registry,
            &[
                ("Common", true),
                ("Architecture", true),
                ("Wood", true),
                ("Sheet metal", false),
                ("Weld", false),
                ("Other", true),
                ("Relationships", true),
                ("Mechanical", false),
            ],
        );
        preferences.normalize(&registry);
        let entries = &preferences.workbenches[&Mode::Model];
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.title.as_str(), entry.visible))
                .collect::<Vec<_>>(),
            [
                ("Common", true),
                ("Architecture", true),
                ("Wood", true),
                ("Metal", false),
                ("Mechanical", false),
                ("Other", true),
                ("Relations", true),
            ]
        );
    }

    #[test]
    fn metal_preserves_saved_sheet_position_and_either_legacy_visibility() {
        let registry = Registry::default();
        for (sheet, weld, expected) in [
            (true, true, true),
            (false, true, true),
            (true, false, true),
            (false, false, false),
        ] {
            let mut preferences = model_preferences(
                &registry,
                &[
                    ("Weld", weld),
                    ("Mechanical", false),
                    ("Common", true),
                    ("Sheet metal", sheet),
                    ("Wood", true),
                ],
            );
            preferences.normalize(&registry);
            let entries = &preferences.workbenches[&Mode::Model];
            assert_eq!(
                entries[..4]
                    .iter()
                    .map(|entry| (entry.title.as_str(), entry.visible))
                    .collect::<Vec<_>>(),
                [
                    ("Mechanical", false),
                    ("Common", true),
                    ("Metal", expected),
                    ("Wood", true)
                ]
            );
            assert!(
                entries
                    .iter()
                    .all(|entry| !matches!(entry.title.as_str(), "Sheet metal" | "Weld"))
            );
            let normalized = preferences.clone();
            preferences.normalize(&registry);
            assert_eq!(preferences, normalized);
        }
    }

    #[test]
    fn metal_defaults_to_visible_for_missing_legacy_entry_and_respects_modern_preference() {
        let registry = Registry::default();
        for legacy in ["Weld", "Sheet metal"] {
            let mut preferences = model_preferences(
                &registry,
                &[("Common", true), (legacy, false), ("Mechanical", true)],
            );
            preferences.normalize(&registry);
            assert_eq!(
                &titles(&preferences, &registry, Mode::Model)[..3],
                ["Common", "Metal", "Mechanical"]
            );
        }

        let mut preferences = model_preferences(
            &registry,
            &[
                ("Sheet metal", true),
                ("Mechanical", true),
                ("Weld", true),
                ("Common", true),
                ("Metal", false),
            ],
        );
        preferences.normalize(&registry);
        let entries = &preferences.workbenches[&Mode::Model];
        assert_eq!(
            entries[..3]
                .iter()
                .map(|entry| (entry.title.as_str(), entry.visible))
                .collect::<Vec<_>>(),
            [("Mechanical", true), ("Common", true), ("Metal", false)]
        );
    }
    #[test]
    fn legacy_fdm_preferences_keep_visibility_and_move_before_relationships() {
        let registry = Registry::default();
        let mut preferences = Preferences::defaults(&registry);
        let entries = preferences.workbenches.get_mut(&Mode::Model).unwrap();
        let index = entries
            .iter()
            .position(|entry| entry.title == "Other")
            .unwrap();
        let mut fdm = entries.remove(index);
        fdm.title = "FDM aids".into();
        fdm.visible = false;
        let after = entries
            .iter()
            .position(|entry| entry.title == "Relations")
            .unwrap()
            + 1;
        entries.insert(after, fdm);
        preferences.normalize(&registry);
        let entries = &preferences.workbenches[&Mode::Model];
        let fdm = entries
            .iter()
            .position(|entry| entry.title == "Other")
            .unwrap();
        assert!(!entries[fdm].visible);
        assert_eq!(entries[fdm + 1].title, "Relations");
        assert!(!entries.iter().any(|entry| entry.title == "FDM aids"));
        let normalized = preferences.clone();
        preferences.normalize(&registry);
        assert_eq!(preferences, normalized);
    }
    #[test]
    fn hide_show_and_reorder_change_only_navigation_order() {
        let registry = Registry::default();
        let mut preferences = Preferences::defaults(&registry);
        preferences.set_visible(Mode::Model, "Wood", false).unwrap();
        preferences
            .move_workbench(Mode::Model, "Other", -1)
            .unwrap();
        preferences
            .move_workbench(Mode::Model, "Metal", -3)
            .unwrap();
        let model = titles(&preferences, &registry, Mode::Model);
        assert!(!model.contains(&"Wood"));
        assert_eq!(&model[..3], ["Metal", "Common", "Architecture"]);
        let other = model.iter().position(|t| *t == "Other").unwrap();
        assert_eq!(model[other + 1], "Mechanical");
        // Other modes are untouched; showing it again restores it in place.
        assert_eq!(
            titles(&preferences, &registry, Mode::Export),
            ["Common", "Print", "Render"]
        );
        preferences.set_visible(Mode::Model, "Wood", true).unwrap();
        assert!(titles(&preferences, &registry, Mode::Model).contains(&"Wood"));
        // Invalid moves and unknown workbenches are reported, not applied.
        let before = preferences.clone();
        assert!(
            preferences
                .move_workbench(Mode::Model, "Metal", -1)
                .is_err()
        );
        assert!(preferences.set_visible(Mode::Model, "Nope", false).is_err());
        assert_eq!(preferences, before);
    }
    #[test]
    fn common_cannot_be_hidden_so_navigation_never_becomes_empty() {
        let registry = Registry::default();
        let mut preferences = Preferences::defaults(&registry);
        for mode in Mode::WORKSPACES
            .into_iter()
            .filter(|mode| *mode != Mode::Parameters)
        {
            for (index, _) in preferences.order(&registry, mode) {
                let title = registry.workspace(mode).tabs[index].title;
                let result = preferences.set_visible(mode, title, false);
                assert_eq!(result.is_err(), title == ALWAYS_VISIBLE, "{title}");
            }
            assert_eq!(titles(&preferences, &registry, mode), [ALWAYS_VISIBLE]);
        }
        // Stored data cannot hide Common either.
        preferences
            .workbenches
            .get_mut(&Mode::Sketch)
            .unwrap()
            .iter_mut()
            .for_each(|e| e.visible = false);
        preferences.normalize(&registry);
        assert_eq!(
            titles(&preferences, &registry, Mode::Sketch),
            [ALWAYS_VISIBLE]
        );
    }
    #[test]
    fn normalization_drops_unknown_entries_and_appends_new_workbenches() {
        let registry = Registry::default();
        let mut preferences = Preferences {
            title_logo: true,
            length_unit: LengthUnit::Inch,
            scale_step_percent: DEFAULT_SCALE_STEP_PERCENT,
            workbenches: BTreeMap::from([(
                Mode::Export,
                vec![
                    WorkbenchPreference {
                        title: "Render".into(),
                        visible: false,
                    },
                    WorkbenchPreference {
                        title: "Retired".into(),
                        visible: true,
                    },
                    WorkbenchPreference {
                        title: "Render".into(),
                        visible: true,
                    },
                ],
            )]),
        };
        preferences.validate().unwrap();
        preferences.normalize(&registry);
        assert_eq!(
            preferences.order(&registry, Mode::Export),
            [(2, false), (0, true), (1, true)]
        );
        assert_eq!(preferences.length_unit, LengthUnit::Inch);
        assert_eq!(
            preferences.order(&registry, Mode::Model).len(),
            registry.workspace(Mode::Model).tabs.len()
        );
        preferences.workbenches.get_mut(&Mode::Model).unwrap()[0].title = "bad\ntitle".into();
        assert!(preferences.validate().is_err());
    }
    #[test]
    fn worker_writes_the_latest_preferences() {
        use std::sync::{Arc, Mutex};
        struct Memory(Arc<Mutex<Vec<LengthUnit>>>);
        impl PreferenceStore for Memory {
            fn load(&mut self) -> Result<Option<Preferences>, String> {
                Ok(None)
            }
            fn save(&mut self, preferences: &Preferences) -> Result<(), String> {
                self.0.lock().unwrap().push(preferences.length_unit);
                Ok(())
            }
        }
        let saved = Arc::new(Mutex::new(vec![]));
        let worker = PreferenceWorker::new(Box::new(Memory(Arc::clone(&saved)))).unwrap();
        let mut preferences = Preferences::defaults(&Registry::default());
        preferences.length_unit = LengthUnit::Inch;
        worker.save(&preferences).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while worker.poll().is_none() {
            assert!(std::time::Instant::now() < deadline, "write timed out");
            std::thread::yield_now();
        }
        assert_eq!(saved.lock().unwrap().last(), Some(&LengthUnit::Inch));
    }
    #[test]
    fn length_unit_preference_is_thread_local_display_state() {
        assert_eq!(length_unit(), LengthUnit::Millimeter);
        set_length_unit(LengthUnit::Inch);
        assert_eq!(format_mm(50.8), "2.0000 in");
        std::thread::spawn(|| assert_eq!(length_unit(), LengthUnit::Millimeter))
            .join()
            .unwrap();
        set_length_unit(LengthUnit::Millimeter);
    }
}
