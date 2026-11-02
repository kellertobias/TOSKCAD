//! Architecture navigation refers to shared tools and canonical equipment kinds.
use crate::{
    icons::Icon,
    registry::{Command, Tool},
    ribbon_ui::{Entry, Group},
};
use multicad_core::equipment::{Category as EquipmentCategory, Kind};

pub const PARENT_CUE_HEIGHT: f32 = 24.;
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
pub fn tools() -> Vec<Tool> {
    let mut tools: Vec<_> = PRESETS
        .iter()
        .map(|p| Tool {
            id: p.id,
            label: p.label,
            icon: Category::for_equipment(p.kind).icon(),
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
    let mut groups = vec![group("", &["architecture-back"])];
    groups.extend(match category {
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
    });
    groups
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
    ids.push("equipment");
    ids
}

/// The real parent controls move into this narrow row while child tools use
/// the full strip. Back/Close return context without invoking a modeling tool.
pub fn parent_cue(
    ui: &mut crate::egui::Ui,
    registry: &crate::registry::Registry,
    category: Category,
    enabled: impl Fn(Command) -> bool,
) -> Option<Command> {
    use crate::egui;
    let mut action = None;
    ui.push_id("architecture-parent-cue",|ui| {
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),PARENT_CUE_HEIGHT),egui::Layout::left_to_right(egui::Align::Center),|ui| {
            ui.spacing_mut().item_spacing=egui::vec2(4.,0.);
            ui.strong(egui::RichText::new("Architecture").small());
            for group in groups(None) { for entry in group.entries { let Entry::Tool(id)=entry else {continue}; let tool=registry.tool(id).expect("Architecture parent tool");
                if ui.add_enabled(enabled(tool.action),egui::Button::new(tool.label).small().selected(tool.action==Command::ArchitectureCategory(category))).clicked() {action=Some(tool.action);}
            }}
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                if ui.small_button("Close").on_hover_text("Close this category and return to Architecture; keep the current operation open").clicked() {action=Some(Command::ArchitectureParent);}
            });
        });
    });
    action
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
        let mut offered = std::collections::BTreeSet::new();
        for category in Category::ALL {
            for group in groups(Some(category)) {
                for entry in group.entries {
                    let Entry::Tool(id) = entry else {
                        panic!("explicit category tool")
                    };
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
        assert_eq!(electrical[1].label, "Connections");
        let required: Vec<_> = electrical[1]
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
    }
}
