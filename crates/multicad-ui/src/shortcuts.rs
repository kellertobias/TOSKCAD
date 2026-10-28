//! Declarative bindings resolved from specific workspace to global context.
//! Keyboard navigation never edits source documents directly.
use crate::registry::{Command, Mode};
use crate::sketch_editor::DrawTool;
use crate::sketch_operations::OperationTool;
use eframe::egui::{self, Key, Modifiers, Vec2};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub primary: bool,
    pub shift: bool,
}
impl Chord {
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            primary: false,
            shift: false,
        }
    }
    pub const fn shifted(key: Key) -> Self {
        Self {
            key,
            primary: false,
            shift: true,
        }
    }
    pub const fn primary(key: Key) -> Self {
        Self {
            key,
            primary: true,
            shift: false,
        }
    }
    pub const fn primary_shift(key: Key) -> Self {
        Self {
            key,
            primary: true,
            shift: true,
        }
    }
    fn matches(self, key: Key, m: Modifiers) -> bool {
        self.key == key
            && self.primary == m.command
            && self.shift == m.shift
            && !m.alt
            && (!m.ctrl || m.command)
            && (!m.mac_cmd || m.command)
            && !(m.ctrl && m.mac_cmd)
    }
    pub fn label(self) -> String {
        format!(
            "{}{}{}",
            if self.primary {
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl+"
                }
            } else {
                ""
            },
            if self.shift { "Shift+" } else { "" },
            self.key.name()
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Viewport,
    Mode(Mode),
    SketchEditing,
    Workspace(Mode, &'static str),
}
impl Scope {
    fn priority(self) -> u8 {
        match self {
            Self::Global => 0,
            Self::Viewport => 1,
            Self::Mode(_) => 2,
            Self::SketchEditing => 3,
            Self::Workspace(..) => 4,
        }
    }
    fn matches(self, c: &Context<'_>) -> bool {
        match self {
            Self::Global => true,
            Self::Viewport => c.viewport,
            Self::Mode(m) => m == c.mode,
            Self::SketchEditing => c.mode == Mode::Sketch && c.sketch_editing,
            Self::Workspace(m, t) => m == c.mode && t == c.workspace,
        }
    }
    pub fn label(self) -> String {
        match self {
            Self::Global => "Global".into(),
            Self::Viewport => "Viewport".into(),
            Self::Mode(m) => m.title().into(),
            Self::SketchEditing => "Editing a sketch".into(),
            Self::Workspace(m, t) => format!("{} / {t}", m.title()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pan {
    Up,
    Left,
    Down,
    Right,
}
impl Pan {
    fn vector(self) -> Vec2 {
        match self {
            Self::Up => Vec2::new(0., -1.),
            Self::Left => Vec2::new(-1., 0.),
            Self::Down => Vec2::new(0., 1.),
            Self::Right => Vec2::new(1., 0.),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Command(Command),
    FindTool,
    FocusCommand,
    NextWorkspace,
    PreviousWorkspace,
    Pan(Pan),
    Fit,
    Home,
    Projection,
    Transform,
    Cancel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub chord: Chord,
    pub scope: Scope,
    pub action: Action,
    pub label: &'static str,
    pub allow_in_text: bool,
}
pub struct Context<'a> {
    pub mode: Mode,
    pub workspace: &'a str,
    pub sketch_editing: bool,
    pub viewport: bool,
    pub text_input: bool,
    pub blocked: bool,
}
#[derive(Default)]
pub struct ShortcutMap {
    bindings: Vec<Binding>,
}
impl ShortcutMap {
    /// Same chord is legal in different scopes. Equal-scope duplicates are errors.
    pub fn bind(&mut self, binding: Binding) -> Result<(), String> {
        if self
            .bindings
            .iter()
            .any(|b| b.chord == binding.chord && b.scope == binding.scope)
        {
            return Err(format!(
                "Duplicate {} shortcut in {}",
                binding.chord.label(),
                binding.scope.label()
            ));
        }
        if binding.allow_in_text && !binding.chord.primary {
            return Err("Typing overrides require the primary modifier".into());
        }
        if !binding.chord.primary
            && !binding.chord.shift
            && matches!(binding.chord.key, Key::W | Key::A | Key::S | Key::D)
            && !matches!(binding.action, Action::Pan(_))
        {
            return Err("WASD is reserved for viewport navigation".into());
        }
        self.bindings.push(binding);
        Ok(())
    }
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
    pub fn resolve(
        &self,
        key: Key,
        modifiers: Modifiers,
        context: &Context<'_>,
    ) -> Option<Binding> {
        if context.blocked {
            return None;
        }
        self.bindings
            .iter()
            .filter(|b| {
                b.scope.matches(context)
                    && b.chord.matches(key, modifiers)
                    && (!context.text_input || b.allow_in_text)
            })
            .max_by_key(|b| b.scope.priority())
            .copied()
    }
    pub fn hint(&self, command: Command, context: &Context<'_>) -> Option<String> {
        let hints: Vec<_> = self
            .bindings
            .iter()
            .filter(|b| b.action == Action::Command(command) && b.scope.matches(context))
            .filter(|b| {
                !self.bindings.iter().any(|other| {
                    other.chord == b.chord
                        && other.scope.matches(context)
                        && other.scope.priority() > b.scope.priority()
                })
            })
            .map(|b| b.chord.label())
            .collect();
        (!hints.is_empty()).then(|| hints.join(" / "))
    }
    pub fn standard() -> Self {
        use Action::*;
        let mut map = Self::default();
        let mut add = |scope, chord, action, label, allow_in_text| {
            map.bind(Binding {
                scope,
                chord,
                action,
                label,
                allow_in_text,
            })
            .expect("valid standard shortcut")
        };
        for (key, command, label) in [
            (Key::S, crate::registry::Command::Save, "Save"),
            (Key::N, crate::registry::Command::New, "New document"),
            (Key::O, crate::registry::Command::Open, "Open document"),
        ] {
            add(
                Scope::Global,
                Chord::primary(key),
                Command(command),
                label,
                true,
            );
        }
        add(
            Scope::Global,
            Chord::primary_shift(Key::S),
            Command(crate::registry::Command::SaveAs),
            "Save project as",
            true,
        );
        add(
            Scope::Global,
            Chord::primary(Key::Z),
            Command(crate::registry::Command::Undo),
            "Undo",
            false,
        );
        add(
            Scope::Global,
            Chord::primary_shift(Key::Z),
            Command(crate::registry::Command::Redo),
            "Redo",
            false,
        );
        add(
            Scope::Global,
            Chord::primary(Key::F),
            FindTool,
            "Find a tool",
            true,
        );
        add(
            Scope::Global,
            Chord::plain(Key::Backtick),
            FocusCommand,
            "Focus command line",
            false,
        );
        add(
            Scope::Global,
            Chord::plain(Key::F1),
            Command(crate::registry::Command::KeyboardShortcuts),
            "Keyboard shortcuts",
            false,
        );
        add(
            Scope::Global,
            Chord::plain(Key::Tab),
            NextWorkspace,
            "Next ribbon workspace",
            false,
        );
        add(
            Scope::Global,
            Chord::shifted(Key::Tab),
            PreviousWorkspace,
            "Previous ribbon workspace",
            false,
        );
        for (key, pan) in [
            (Key::W, crate::shortcuts::Pan::Up),
            (Key::A, crate::shortcuts::Pan::Left),
            (Key::S, crate::shortcuts::Pan::Down),
            (Key::D, crate::shortcuts::Pan::Right),
        ] {
            add(
                Scope::Viewport,
                Chord::plain(key),
                Pan(pan),
                "Pan viewport",
                false,
            );
        }
        for (key, action, label) in [
            (Key::F, Fit, "Fit view"),
            (Key::V, Projection, "Toggle projection"),
            (Key::X, Transform, "Toggle transform gizmo"),
            (Key::Escape, Cancel, "Cancel active interaction"),
        ] {
            add(Scope::Viewport, Chord::plain(key), action, label, false);
        }
        add(
            Scope::Viewport,
            Chord::shifted(Key::F),
            Home,
            "Home view",
            false,
        );
        for (key, command, label) in [
            (Key::E, crate::registry::Command::Apply, "Apply and rebuild"),
            (
                Key::Q,
                crate::registry::Command::Discard,
                "Discard parameter draft",
            ),
            (
                Key::R,
                crate::registry::Command::Preview,
                "Preview parameters",
            ),
        ] {
            add(
                Scope::Mode(Mode::Parameters),
                Chord::plain(key),
                Command(command),
                label,
                false,
            );
        }
        add(
            Scope::Mode(Mode::Parameters),
            Chord::primary(Key::Enter),
            Command(crate::registry::Command::Apply),
            "Apply and rebuild from an input",
            true,
        );
        add(
            Scope::Mode(Mode::Sketch),
            Chord::plain(Key::Q),
            Command(crate::registry::Command::NewSketch),
            "New sketch",
            false,
        );
        for (key, command, label) in [
            (
                Key::E,
                crate::registry::Command::Draw(DrawTool::Line),
                "Line",
            ),
            (
                Key::R,
                crate::registry::Command::Draw(DrawTool::Rectangle),
                "Rectangle",
            ),
            (
                Key::F,
                crate::registry::Command::Draw(DrawTool::Circle),
                "Circle",
            ),
            (Key::G, crate::registry::Command::Draw(DrawTool::Arc), "Arc"),
            (
                Key::V,
                crate::registry::Command::Draw(DrawTool::Spline),
                "Spline",
            ),
            (
                Key::Enter,
                crate::registry::Command::FinishCurve,
                "Finish curve",
            ),
            (
                Key::T,
                crate::registry::Command::Draw(DrawTool::Polyline),
                "Polyline",
            ),
            (Key::C, crate::registry::Command::Dimension, "Dimension"),
            (
                Key::Q,
                crate::registry::Command::FinishSketch,
                "Finish sketch",
            ),
            (
                Key::Delete,
                crate::registry::Command::Delete,
                "Delete selected geometry",
            ),
        ] {
            add(
                Scope::SketchEditing,
                Chord::plain(key),
                Command(command),
                label,
                false,
            );
        }
        for (key, tool) in [
            (Key::E, OperationTool::Move),
            (Key::R, OperationTool::Rotate),
            (Key::F, OperationTool::Mirror),
            (Key::G, OperationTool::LinearPattern),
            (Key::V, OperationTool::CircularPattern),
            (Key::T, OperationTool::Trim),
            (Key::X, OperationTool::Extend),
        ] {
            add(
                Scope::SketchEditing,
                Chord::shifted(key),
                Command(crate::registry::Command::SketchOperation(tool)),
                tool.label(),
                false,
            );
        }
        add(
            Scope::Mode(Mode::Model),
            Chord::plain(Key::E),
            Command(crate::registry::Command::Modeling(
                crate::modeling::ModelingTool::Extrude,
            )),
            "Extrude selected profiles",
            false,
        );
        add(
            Scope::Mode(Mode::Model),
            Chord::plain(Key::R),
            Command(crate::registry::Command::Extension("revolve")),
            "Revolve (when available)",
            false,
        );
        map
    }
}
#[derive(Default)]
pub struct Runtime {
    pan_keys: BTreeSet<Key>,
}
#[derive(Default)]
pub struct FrameActions {
    pub actions: Vec<Action>,
    pub pan: Vec2,
}
impl Runtime {
    pub fn collect(
        &mut self,
        ctx: &egui::Context,
        map: &ShortcutMap,
        context: &Context<'_>,
    ) -> FrameActions {
        let mut result = FrameActions::default();
        let (focused, pointer_pressed, mods, keys) = ctx.input(|i| {
            (
                i.focused,
                i.pointer.any_pressed(),
                i.modifiers,
                i.keys_down.clone(),
            )
        });
        if !focused || context.blocked || context.text_input || !context.viewport || mods.any() {
            self.pan_keys.clear();
        }
        self.pan_keys.retain(|k| keys.contains(k));
        if !focused || pointer_pressed {
            return result;
        }
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                    && let Some(binding) = map.resolve(*key, *modifiers, context)
                {
                    if matches!(binding.action, Action::Pan(_)) {
                        if context.viewport && !context.text_input {
                            self.pan_keys.insert(*key);
                        }
                    } else {
                        result.actions.push(binding.action);
                    }
                    return false;
                }
                true
            })
        });
        for key in &self.pan_keys {
            if let Some(Binding {
                action: Action::Pan(pan),
                ..
            }) = map.resolve(*key, mods, context)
            {
                result.pan += pan.vector();
            }
        }
        if result.pan.length_sq() > 1. {
            result.pan = result.pan.normalized();
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn context(mode: Mode) -> Context<'static> {
        Context {
            mode,
            workspace: "Common",
            sketch_editing: false,
            viewport: mode != Mode::Parameters,
            text_input: false,
            blocked: false,
        }
    }
    fn key(key: Key, modifiers: Modifiers, pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        }
    }
    fn frame(
        ctx: &egui::Context,
        runtime: &mut Runtime,
        map: &ShortcutMap,
        c: &Context<'_>,
        mut events: Vec<egui::Event>,
        modifiers: Modifiers,
    ) -> FrameActions {
        let mut result = FrameActions::default();
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        ctx.run_ui(
            egui::RawInput {
                focused: true,
                events,
                ..Default::default()
            },
            |_| result = runtime.collect(ctx, map, c),
        )
        .drop_without_applying_deltas();
        result
    }
    #[test]
    fn context_overrides_are_deterministic_and_collisions_are_rejected() {
        let mut map = ShortcutMap::standard();
        let mut c = context(Mode::Parameters);
        assert_eq!(
            map.resolve(Key::E, Modifiers::NONE, &c).unwrap().action,
            Action::Command(Command::Apply)
        );
        c.mode = Mode::Sketch;
        c.sketch_editing = true;
        assert_eq!(
            map.resolve(Key::E, Modifiers::NONE, &c).unwrap().action,
            Action::Command(Command::Draw(DrawTool::Line))
        );
        assert_eq!(
            map.resolve(Key::Q, Modifiers::NONE, &c).unwrap().action,
            Action::Command(Command::FinishSketch)
        );
        c.sketch_editing = false;
        assert_eq!(
            map.resolve(Key::Q, Modifiers::NONE, &c).unwrap().action,
            Action::Command(Command::NewSketch)
        );
        c.mode = Mode::Model;
        c.workspace = "Wood";
        let binding = Binding {
            chord: Chord::plain(Key::E),
            scope: Scope::Workspace(Mode::Model, "Wood"),
            action: Action::Command(Command::Extension("drill")),
            label: "Drill",
            allow_in_text: false,
        };
        map.bind(binding).unwrap();
        assert!(map.bind(binding).is_err());
        assert_eq!(
            map.resolve(Key::E, Modifiers::NONE, &c).unwrap().action,
            binding.action
        );
        assert!(
            map.hint(
                Command::Modeling(crate::modeling::ModelingTool::Extrude),
                &c
            )
            .is_none()
        );
        c.workspace = "Common";
        assert_eq!(
            map.resolve(Key::E, Modifiers::NONE, &c).unwrap().action,
            Action::Command(Command::Modeling(crate::modeling::ModelingTool::Extrude))
        );
        assert!(
            map.bind(Binding {
                chord: Chord::plain(Key::W),
                scope: Scope::Mode(Mode::Model),
                ..binding
            })
            .is_err()
        );
    }
    #[test]
    fn drawing_keys_use_sketch_scope_and_fit_returns_when_editing_ends() {
        let map = ShortcutMap::standard();
        let mut c = context(Mode::Sketch);
        c.sketch_editing = true;
        for (key, tool) in [
            (Key::E, DrawTool::Line),
            (Key::R, DrawTool::Rectangle),
            (Key::F, DrawTool::Circle),
            (Key::G, DrawTool::Arc),
            (Key::T, DrawTool::Polyline),
        ] {
            assert_eq!(
                map.resolve(key, Modifiers::NONE, &c).unwrap().action,
                Action::Command(Command::Draw(tool))
            );
        }
        assert_eq!(
            map.resolve(Key::C, Modifiers::NONE, &c).unwrap().action,
            Action::Command(Command::Dimension)
        );
        c.sketch_editing = false;
        assert_eq!(
            map.resolve(Key::F, Modifiers::NONE, &c).unwrap().action,
            Action::Fit
        );
        for key in [Key::E, Key::R, Key::G, Key::T] {
            assert!(map.resolve(key, Modifiers::NONE, &c).is_none());
        }
        c.sketch_editing = true;
        c.text_input = true;
        for key in [Key::E, Key::R, Key::F, Key::G, Key::T] {
            assert!(map.resolve(key, Modifiers::NONE, &c).is_none());
        }
    }
    #[test]
    fn text_modal_and_modifier_guards_preserve_typing() {
        let map = ShortcutMap::standard();
        let mut c = context(Mode::Sketch);
        c.sketch_editing = true;
        c.text_input = true;
        for key in [
            Key::W,
            Key::A,
            Key::S,
            Key::D,
            Key::E,
            Key::Q,
            Key::R,
            Key::Tab,
            Key::Backtick,
        ] {
            assert!(map.resolve(key, Modifiers::NONE, &c).is_none());
        }
        assert!(map.resolve(Key::Z, Modifiers::COMMAND, &c).is_none());
        assert_eq!(
            map.resolve(Key::S, Modifiers::COMMAND, &c).unwrap().action,
            Action::Command(Command::Save)
        );
        c.blocked = true;
        assert!(map.resolve(Key::S, Modifiers::COMMAND, &c).is_none());
        c.blocked = false;
        c.text_input = false;
        assert!(map.resolve(Key::E, Modifiers::ALT, &c).is_none());
    }
    #[test]
    fn held_navigation_stops_for_typing_and_does_not_arm_from_save() {
        let ctx = egui::Context::default();
        let map = ShortcutMap::standard();
        let mut runtime = Runtime::default();
        let mut c = context(Mode::Model);
        frame(&ctx, &mut runtime, &map, &c, vec![], Modifiers::NONE);
        let first = frame(
            &ctx,
            &mut runtime,
            &map,
            &c,
            vec![key(Key::W, Modifiers::NONE, true, false)],
            Modifiers::NONE,
        );
        assert_eq!(first.pan, Vec2::new(0., -1.));
        assert_eq!(
            frame(&ctx, &mut runtime, &map, &c, vec![], Modifiers::NONE).pan,
            first.pan
        );
        c.text_input = true;
        assert_eq!(
            frame(&ctx, &mut runtime, &map, &c, vec![], Modifiers::NONE).pan,
            Vec2::ZERO
        );
        c.text_input = false;
        let saved = frame(
            &ctx,
            &mut runtime,
            &map,
            &c,
            vec![key(Key::S, Modifiers::COMMAND, true, false)],
            Modifiers::COMMAND,
        );
        assert_eq!(saved.actions, vec![Action::Command(Command::Save)]);
        assert_eq!(saved.pan, Vec2::ZERO);
        assert_eq!(
            frame(&ctx, &mut runtime, &map, &c, vec![], Modifiers::NONE).pan,
            Vec2::ZERO
        );
        frame(
            &ctx,
            &mut runtime,
            &map,
            &c,
            vec![
                key(Key::S, Modifiers::NONE, false, false),
                key(Key::W, Modifiers::NONE, false, false),
            ],
            Modifiers::NONE,
        );
        let diagonal = frame(
            &ctx,
            &mut runtime,
            &map,
            &c,
            vec![
                key(Key::W, Modifiers::NONE, true, false),
                key(Key::D, Modifiers::NONE, true, false),
            ],
            Modifiers::NONE,
        );
        assert!((diagonal.pan.length() - 1.).abs() < 1e-6);
    }
    #[test]
    fn repeated_discrete_keys_do_not_repeat_transactions() {
        let ctx = egui::Context::default();
        let map = ShortcutMap::standard();
        let mut runtime = Runtime::default();
        let c = context(Mode::Parameters);
        frame(&ctx, &mut runtime, &map, &c, vec![], Modifiers::NONE);
        assert_eq!(
            frame(
                &ctx,
                &mut runtime,
                &map,
                &c,
                vec![key(Key::E, Modifiers::NONE, true, false)],
                Modifiers::NONE
            )
            .actions,
            vec![Action::Command(Command::Apply)]
        );
        assert!(
            frame(
                &ctx,
                &mut runtime,
                &map,
                &c,
                vec![key(Key::E, Modifiers::NONE, true, true)],
                Modifiers::NONE
            )
            .actions
            .is_empty()
        );
    }
    #[test]
    fn sketch_edit_shortcuts_are_scoped_and_preserve_navigation_and_text_input() {
        let map = ShortcutMap::standard();
        let mut c = context(Mode::Sketch);
        c.sketch_editing = true;
        for (key, tool) in [
            (Key::E, OperationTool::Move),
            (Key::R, OperationTool::Rotate),
            (Key::F, OperationTool::Mirror),
            (Key::G, OperationTool::LinearPattern),
            (Key::V, OperationTool::CircularPattern),
            (Key::T, OperationTool::Trim),
            (Key::X, OperationTool::Extend),
        ] {
            assert_eq!(
                map.resolve(key, Modifiers::SHIFT, &c).unwrap().action,
                Action::Command(Command::SketchOperation(tool))
            );
        }
        assert_eq!(
            map.resolve(Key::W, Modifiers::NONE, &c).unwrap().action,
            Action::Pan(Pan::Up)
        );
        assert_eq!(
            map.resolve(Key::Tab, Modifiers::NONE, &c).unwrap().action,
            Action::NextWorkspace
        );
        c.sketch_editing = false;
        assert_eq!(
            map.resolve(Key::F, Modifiers::SHIFT, &c).unwrap().action,
            Action::Home
        );
        c.sketch_editing = true;
        c.text_input = true;
        assert!(map.resolve(Key::E, Modifiers::SHIFT, &c).is_none());
    }
}
