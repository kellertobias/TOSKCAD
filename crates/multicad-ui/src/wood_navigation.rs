//! Wood command intents reuse the existing native configuration workflows.
use crate::{
    icons::Icon,
    modeling::ModelingTool,
    registry::{Command, Tool},
    ribbon_ui::{Entry, Group},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Batten,
    Drill,
    Pocket,
    Hardware,
}
impl Preset {
    pub const ALL: [Self; 4] = [Self::Batten, Self::Drill, Self::Pocket, Self::Hardware];
    pub fn id(self) -> &'static str {
        match self {
            Self::Batten => "batten",
            Self::Drill => "drill",
            Self::Pocket => "pocket",
            Self::Hardware => "wood-hardware",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Batten => "Batten",
            Self::Drill => "Drill",
            Self::Pocket => "Pocket",
            Self::Hardware => "Screw / Hardware",
        }
    }
    pub fn tool(self) -> ModelingTool {
        match self {
            Self::Batten => ModelingTool::Member,
            Self::Drill | Self::Pocket => ModelingTool::Machining,
            Self::Hardware => ModelingTool::MechanicalPart,
        }
    }
}
pub fn tools() -> Vec<Tool> {
    Preset::ALL
        .into_iter()
        .map(|preset| Tool {
            id: preset.id(),
            label: if preset == Preset::Hardware {
                "Configure screw / hardware"
            } else {
                preset.label()
            },
            icon: preset.tool().icon(),
            action: Command::WoodPreset(preset),
            aliases: match preset {
                Preset::Batten => &["timber", "latte", "wood-batten"],
                Preset::Drill => &["drilling", "panel-drill"],
                Preset::Pocket => &["panel-pocket"],
                Preset::Hardware => &["wood-hardware", "screw-hardware"],
            },
        })
        .chain([
            Tool {
                id: "wood-library",
                label: "Browse Library",
                icon: Icon::Library,
                action: Command::Extension("library"),
                aliases: &[],
            },
            Tool {
                id: "wood-joint",
                label: "Joint",
                icon: Icon::Link,
                action: Command::Modeling(ModelingTool::Joint),
                aliases: &[],
            },
            Tool {
                id: "wood-cabinet",
                label: "Cabinet",
                icon: Icon::Box,
                action: Command::Modeling(ModelingTool::Cabinet),
                aliases: &[],
            },
        ])
        .collect()
}
pub fn tab_tools() -> Vec<&'static str> {
    vec![
        "panel",
        "batten",
        "wood-hardware",
        "wood-library",
        "screw-connection",
        "drill",
        "pocket",
        "wood-joint",
        "system32",
        "wood-cabinet",
        "wood-box",
        "cut-list",
        "cabinet",
        "joint",
        "machining",
        "domino",
    ]
}
pub fn groups() -> Vec<Group> {
    [
        ("Create", vec!["panel", "batten", "wood-hardware"]),
        ("Modify", vec!["drill", "pocket", "wood-joint"]),
        ("Assistants", vec!["system32", "wood-cabinet", "wood-box"]),
        ("Output", vec!["cut-list"]),
    ]
    .into_iter()
    .map(|(label, tools)| Group {
        label,
        entries: tools
            .into_iter()
            .map(|id| {
                if id == "wood-hardware" {
                    Entry::Menu {
                        label: "Screw / Hardware",
                        icon: Icon::Revolve,
                        tools: vec!["wood-hardware", "wood-library", "screw-connection"],
                        unavailable: None,
                    }
                } else {
                    Entry::Tool(id)
                }
            })
            .collect(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Mode, Registry};

    #[test]
    fn wood_groups_expose_real_workflows_without_a_peer_domino_tool() {
        let registry = Registry::default();
        let groups = groups();
        assert_eq!(
            groups.iter().map(|group| group.label).collect::<Vec<_>>(),
            ["Create", "Modify", "Assistants", "Output"]
        );
        let ids = groups
            .iter()
            .flat_map(|group| group.entries.iter())
            .flat_map(|entry| match entry {
                Entry::Tool(id) => vec![*id],
                Entry::Menu {
                    tools, unavailable, ..
                } => {
                    assert!(unavailable.is_none());
                    tools.clone()
                }
            })
            .collect::<Vec<_>>();
        assert!(!ids.contains(&"domino"));
        for id in ids {
            assert!(registry.tool(id).is_some(), "{id}");
        }
        for preset in Preset::ALL {
            assert_eq!(
                registry.resolve(preset.id()).unwrap().action,
                Command::WoodPreset(preset)
            );
            assert_eq!(
                registry.location(Command::WoodPreset(preset)),
                Some((Mode::Model, "Wood"))
            );
        }
        assert_eq!(
            registry.resolve("pocket").unwrap().action,
            Command::WoodPreset(Preset::Pocket)
        );
        assert_eq!(
            registry.resolve("drill").unwrap().action,
            Command::WoodPreset(Preset::Drill)
        );
        assert_eq!(
            registry.resolve("groove").unwrap().action,
            Command::Modeling(ModelingTool::Machining)
        );
        assert_eq!(
            registry.resolve("domino").unwrap().action,
            Command::Modeling(ModelingTool::LooseTenon)
        );
    }
}
