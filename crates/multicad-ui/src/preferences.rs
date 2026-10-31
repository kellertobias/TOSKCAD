//! User-local application preferences (CAD-187): the default length unit and
//! the visibility and order of each mode's ribbon workbenches. Preferences
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbenchPreference {
    pub title: String,
    pub visible: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub length_unit: LengthUnit,
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
        let stored = std::mem::take(&mut self.workbenches);
        for mode in Mode::ALL {
            let installed: Vec<&str> = registry
                .workspace(mode)
                .tabs
                .iter()
                .map(|tab| tab.title)
                .collect();
            let mut entries: Vec<WorkbenchPreference> = Vec::new();
            for entry in stored.get(&mode).into_iter().flatten() {
                if installed.contains(&entry.title.as_str())
                    && !entries.iter().any(|e| e.title == entry.title)
                {
                    entries.push(entry.clone());
                }
            }
            for title in installed {
                if !entries.iter().any(|e| e.title == title) {
                    entries.push(WorkbenchPreference {
                        title: title.into(),
                        visible: true,
                    });
                }
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
        for mode in Mode::ALL {
            let tabs = &registry.workspace(mode).tabs;
            assert_eq!(tabs[0].title, ALWAYS_VISIBLE, "{mode:?}");
            assert_eq!(
                preferences.visible_tabs(&registry, mode),
                (0..tabs.len()).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn hide_show_and_reorder_change_only_navigation_order() {
        let registry = Registry::default();
        let mut preferences = Preferences::defaults(&registry);
        preferences.set_visible(Mode::Model, "Wood", false).unwrap();
        preferences
            .move_workbench(Mode::Model, "Materials", -1)
            .unwrap();
        preferences
            .move_workbench(Mode::Model, "Sheet metal", -3)
            .unwrap();
        let model = titles(&preferences, &registry, Mode::Model);
        assert!(!model.contains(&"Wood"));
        assert_eq!(&model[..3], ["Sheet metal", "Common", "Architecture"]);
        let materials = model.iter().position(|t| *t == "Materials").unwrap();
        assert_eq!(model[materials + 1], "FDM aids");
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
                .move_workbench(Mode::Model, "Sheet metal", -1)
                .is_err()
        );
        assert!(preferences.set_visible(Mode::Model, "Nope", false).is_err());
        assert_eq!(preferences, before);
    }
    #[test]
    fn common_cannot_be_hidden_so_navigation_never_becomes_empty() {
        let registry = Registry::default();
        let mut preferences = Preferences::defaults(&registry);
        for mode in Mode::ALL {
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
