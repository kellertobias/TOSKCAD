//! Architecture navigation refers to shared tools and canonical equipment kinds.
use crate::{
    icons::Icon,
    registry::{Command, Tool},
    ribbon_ui::{Entry, Group},
};
use multicad_core::equipment::{Category as EquipmentCategory, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Water,
    Electrical,
    Heating,
    Furniture,
    Landscaping,
}
impl Category {
    pub const ALL: [Self; 5] = [
        Self::Water,
        Self::Electrical,
        Self::Heating,
        Self::Furniture,
        Self::Landscaping,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Water => "Water",
            Self::Electrical => "Electrical",
            Self::Heating => "Heating",
            Self::Furniture => "Furniture",
            Self::Landscaping => "Landscaping",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Water => "architecture-water",
            Self::Electrical => "architecture-electrical",
            Self::Heating => "architecture-heating",
            Self::Furniture => "architecture-furniture",
            Self::Landscaping => "architecture-landscaping",
        }
    }
    pub fn workspace(self) -> &'static str {
        match self {
            Self::Water => "Architecture / Water",
            Self::Electrical => "Architecture / Electrical",
            Self::Heating => "Architecture / Heating",
            Self::Furniture => "Architecture / Furniture",
            Self::Landscaping => "Architecture / Landscaping",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Water => Icon::Water,
            Self::Electrical => Icon::Electrical,
            Self::Heating => Icon::Heating,
            Self::Furniture => Icon::Furniture,
            Self::Landscaping => Icon::Plant,
        }
    }
    pub fn from_workspace(workspace: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.workspace() == workspace)
    }
    pub(crate) fn adjacent(self, forward: bool) -> Option<Self> {
        let index = Self::ALL.iter().position(|category| *category == self)?;
        let next = if forward {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        }?;
        Self::ALL.get(next).copied()
    }
    pub fn contains(self, kind: Kind) -> bool {
        match self {
            Self::Water => {
                kind.preset().category == EquipmentCategory::Sanitary
                    || matches!(kind, Kind::WashingMachine | Kind::Dishwasher)
            }
            Self::Electrical => kind.preset().category == EquipmentCategory::ElectricalLighting,
            Self::Heating => kind.preset().category == EquipmentCategory::Heating,
            Self::Furniture => kind.preset().category == EquipmentCategory::Appliances,
            Self::Landscaping => false,
        }
    }
    pub fn for_equipment(kind: Kind) -> Self {
        match kind.preset().category {
            EquipmentCategory::ElectricalLighting => Self::Electrical,
            EquipmentCategory::Heating => Self::Heating,
            EquipmentCategory::Sanitary => Self::Water,
            EquipmentCategory::Appliances => Self::Furniture,
        }
    }
}
struct PresetTool {
    kind: Kind,
    id: &'static str,
    label: &'static str,
    aliases: &'static [&'static str],
}
macro_rules! presets { ($(($kind:ident,$slug:literal,$label:literal,[$($alias:literal),*])),+ $(,)?)=>{ const PRESETS: &[PresetTool] = &[$(PresetTool{kind:Kind::$kind,id:concat!("equipment-",$slug),label:$label,aliases:&[$($alias),*]}),+]; }; }
presets!(
    (
        PowerOutletTypeF,
        "power-outlet-type-f",
        "Socket",
        ["socket", "outlet", "power-outlet"]
    ),
    (
        DoublePowerOutletTypeF,
        "double-power-outlet-type-f",
        "Double socket",
        ["double-socket"]
    ),
    (
        TriplePowerOutletTypeF,
        "triple-power-outlet-type-f",
        "Triple socket",
        ["triple-socket"]
    ),
    (
        DataOutletRj45,
        "data-outlet-rj45",
        "Data socket",
        ["data-outlet", "rj45"]
    ),
    (
        TvAntennaOutlet,
        "tv-antenna-outlet",
        "TV socket",
        ["tv-socket"]
    ),
    (Switch, "switch", "Switch", ["switch", "wall-switch"]),
    (
        CeilingLuminaire,
        "ceiling-luminaire",
        "Ceiling light",
        ["ceiling-light"]
    ),
    (
        WallLuminaire,
        "wall-luminaire",
        "Wall light",
        ["wall-light"]
    ),
    (
        LampHolderE27,
        "lamp-holder-e27",
        "Lamp holder",
        ["lamp-holder", "e27"]
    ),
    (Radiator, "radiator", "Radiator", ["radiator"]),
    (
        KitchenSink,
        "kitchen-sink",
        "Kitchen sink",
        ["kitchen-sink", "sink"]
    ),
    (Washbasin, "washbasin", "Washbasin", ["washbasin"]),
    (Toilet, "toilet", "Toilet", ["toilet"]),
    (Bathtub, "bathtub", "Bathtub", ["bathtub"]),
    (Shower, "shower", "Shower", ["shower"]),
    (
        CookerOven,
        "cooker-oven",
        "Cooker / oven",
        ["cooker", "oven"]
    ),
    (Cooktop, "cooktop", "Cooktop", ["cooktop"]),
    (
        WashingMachine,
        "washing-machine",
        "Washing machine",
        ["washing-machine"]
    ),
    (Dryer, "dryer", "Dryer", ["dryer"]),
    (
        Refrigerator,
        "refrigerator",
        "Refrigerator",
        ["refrigerator", "fridge"]
    ),
    (Dishwasher, "dishwasher", "Dishwasher", ["dishwasher"]),
    (
        JunctionBox,
        "junction-box",
        "Junction box",
        ["junction-box"]
    ),
);
fn equipment_icon(kind: Kind) -> Icon {
    match kind {
        Kind::CeilingLuminaire | Kind::WallLuminaire | Kind::LampHolderE27 => Icon::Light,
        Kind::JunctionBox => Icon::Box,
        _ if kind.preset().category == EquipmentCategory::Appliances => Icon::Appliance,
        _ => Category::for_equipment(kind).icon(),
    }
}
pub fn tools() -> Vec<Tool> {
    let mut tools: Vec<_> = PRESETS
        .iter()
        .map(|p| Tool {
            id: p.id,
            label: p.label,
            icon: equipment_icon(p.kind),
            action: Command::EquipmentPreset(p.kind),
            aliases: p.aliases,
        })
        .collect();
    tools.extend(Category::ALL.map(|category| Tool {
        id: category.id(),
        label: category.label(),
        icon: category.icon(),
        action: Command::ArchitectureCategory(category),
        aliases: match category {
            Category::Water => &["water", "plumbing"],
            Category::Electrical => &["electrical", "lighting", "light"],
            Category::Heating => &["heating"],
            Category::Furniture => &["furniture"],
            Category::Landscaping => &["landscaping"],
        },
    }));
    tools.push(Tool {
        id: "architecture-back",
        label: "Back",
        icon: Icon::Back,
        action: Command::ArchitectureParent,
        aliases: &["back-to-architecture", "close-category"],
    });
    tools
}
fn group(label: &'static str, ids: &[&'static str]) -> Group {
    Group {
        label,
        entries: ids.iter().map(|id| Entry::Tool(id)).collect(),
    }
}
pub fn groups(category: Option<Category>) -> Vec<Group> {
    let Some(category) = category else {
        return vec![
            group(
                "Create",
                &[
                    "wall",
                    "door",
                    "window-tool",
                    "vertical-cutout",
                    "level",
                    "stairs",
                ],
            ),
            group(
                "Place",
                &[
                    "architecture-water",
                    "architecture-electrical",
                    "architecture-heating",
                    "architecture-furniture",
                    "architecture-landscaping",
                ],
            ),
        ];
    };
    match category {
        Category::Water => vec![
            group(
                "Fixtures",
                &[
                    "equipment-kitchen-sink",
                    "equipment-washbasin",
                    "equipment-toilet",
                    "equipment-bathtub",
                    "equipment-shower",
                ],
            ),
            group(
                "Appliances",
                &["equipment-washing-machine", "equipment-dishwasher"],
            ),
            group("Library", &["library"]),
        ],
        Category::Electrical => vec![
            group(
                "Connections",
                &[
                    "equipment-switch",
                    "equipment-power-outlet-type-f",
                    "equipment-junction-box",
                    "equipment-double-power-outlet-type-f",
                    "equipment-triple-power-outlet-type-f",
                    "equipment-data-outlet-rj45",
                    "equipment-tv-antenna-outlet",
                ],
            ),
            group(
                "Lighting",
                &[
                    "equipment-ceiling-luminaire",
                    "equipment-wall-luminaire",
                    "equipment-lamp-holder-e27",
                ],
            ),
        ],
        Category::Heating => vec![
            group("Heating", &["equipment-radiator"]),
            group("Library", &["library"]),
        ],
        Category::Furniture => vec![
            group("Create", &["cabinet", "panel"]),
            group(
                "Appliances",
                &[
                    "equipment-cooker-oven",
                    "equipment-cooktop",
                    "equipment-washing-machine",
                    "equipment-dryer",
                    "equipment-refrigerator",
                    "equipment-dishwasher",
                ],
            ),
            group("Library", &["library"]),
        ],
        Category::Landscaping => vec![
            group("Create", &["plant", "fence"]),
            group("Library", &["library"]),
        ],
    }
}
/// The shared registry retains every real command for search and workbench routing.
pub fn tab_tools() -> Vec<&'static str> {
    let mut ids = vec![];
    for category in std::iter::once(None).chain(Category::ALL.into_iter().map(Some)) {
        for group in groups(category) {
            for entry in group.entries {
                if let Entry::Tool(id) = entry
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
        }
    }
    ids.extend(["architecture-back", "equipment"]);
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{modeling::ModelingTool, registry::Registry};
    #[test]
    fn architecture_groups_expose_real_commands_and_every_canonical_equipment_family() {
        let registry = Registry::default();
        let root = groups(None);
        let ids: Vec<_> = root[0]
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Tool(id) => *id,
                _ => panic!("explicit creation command"),
            })
            .collect();
        assert_eq!(
            ids,
            [
                "wall",
                "door",
                "window-tool",
                "vertical-cutout",
                "level",
                "stairs"
            ]
        );
        assert_eq!(
            root[1]
                .entries
                .iter()
                .map(|entry| match entry {
                    Entry::Tool(id) => *id,
                    _ => panic!("explicit category launcher"),
                })
                .collect::<Vec<_>>(),
            Category::ALL.map(Category::id)
        );
        let mut offered = std::collections::BTreeSet::new();
        for category in Category::ALL {
            for group in groups(Some(category)) {
                for entry in group.entries {
                    let Entry::Tool(id) = entry else {
                        panic!("explicit category tool")
                    };
                    assert_ne!(
                        id, "architecture-back",
                        "category navigation uses the tab row"
                    );
                    let tool = registry.tool(id).expect("live catalog command");
                    if let Command::EquipmentPreset(kind) = tool.action {
                        assert!(category.contains(kind));
                        offered.insert(kind);
                    }
                }
            }
        }
        assert_eq!(offered, Kind::ALL.into_iter().collect());
        let electrical = groups(Some(Category::Electrical));
        assert_eq!(electrical[0].label, "Connections");
        let required: Vec<_> = electrical[0]
            .entries
            .iter()
            .take(3)
            .map(|entry| match entry {
                Entry::Tool(id) => registry.tool(id).unwrap().label,
                _ => panic!(),
            })
            .collect();
        assert_eq!(required, ["Switch", "Socket", "Junction box"]);
        assert!(
            groups(Some(Category::Furniture))
                .iter()
                .any(|g| g.label == "Appliances")
        );
        for category in Category::ALL {
            assert_eq!(
                Category::from_workspace(category.workspace()),
                Some(category)
            );
        }
    }
    #[test]
    fn category_keyboard_order_reaches_the_parent_at_both_ends() {
        assert_eq!(Category::Water.adjacent(false), None);
        for pair in Category::ALL.windows(2) {
            assert_eq!(pair[0].adjacent(true), Some(pair[1]));
            assert_eq!(pair[1].adjacent(false), Some(pair[0]));
        }
        assert_eq!(Category::Landscaping.adjacent(true), None);
    }

    #[test]
    fn finder_aliases_choose_specific_families_and_keep_generic_and_legacy_level_commands() {
        let registry = Registry::default();
        for (name, kind) in [
            ("switch", Kind::Switch),
            ("socket", Kind::PowerOutletTypeF),
            ("junction-box", Kind::JunctionBox),
            ("radiator", Kind::Radiator),
            ("sink", Kind::KitchenSink),
            ("fridge", Kind::Refrigerator),
        ] {
            assert_eq!(
                registry.parse_line(name).unwrap().0,
                Command::EquipmentPreset(kind)
            );
        }
        assert_eq!(
            registry.parse_line("washer").unwrap().0,
            Command::Modeling(ModelingTool::MechanicalPart)
        );
        assert_eq!(
            registry.parse_line("equipment").unwrap().0,
            Command::Modeling(ModelingTool::Equipment)
        );
        assert_eq!(
            registry.parse_line("stairs").unwrap().0,
            Command::Modeling(ModelingTool::Stair)
        );
        assert_eq!(
            registry.parse_line("slab").unwrap().0,
            Command::Modeling(ModelingTool::Level)
        );
        assert_eq!(
            registry.parse_line("electrical").unwrap().0,
            Command::ArchitectureCategory(Category::Electrical)
        );
        assert!(tab_tools().contains(&"architecture-back"));
        for alias in [
            "architecture-back",
            "back-to-architecture",
            "close-category",
        ] {
            assert_eq!(
                registry.parse_line(alias).unwrap().0,
                Command::ArchitectureParent
            );
        }
    }
}
