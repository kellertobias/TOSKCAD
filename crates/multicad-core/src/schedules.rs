//! Which schedule a drawing table or a CSV output is built from. The rows
//! themselves come from the native model (`multicad_model::schedules`); this
//! source-level choice is what a placed table, an export recipe and a direct
//! CSV export persist or request.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    Bom,
    PanelCutList,
    Machining,
    MemberCutList,
    WeldSchedule,
    BendSchedule,
}
impl ScheduleKind {
    pub const ALL: [Self; 6] = [
        Self::Bom,
        Self::PanelCutList,
        Self::Machining,
        Self::MemberCutList,
        Self::WeldSchedule,
        Self::BendSchedule,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bom => "Bill of materials",
            Self::PanelCutList => "Panel cut list",
            Self::Machining => "Machining",
            Self::MemberCutList => "Member cut list",
            Self::WeldSchedule => "Weld schedule",
            Self::BendSchedule => "Bend schedule",
        }
    }
    /// The snake_case kind, as written in CSV headers and file names.
    pub fn token(self) -> &'static str {
        match self {
            Self::Bom => "bom",
            Self::PanelCutList => "panel_cut_list",
            Self::Machining => "machining",
            Self::MemberCutList => "member_cut_list",
            Self::WeldSchedule => "weld_schedule",
            Self::BendSchedule => "bend_schedule",
        }
    }
}
fn yes() -> bool {
    true
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_true(v: &bool) -> bool {
    *v
}
/// A schedule and its inclusion options. The BOM options are ignored by
/// the other kinds and normalized away by [`ScheduleSpec::normalized`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSpec {
    pub kind: ScheduleKind,
    /// BOM only: include FDM aid Parts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_aids: bool,
    /// BOM only: include purchased and imported Parts.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub include_purchased: bool,
}
impl ScheduleSpec {
    pub fn new(kind: ScheduleKind) -> Self {
        Self {
            kind,
            include_aids: false,
            include_purchased: true,
        }
    }
    pub fn normalized(self) -> Self {
        if self.kind == ScheduleKind::Bom {
            self
        } else {
            Self::new(self.kind)
        }
    }
    /// Every distinct normalized spec: six kinds, four BOM variants.
    pub fn all() -> Vec<Self> {
        let mut all = vec![];
        for kind in ScheduleKind::ALL {
            if kind == ScheduleKind::Bom {
                for include_aids in [false, true] {
                    for include_purchased in [true, false] {
                        all.push(Self {
                            kind,
                            include_aids,
                            include_purchased,
                        });
                    }
                }
            } else {
                all.push(Self::new(kind));
            }
        }
        all
    }
    /// A compact bit index (< 24) for batching several specs in one request.
    pub fn bit(self) -> u32 {
        let s = self.normalized();
        let kind = ScheduleKind::ALL
            .iter()
            .position(|k| *k == s.kind)
            .expect("known kind") as u32;
        kind * 4 + u32::from(s.include_aids) * 2 + u32::from(!s.include_purchased)
    }
    pub fn from_bit(bit: u32) -> Option<Self> {
        let kind = *ScheduleKind::ALL.get((bit / 4) as usize)?;
        let spec = Self {
            kind,
            include_aids: bit & 2 != 0,
            include_purchased: bit & 1 == 0,
        };
        (spec.normalized() == spec).then_some(spec)
    }
    /// A batch of specs as a bit mask, and back.
    pub fn mask(specs: impl IntoIterator<Item = Self>) -> u64 {
        specs.into_iter().fold(0, |m, s| m | 1 << s.bit())
    }
    pub fn from_mask(mask: u64) -> Vec<Self> {
        (0..24)
            .filter(|b| mask & (1 << b) != 0)
            .filter_map(Self::from_bit)
            .collect()
    }
    /// The display label with the BOM options that differ from the default.
    pub fn label(self) -> String {
        let s = self.normalized();
        let mut label = s.kind.label().to_owned();
        if s.kind == ScheduleKind::Bom {
            if s.include_aids {
                label.push_str(" · with FDM aids");
            }
            if !s.include_purchased {
                label.push_str(" · manufactured only");
            }
        }
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn specs_round_trip_through_bits_masks_and_json() {
        let all = ScheduleSpec::all();
        assert_eq!(all.len(), 9);
        for spec in &all {
            assert_eq!(ScheduleSpec::from_bit(spec.bit()), Some(*spec));
            let json = serde_json::to_string(spec).unwrap();
            assert_eq!(serde_json::from_str::<ScheduleSpec>(&json).unwrap(), *spec);
        }
        assert_eq!(
            ScheduleSpec::from_mask(ScheduleSpec::mask(all.clone())),
            all
        );
        // Non-BOM options are normalized; unknown bits are ignored.
        let odd = ScheduleSpec {
            kind: ScheduleKind::Machining,
            include_aids: true,
            include_purchased: false,
        };
        assert_eq!(odd.bit(), ScheduleSpec::new(ScheduleKind::Machining).bit());
        assert_eq!(ScheduleSpec::from_bit(5 * 4 + 1), None);
        assert_eq!(
            serde_json::to_string(&ScheduleSpec::new(ScheduleKind::Bom)).unwrap(),
            r#"{"kind":"bom"}"#
        );
        assert!(serde_json::from_str::<ScheduleSpec>(r#"{"kind":"bom","x":1}"#).is_err());
    }
}
