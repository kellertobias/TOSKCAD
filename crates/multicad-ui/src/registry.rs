//! Declarative workspace/ribbon catalog shared by buttons and command entry.
use crate::icons::Icon;
use eframe::egui::Color32;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Parameters,
    Sketch,
    Model,
    Drawings,
    Export,
}
impl Mode {
    pub const ALL: [Self; 5] = [
        Self::Parameters,
        Self::Sketch,
        Self::Model,
        Self::Drawings,
        Self::Export,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Parameters => "Parameters",
            Self::Sketch => "Sketch",
            Self::Model => "Model",
            Self::Drawings => "Drawings",
            Self::Export => "Export",
        }
    }
    pub fn color(self) -> Color32 {
        match self {
            Self::Parameters => Color32::from_rgb(108, 73, 160),
            Self::Sketch => Color32::from_rgb(28, 113, 119),
            Self::Model => Color32::from_rgb(34, 98, 171),
            Self::Drawings => Color32::from_rgb(172, 91, 18),
            Self::Export => Color32::from_rgb(53, 114, 65),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    KeyboardShortcuts,
    Parameter,
    Apply,
    Discard,
    Preview,
    Undo,
    Redo,
    Save,
    SaveAs,
    ImportSource,
    RecoverProjects,
    RecoverySnapshot,
    Open,
    New,
    NewWindow,
    NewSketch,
    Rectangle,
    Line,
    Dimension,
    FinishSketch,
    Delete,
    Mode(Mode),
    Extension(&'static str),
}
#[derive(Clone, Copy)]
pub struct Tool {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: Icon,
    pub action: Command,
    pub aliases: &'static [&'static str],
}
#[derive(Clone)]
pub struct RibbonTab {
    pub title: &'static str,
    pub tools: Vec<&'static str>,
}
#[derive(Clone)]
pub struct Workspace {
    pub mode: Mode,
    pub tabs: Vec<RibbonTab>,
}
pub struct Registry {
    pub tools: Vec<Tool>,
    pub workspaces: Vec<Workspace>,
}
impl Default for Registry {
    fn default() -> Self {
        let tools = vec![
            Tool {
                id: "shortcuts",
                label: "Keyboard shortcuts",
                icon: Icon::Keyboard,
                action: Command::KeyboardShortcuts,
                aliases: &["shortcuts", "keys"],
            },
            Tool {
                id: "parameter",
                label: "Set parameter",
                icon: Icon::New,
                action: Command::Parameter,
                aliases: &["param"],
            },
            Tool {
                id: "apply",
                label: "Apply and rebuild",
                icon: Icon::Save,
                action: Command::Apply,
                aliases: &["apply", "rebuild"],
            },
            Tool {
                id: "discard",
                label: "Discard edits",
                icon: Icon::Undo,
                action: Command::Discard,
                aliases: &["discard", "cancel"],
            },
            Tool {
                id: "preview",
                label: "Preview",
                icon: Icon::Box,
                action: Command::Preview,
                aliases: &["preview"],
            },
            Tool {
                id: "undo",
                label: "Undo",
                icon: Icon::Undo,
                action: Command::Undo,
                aliases: &["undo"],
            },
            Tool {
                id: "redo",
                label: "Redo",
                icon: Icon::Redo,
                action: Command::Redo,
                aliases: &["redo"],
            },
            Tool {
                id: "save",
                label: "Save",
                icon: Icon::Save,
                action: Command::Save,
                aliases: &["save"],
            },
            Tool {
                id: "save-as",
                label: "Save project as",
                icon: Icon::Save,
                action: Command::SaveAs,
                aliases: &["saveas"],
            },
            Tool {
                id: "import-source",
                label: "Import source snapshot",
                icon: Icon::Page,
                action: Command::ImportSource,
                aliases: &["import"],
            },
            Tool {
                id: "recover-projects",
                label: "Recover projects",
                icon: Icon::Page,
                action: Command::RecoverProjects,
                aliases: &["recover"],
            },
            Tool {
                id: "recovery-snapshot",
                label: "Create recovery snapshot",
                icon: Icon::Save,
                action: Command::RecoverySnapshot,
                aliases: &["checkpoint"],
            },
            Tool {
                id: "open",
                label: "Open",
                icon: Icon::Page,
                action: Command::Open,
                aliases: &["open"],
            },
            Tool {
                id: "new",
                label: "New document",
                icon: Icon::New,
                action: Command::New,
                aliases: &["new"],
            },
            Tool {
                id: "new-window",
                label: "New window",
                icon: Icon::New,
                action: Command::NewWindow,
                aliases: &["window"],
            },
            Tool {
                id: "new-sketch",
                label: "New sketch",
                icon: Icon::Sketch,
                action: Command::NewSketch,
                aliases: &["start", "new-sketch"],
            },
            Tool {
                id: "rectangle",
                label: "Rectangle",
                icon: Icon::Rectangle,
                action: Command::Rectangle,
                aliases: &["rect", "rectangle"],
            },
            Tool {
                id: "line",
                label: "Line",
                icon: Icon::Line,
                action: Command::Line,
                aliases: &["line"],
            },
            Tool {
                id: "finish-sketch",
                label: "Finish sketch",
                icon: Icon::Save,
                action: Command::FinishSketch,
                aliases: &["finish"],
            },
            Tool {
                id: "delete",
                label: "Delete",
                icon: Icon::Cut,
                action: Command::Delete,
                aliases: &["delete"],
            },
            Tool {
                id: "dimension",
                label: "Dimension",
                icon: Icon::Measure,
                action: Command::Dimension,
                aliases: &["dim"],
            },
        ];
        let mut registry = Self {
            tools,
            workspaces: vec![],
        };
        for mode in Mode::ALL {
            registry.tools.push(Tool {
                id: mode.title(),
                label: mode.title(),
                icon: Icon::Page,
                action: Command::Mode(mode),
                aliases: match mode {
                    Mode::Parameters => &["parameters", "params"],
                    Mode::Sketch => &["sketch"],
                    Mode::Model => &["model"],
                    Mode::Drawings => &["drawings"],
                    Mode::Export => &["export"],
                },
            });
        }
        for (id, label, icon) in [
            ("extrude", "Extrude", Icon::Box),
            ("revolve", "Revolve", Icon::Box),
            ("boolean", "Boolean", Icon::Cut),
            ("edge", "Fillet / chamfer", Icon::Edge),
            ("move", "Move / rotate", Icon::Link),
            ("symbol", "Create symbol", Icon::Library),
            ("panel", "Panel", Icon::Box),
            ("joint", "Finger / miter joint", Icon::Link),
            ("drill", "Drill / pocket", Icon::Cut),
            ("cut-list", "Cut list", Icon::Page),
            ("sheet", "Base sheet", Icon::Page),
            ("bend", "Flange / bend", Icon::Edge),
            ("fastener", "Attached fastener", Icon::Link),
            ("flat", "Flat pattern", Icon::Page),
            ("bend-list", "Bend schedule", Icon::Page),
            ("tube", "Tube / profile", Icon::Box),
            ("cope", "Miter / cope", Icon::Cut),
            ("weld", "Weld joint", Icon::Link),
            ("weld-list", "Weld sequence", Icon::Page),
            ("transfer", "Transfer rail holes", Icon::Link),
            ("align", "Align / offset", Icon::Measure),
            ("support", "Thin support", Icon::Box),
            ("tab", "Breakaway tab", Icon::Edge),
            ("materials", "Materials", Icon::Library),
            ("wall", "Wall", Icon::Box),
            ("door", "Door", Icon::Page),
            ("window-tool", "Window", Icon::Page),
            ("stairs", "Stairs", Icon::Edge),
            ("sheet-page", "Add page", Icon::New),
            ("page-setup", "Page setup", Icon::Settings),
            ("projection", "Projection / section", Icon::Page),
            ("text", "Text / leader", Icon::Sketch),
            ("export-set", "Tag export set", Icon::Library),
            ("export-3d", "STEP / OBJ", Icon::Export),
            ("export-2d", "DXF / PDF", Icon::Page),
            ("print", "3MF package", Icon::Export),
            ("appearance", "Render scene", Icon::Box),
            ("blender", "Blender handoff", Icon::Export),
        ] {
            registry.tools.push(Tool {
                id,
                label,
                icon,
                action: Command::Extension(id),
                aliases: &[],
            });
        }
        let tab = |title, tools: &[&'static str]| RibbonTab {
            title,
            tools: tools.to_vec(),
        };
        registry.workspaces = vec![
            Workspace {
                mode: Mode::Parameters,
                tabs: vec![tab(
                    "Common",
                    &["apply", "discard", "preview", "undo", "redo"],
                )],
            },
            Workspace {
                mode: Mode::Sketch,
                tabs: vec![
                    tab(
                        "Common",
                        &[
                            "new-sketch",
                            "rectangle",
                            "line",
                            "dimension",
                            "delete",
                            "finish-sketch",
                        ],
                    ),
                    tab("Room", &["wall", "door", "window-tool", "stairs"]),
                ],
            },
            Workspace {
                mode: Mode::Model,
                tabs: vec![
                    tab(
                        "Common",
                        &["extrude", "revolve", "boolean", "edge", "move", "symbol"],
                    ),
                    tab("Wood", &["panel", "drill", "joint", "cut-list"]),
                    tab(
                        "Sheet metal",
                        &["sheet", "bend", "fastener", "flat", "bend-list"],
                    ),
                    tab("Weld", &["tube", "cope", "weld", "weld-list"]),
                    tab("Relationships", &["transfer", "align"]),
                    tab("FDM aids", &["support", "tab"]),
                    tab("Materials", &["materials"]),
                ],
            },
            Workspace {
                mode: Mode::Drawings,
                tabs: vec![
                    tab("Common", &["sheet-page", "page-setup", "projection"]),
                    tab("Annotate", &["dimension", "text"]),
                ],
            },
            Workspace {
                mode: Mode::Export,
                tabs: vec![
                    tab("Common", &["export-set", "export-3d", "export-2d"]),
                    tab("Print", &["print"]),
                    tab("Render", &["appearance", "blender"]),
                ],
            },
        ];
        registry
    }
}
impl Registry {
    pub fn tool(&self, id: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.id == id)
    }
    pub fn resolve(&self, name: &str) -> Option<&Tool> {
        self.tools
            .iter()
            .find(|t| t.aliases.iter().any(|a| a.eq_ignore_ascii_case(name)))
    }
    pub fn search(&self, query: &str) -> Vec<&Tool> {
        let query = query.trim().to_lowercase();
        self.tools
            .iter()
            .filter(|tool| {
                query.is_empty()
                    || tool.label.to_lowercase().contains(&query)
                    || tool
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
            })
            .collect()
    }
    pub fn workspace(&self, mode: Mode) -> &Workspace {
        self.workspaces
            .iter()
            .find(|w| w.mode == mode)
            .expect("registered built-in workspace")
    }
    pub fn parse_line<'a>(&self, line: &'a str) -> Result<(Command, Vec<&'a str>), String> {
        let mut tokens = line.split_whitespace();
        let name = tokens.next().ok_or("Enter a command")?;
        let tool = self
            .resolve(name)
            .ok_or_else(|| format!("Unknown command '{name}'"))?;
        Ok((tool.action, tokens.collect()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn finder_matches_labels_and_aliases_independently_of_case_and_surrounding_space() {
        let registry = Registry::default();
        assert_eq!(
            registry
                .search("  RECT ")
                .iter()
                .map(|t| t.action)
                .collect::<Vec<_>>(),
            vec![Command::Rectangle]
        );
        assert_eq!(
            registry
                .search("params")
                .iter()
                .map(|t| t.action)
                .collect::<Vec<_>>(),
            vec![Command::Mode(Mode::Parameters)]
        );
        assert_eq!(registry.search("  ").len(), registry.tools.len());
        assert!(registry.search("no-such-tool").is_empty());
    }
    #[test]
    fn definitions_have_unique_ids_aliases_and_resolvable_ribbon_references() {
        let r = Registry::default();
        let mut ids = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        for t in &r.tools {
            assert!(ids.insert(t.id));
            for a in t.aliases {
                assert!(aliases.insert(*a));
                assert_eq!(r.resolve(a).unwrap().id, t.id);
            }
        }
        for w in &r.workspaces {
            for tab in &w.tabs {
                for id in &tab.tools {
                    assert!(r.tool(id).is_some());
                }
            }
        }
        assert_eq!(
            r.parse_line("rect 0 0 80 60").unwrap(),
            (Command::Rectangle, vec!["0", "0", "80", "60"])
        );
        assert!(r.parse_line("rm -rf /").is_err());
        assert!(
            r.workspace(Mode::Export)
                .tabs
                .iter()
                .any(|t| t.title == "Render")
        );
    }
}
