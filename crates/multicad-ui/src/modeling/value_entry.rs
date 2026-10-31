//! Pure drag / type / Enter state for operation gizmos (Extrude distance and
//! Revolve angle). It never touches a document: the controller turns effects
//! into previews and at most one transactional commit per opened tool.
//!
//! Rules:
//! - A drag updates the signed value continuously, including across zero.
//! - Typing during or after a drag takes over without focusing a field; later
//!   drag motion no longer changes the value.
//! - Pointer release never commits. Enter commits exactly once; every input
//!   after a commit is ignored until the host reopens the entry because the
//!   commit failed.
//! - Zero, nonfinite, out-of-range or incomplete text is reported as
//!   `Invalid` and never produces `Commit`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// Signed length from the configured start plane.
    Distance,
    /// Signed sweep about the revolution axis, in degrees.
    Angle,
}
impl ValueKind {
    /// Canonical unit of `ValueEntry::value` (millimetres or degrees).
    pub fn unit(self) -> &'static str {
        match self {
            Self::Distance => "mm",
            Self::Angle => "°",
        }
    }
    pub fn validate(self, value: f64) -> Result<f64, String> {
        if !value.is_finite() {
            return Err("Enter a finite number".into());
        }
        match self {
            Self::Distance if value == 0. => Err(
                "Zero extrusion distance creates no solid; drag or type a nonzero signed distance"
                    .into(),
            ),
            Self::Distance if value.abs() > 1e9 => {
                Err("Distance must be no greater than 1,000,000,000 mm".into())
            }
            Self::Angle if value == 0. => Err(
                "Zero revolution angle creates no solid; drag or type a nonzero signed angle"
                    .into(),
            ),
            Self::Angle if value.abs() > 360. => {
                Err("Revolution angle must be no greater than one full turn (360°)".into())
            }
            _ => Ok(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntryInput {
    /// Primary press on the handle at this pointer coordinate (axis parameter
    /// in mm, or unwrapped angle in degrees).
    Press(f64),
    /// Pointer motion at this coordinate in the same frame as `Press`.
    Drag(f64),
    Release,
    /// Click on the adjacent numeric field.
    Focus,
    Char(char),
    Backspace,
    Enter,
    Escape,
}
#[derive(Clone, Debug, PartialEq)]
pub enum EntryEffect {
    None,
    /// A valid value the controller should preview natively.
    Preview(f64),
    /// The displayed value cannot be previewed or committed.
    Invalid(String),
    /// Enter accepted this exact value; commit once after its preview.
    Commit(f64),
    /// Leave the tool without changing the document.
    Cancel,
}

#[derive(Clone, Debug)]
pub struct ValueEntry {
    kind: ValueKind,
    /// Display/typing unit for distances, fixed when the entry opens
    /// (CAD-187). `value` itself always stays in millimetres.
    length: crate::preferences::LengthUnit,
    value: f64,
    drag: Option<(f64, f64)>,
    text: Option<String>,
    replace_text: bool,
    committed: bool,
}
impl ValueEntry {
    pub fn new(kind: ValueKind, value: f64) -> Self {
        Self {
            kind,
            length: crate::preferences::length_unit(),
            value,
            drag: None,
            text: None,
            replace_text: false,
            committed: false,
        }
    }
    pub fn kind(&self) -> ValueKind {
        self.kind
    }
    pub fn value(&self) -> f64 {
        self.value
    }
    #[cfg(test)]
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn typing(&self) -> bool {
        self.text.is_some()
    }
    #[cfg(test)]
    pub fn committed(&self) -> bool {
        self.committed
    }
    /// The commit failed (invalid geometry or preview error): keep the typed
    /// or dragged value and allow another Enter.
    pub fn reopen(&mut self) {
        self.committed = false;
    }
    /// Inspector edits are adopted only while the gizmo does not own input.
    pub fn sync_external(&mut self, value: f64) {
        if self.drag.is_none() && self.text.is_none() && !self.committed {
            self.value = value;
        }
    }
    /// Unit shown and typed: the Settings length unit for distances.
    pub fn unit(&self) -> &'static str {
        match self.kind {
            ValueKind::Distance => self.length.symbol(),
            ValueKind::Angle => self.kind.unit(),
        }
    }
    fn shown(&self, value: f64) -> f64 {
        match self.kind {
            ValueKind::Distance => self.length.from_mm(value),
            ValueKind::Angle => value,
        }
    }
    fn canonical(&self, typed: f64) -> f64 {
        match self.kind {
            ValueKind::Distance => self.length.to_mm(typed),
            ValueKind::Angle => typed,
        }
    }
    /// Field text including the unit, e.g. `25 mm` or `-12.5|  mm` while typing.
    pub fn display(&self) -> String {
        match &self.text {
            Some(text) => format!("{text}| {}", self.unit()),
            None => format!("{} {}", format_number(self.shown(self.value)), self.unit()),
        }
    }
    fn evaluate(&mut self) -> EntryEffect {
        let Some(text) = &self.text else {
            return match self.kind.validate(self.value) {
                Ok(value) => EntryEffect::Preview(value),
                Err(error) => EntryEffect::Invalid(error),
            };
        };
        match parse(text).and_then(|value| self.kind.validate(self.canonical(value))) {
            Ok(value) => {
                self.value = value;
                EntryEffect::Preview(value)
            }
            Err(error) => EntryEffect::Invalid(error),
        }
    }
    pub fn handle(&mut self, input: EntryInput) -> EntryEffect {
        if self.committed {
            return EntryEffect::None;
        }
        match input {
            EntryInput::Press(pointer) => {
                self.drag = Some((self.value, pointer));
                EntryEffect::None
            }
            EntryInput::Drag(pointer) => {
                let Some((start, origin)) = self.drag else {
                    return EntryEffect::None;
                };
                if self.text.is_some() {
                    // Typing owns the value; the pointer is only a pointer now.
                    return EntryEffect::None;
                }
                let next = start + (pointer - origin);
                if next == self.value {
                    return EntryEffect::None;
                }
                self.value = next;
                self.evaluate()
            }
            EntryInput::Release => {
                self.drag = None;
                EntryEffect::None
            }
            EntryInput::Focus => {
                if self.text.is_none() {
                    self.text = Some(format_number(self.shown(self.value)));
                    self.replace_text = true;
                }
                EntryEffect::None
            }
            EntryInput::Char(c) => {
                let c = if c == ',' { '.' } else { c };
                if !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')) {
                    return EntryEffect::None;
                }
                let text = self.text.get_or_insert_with(String::new);
                if self.replace_text {
                    text.clear();
                    self.replace_text = false;
                }
                text.push(c);
                self.evaluate()
            }
            EntryInput::Backspace => {
                let Some(text) = &mut self.text else {
                    return EntryEffect::None;
                };
                if self.replace_text {
                    text.clear();
                    self.replace_text = false;
                } else {
                    text.pop();
                }
                self.evaluate()
            }
            EntryInput::Enter => match self.evaluate() {
                EntryEffect::Preview(value) => {
                    self.committed = true;
                    self.drag = None;
                    EntryEffect::Commit(value)
                }
                other => other,
            },
            EntryInput::Escape => EntryEffect::Cancel,
        }
    }
}

fn parse(text: &str) -> Result<f64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type a signed number".into());
    }
    text.parse::<f64>()
        .map_err(|_| format!("'{text}' is not a number"))
}
pub fn format_number(value: f64) -> String {
    let rounded = (value * 1000.).round() / 1000.;
    let rounded = if rounded == 0. { 0. } else { rounded };
    format!("{rounded}")
}

/// Unwraps successive `atan2` angles (radians) into a continuous signed sweep
/// in degrees, so a drag past ±180° keeps counting instead of jumping.
#[derive(Clone, Copy, Debug, Default)]
pub struct AngleTracker {
    last: Option<f64>,
    total_deg: f64,
}
impl AngleTracker {
    pub fn start(angle_rad: f64, total_deg: f64) -> Self {
        Self {
            last: Some(angle_rad),
            total_deg,
        }
    }
    pub fn update(&mut self, angle_rad: f64) -> f64 {
        if let Some(last) = self.last {
            let mut delta = angle_rad - last;
            while delta > std::f64::consts::PI {
                delta -= std::f64::consts::TAU;
            }
            while delta < -std::f64::consts::PI {
                delta += std::f64::consts::TAU;
            }
            self.total_deg += delta.to_degrees();
        }
        self.last = Some(angle_rad);
        self.total_deg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inch_preference_shows_and_reads_inches_but_previews_millimetres() {
        use crate::preferences::{LengthUnit, set_length_unit};
        set_length_unit(LengthUnit::Inch);
        let mut entry = ValueEntry::new(ValueKind::Distance, 25.4);
        set_length_unit(LengthUnit::Millimeter);
        assert_eq!(entry.display(), "1 in");
        assert_eq!(entry.handle(EntryInput::Focus), EntryEffect::None);
        assert_eq!(entry.display(), "1| in");
        let mut last = EntryEffect::None;
        for c in "2".chars() {
            last = entry.handle(EntryInput::Char(c));
        }
        assert_eq!(last, EntryEffect::Preview(50.8));
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(50.8));
        // Angles ignore the length preference.
        set_length_unit(LengthUnit::Inch);
        assert_eq!(ValueEntry::new(ValueKind::Angle, 90.).display(), "90 °");
        set_length_unit(LengthUnit::Millimeter);
    }
    fn typed(entry: &mut ValueEntry, text: &str) -> EntryEffect {
        let mut last = EntryEffect::None;
        for c in text.chars() {
            last = entry.handle(EntryInput::Char(c));
        }
        last
    }
    #[test]
    fn drag_then_type_takes_over_and_enter_commits_exactly_once() {
        let mut entry = ValueEntry::new(ValueKind::Distance, 10.);
        assert_eq!(entry.handle(EntryInput::Press(10.)), EntryEffect::None);
        assert_eq!(
            entry.handle(EntryInput::Drag(23.7)),
            EntryEffect::Preview(23.7)
        );
        assert!(entry.dragging());
        // Typing without focusing the field replaces the dragged value.
        assert_eq!(typed(&mut entry, "25"), EntryEffect::Preview(25.));
        assert!(entry.typing());
        // Further pointer motion no longer changes the typed value.
        assert_eq!(entry.handle(EntryInput::Drag(40.)), EntryEffect::None);
        assert_eq!(entry.value(), 25.);
        assert_eq!(entry.display(), "25| mm");
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(25.));
        // Pointer release after Enter, a second Enter and more typing are inert.
        assert_eq!(entry.handle(EntryInput::Release), EntryEffect::None);
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::None);
        assert_eq!(typed(&mut entry, "9"), EntryEffect::None);
        assert_eq!(entry.value(), 25.);
    }
    #[test]
    fn release_never_commits_and_enter_after_release_commits_the_dragged_value() {
        let mut entry = ValueEntry::new(ValueKind::Distance, 10.);
        entry.handle(EntryInput::Press(12.));
        entry.handle(EntryInput::Drag(20.));
        assert_eq!(entry.handle(EntryInput::Release), EntryEffect::None);
        assert!(!entry.committed());
        assert_eq!(entry.value(), 18.);
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(18.));
    }
    #[test]
    fn crossing_zero_is_continuous_signed_and_zero_is_never_committed() {
        let mut entry = ValueEntry::new(ValueKind::Distance, 5.);
        entry.handle(EntryInput::Press(5.));
        assert!(matches!(
            entry.handle(EntryInput::Drag(0.)),
            EntryEffect::Invalid(_)
        ));
        assert_eq!(entry.value(), 0.);
        assert!(matches!(
            entry.handle(EntryInput::Enter),
            EntryEffect::Invalid(_)
        ));
        assert!(!entry.committed());
        assert_eq!(
            entry.handle(EntryInput::Drag(-7.5)),
            EntryEffect::Preview(-7.5)
        );
        entry.handle(EntryInput::Release);
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(-7.5));
    }
    #[test]
    fn typed_negative_and_invalid_text_and_focus_replacement() {
        let mut entry = ValueEntry::new(ValueKind::Distance, 10.);
        entry.handle(EntryInput::Focus);
        assert_eq!(entry.display(), "10| mm");
        // The first character replaces the focused value.
        assert!(matches!(typed(&mut entry, "-"), EntryEffect::Invalid(_)));
        assert_eq!(typed(&mut entry, "25"), EntryEffect::Preview(-25.));
        assert!(matches!(typed(&mut entry, "-"), EntryEffect::Invalid(_)));
        assert!(matches!(
            entry.handle(EntryInput::Enter),
            EntryEffect::Invalid(_)
        ));
        assert_eq!(
            entry.handle(EntryInput::Backspace),
            EntryEffect::Preview(-25.)
        );
        // Letters (including shortcut keys) are ignored, comma is a decimal point.
        assert_eq!(entry.handle(EntryInput::Char('e')), EntryEffect::None);
        assert_eq!(typed(&mut entry, ",5"), EntryEffect::Preview(-25.5));
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(-25.5));
    }
    #[test]
    fn failed_commit_reopens_for_retry_and_escape_cancels() {
        let mut entry = ValueEntry::new(ValueKind::Distance, 10.);
        typed(&mut entry, "25");
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(25.));
        entry.reopen();
        assert_eq!(entry.handle(EntryInput::Enter), EntryEffect::Commit(25.));
        let mut other = ValueEntry::new(ValueKind::Angle, 90.);
        assert_eq!(other.handle(EntryInput::Escape), EntryEffect::Cancel);
    }
    #[test]
    fn angle_limits_and_unwrapped_drag() {
        let mut entry = ValueEntry::new(ValueKind::Angle, 90.);
        assert!(matches!(typed(&mut entry, "400"), EntryEffect::Invalid(_)));
        entry.handle(EntryInput::Backspace);
        entry.handle(EntryInput::Backspace);
        entry.handle(EntryInput::Backspace);
        assert!(matches!(typed(&mut entry, "0"), EntryEffect::Invalid(_)));
        assert_eq!(entry.display(), "0| °");
        let mut tracker = AngleTracker::start(170f64.to_radians(), 0.);
        let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} != {b}");
        near(tracker.update((-170f64).to_radians()), 20.);
        near(tracker.update((-90f64).to_radians()), 100.);
        let mut drag = ValueEntry::new(ValueKind::Angle, 90.);
        drag.handle(EntryInput::Press(0.));
        assert_eq!(
            drag.handle(EntryInput::Drag(-30.)),
            EntryEffect::Preview(60.)
        );
        assert_eq!(typed(&mut drag, "180"), EntryEffect::Preview(180.));
        assert_eq!(drag.handle(EntryInput::Enter), EntryEffect::Commit(180.));
    }
    #[test]
    fn external_inspector_values_sync_only_while_idle() {
        let mut entry = ValueEntry::new(ValueKind::Distance, 10.);
        entry.sync_external(30.);
        assert_eq!(entry.display(), "30 mm");
        entry.handle(EntryInput::Press(0.));
        entry.sync_external(50.);
        assert_eq!(entry.value(), 30.);
    }
}
