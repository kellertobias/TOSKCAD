//! Declarative workspace/ribbon catalog shared by buttons and command entry.
use crate::{
    icons::Icon, modeling::ModelingTool, sketch_editor::DrawTool, sketch_operations::OperationTool,
};
use eframe::egui::Color32;
pub(crate) const PROJECT_ACTIONS: [&str; 5] = ["apply", "discard", "preview", "undo", "redo"];
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Parameters,
    Sketch,
    Model,
    Motion,
    Drawings,
    Export,
}
impl Mode {
    pub const ALL: [Self; 5] = [
        Self::Parameters,
        Self::Model,
        Self::Motion,
        Self::Drawings,
        Self::Export,
    ];
    /// Internal Sketch is the contextual Model editor, never a top-level tab.
    pub const WORKSPACES: [Self; 6] = [
        Self::Parameters,
        Self::Sketch,
        Self::Model,
        Self::Motion,
        Self::Drawings,
        Self::Export,
    ];
    pub fn top_level(self) -> Self {
        if self == Self::Sketch {
            Self::Model
        } else {
            self
        }
    }
    pub fn from_stored_name(name: &str) -> Option<Self> {
        if name == "Parameters" {
            return Some(Self::Parameters);
        }
        Self::WORKSPACES
            .into_iter()
            .find(|mode| mode.title() == name)
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Parameters => "Project",
            Self::Sketch => "Sketch",
            Self::Model => "Model",
            Self::Motion => "Motion",
            Self::Drawings => "Drawings",
            Self::Export => "Export",
        }
    }
    pub fn color(self) -> Color32 {
        match self {
            Self::Parameters => Color32::from_rgb(108, 73, 160),
            Self::Sketch => Color32::from_rgb(28, 113, 119),
            Self::Model => Color32::from_rgb(34, 98, 171),
            Self::Motion => Color32::from_rgb(28, 113, 119),
            Self::Drawings => Color32::from_rgb(172, 91, 18),
            Self::Export => Color32::from_rgb(53, 114, 65),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    KeyboardShortcuts,
    Measure,
    Parameter,
    Apply,
    Discard,
    Preview,
    Undo,
    Redo,
    Save,
    SaveAs,
    Import,
    ImportSource,
    ImportArtwork,
    RecoverProjects,
    /// Gallery of bundled example projects (CAD-186).
    Examples,
    RecoverySnapshot,
    Open,
    New,
    NewWindow,
    NewSketch,
    Engraving,
    Draw(DrawTool),
    SketchOperation(OperationTool),
    External(crate::external_references::ExternalTool),
    Modeling(ModelingTool),
    EquipmentPreset(multicad_core::equipment::Kind),
    WoodPreset(crate::wood_navigation::Preset),
    ArchitectureCategory(crate::architecture_navigation::Category),
    ArchitectureParent,
    /// Boolean form preset to Join, Subtract (Cut) or Intersect.
    BooleanOperation(multicad_core::model::Operation),
    Assembly(crate::assembly::AssemblyTool),
    Transfer,
    Construction,
    Dimension,
    FinishSketch,
    AbortSketch,
    FinishCurve,
    Drawing(crate::drawings::DrawingTool),
    Export(crate::exchange::ExportTool),
    Print(crate::print::PrintTool),
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
    pub requires_active_sketch: bool,
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
/// Canonical title for persisted Model navigation before sparse-tool consolidation.
pub fn canonical_workbench_title(mode: Mode, title: &str) -> &str {
    if mode == Mode::Model && matches!(title, "FDM" | "FDM aids") {
        "Other"
    } else if mode == Mode::Parameters && title == "Parameters" {
        "Common"
    } else {
        title
    }
}
fn command_home(command: Command) -> Option<(Mode, &'static str)> {
    match command {
        Command::Extension("materials") => Some((Mode::Parameters, "Materials")),
        Command::Extension("drawing-pdf") => Some((Mode::Drawings, "Common")),
        Command::WoodPreset(_)
        | Command::Modeling(
            ModelingTool::Panel
            | ModelingTool::Cabinet
            | ModelingTool::WoodBox
            | ModelingTool::Machining
            | ModelingTool::Joint
            | ModelingTool::LooseTenon
            | ModelingTool::ShelfPins,
        ) => Some((Mode::Model, "Wood")),
        Command::EquipmentPreset(_)
        | Command::ArchitectureCategory(_)
        | Command::ArchitectureParent
        | Command::Modeling(
            ModelingTool::Wall
            | ModelingTool::Fence
            | ModelingTool::Door
            | ModelingTool::Window
            | ModelingTool::Cutout
            | ModelingTool::Level
            | ModelingTool::Stair
            | ModelingTool::Plant
            | ModelingTool::Equipment,
        ) => Some((Mode::Model, "Architecture")),
        _ => None,
    }
}
impl Default for Registry {
    fn default() -> Self {
        let tools = vec![
            Tool {
                id: "measure",
                label: "Measure",
                icon: Icon::Measure,
                action: Command::Measure,
                aliases: &["measure", "distance"],
            },
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
                label: "Apply and Rebuild",
                icon: Icon::Save,
                action: Command::Apply,
                aliases: &["apply", "rebuild"],
            },
            Tool {
                id: "discard",
                label: "Discard Edit",
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
                aliases: &["import-snapshot"],
            },
            Tool {
                id: "recover-projects",
                label: "Recover projects",
                icon: Icon::Page,
                action: Command::RecoverProjects,
                aliases: &["recover"],
            },
            Tool {
                id: "examples",
                label: "Examples",
                icon: Icon::Page,
                action: Command::Examples,
                aliases: &["example", "demo", "sample", "gallery"],
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
                label: "Add Sketch",
                icon: Icon::Sketch,
                action: Command::NewSketch,
                aliases: &["start", "new-sketch", "add-sketch", "sketch"],
            },
            Tool {
                id: "import",
                label: "Import",
                icon: Icon::Page,
                action: Command::Import,
                aliases: &["import-file"],
            },
            Tool {
                id: "import-artwork",
                label: "Import DXF/SVG",
                icon: Icon::Page,
                action: Command::ImportArtwork,
                aliases: &["import-artwork", "dxf", "svg"],
            },
            Tool {
                id: "construction",
                label: "Construction",
                icon: Icon::Construction,
                action: Command::Construction,
                aliases: &["construction"],
            },
            Tool {
                id: "finish-sketch",
                label: "Finish sketch",
                icon: Icon::Save,
                action: Command::FinishSketch,
                aliases: &["finish"],
            },
            Tool {
                id: "abort-sketch",
                label: "Abort",
                icon: Icon::Undo,
                action: Command::AbortSketch,
                aliases: &["abort"],
            },
            Tool {
                id: "finish-curve",
                label: "Finish curve",
                icon: Icon::Spline,
                action: Command::FinishCurve,
                aliases: &["endcurve"],
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
        for tool in DrawTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::Draw(tool),
                aliases: tool.aliases(),
            });
        }
        for tool in OperationTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::SketchOperation(tool),
                aliases: tool.aliases(),
            });
        }
        for tool in crate::external_references::ExternalTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::External(tool),
                aliases: tool.aliases(),
            });
        }
        for tool in ModelingTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::Modeling(tool),
                aliases: tool.aliases(),
            });
        }
        for (id, label, operation, aliases) in BOOLEAN_OPERATIONS {
            registry.tools.push(Tool {
                id,
                label,
                icon: Icon::Boolean,
                action: Command::BooleanOperation(operation),
                aliases,
            });
        }
        for tool in crate::assembly::AssemblyTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::Assembly(tool),
                aliases: tool.aliases(),
            });
        }
        registry.tools.push(Tool {
            id: "transfer",
            label: "Transfer features",
            icon: Icon::Transfer,
            action: Command::Transfer,
            aliases: &["derive-holes", "transfer-holes", "project-cutouts"],
        });
        for mode in Mode::ALL {
            registry.tools.push(Tool {
                // Keep saved command identifiers and old command-line aliases.
                id: if mode == Mode::Parameters {
                    "Parameters"
                } else {
                    mode.title()
                },
                label: mode.title(),
                icon: Icon::Page,
                action: Command::Mode(mode),
                aliases: match mode {
                    Mode::Parameters => &["project-mode", "parameters", "params"],
                    Mode::Sketch => unreachable!("contextual Model editor"),
                    Mode::Motion => &["motion-mode"],
                    Mode::Model => &["model"],
                    Mode::Drawings => &["drawings"],
                    Mode::Export => &["export"],
                },
            });
        }
        for tool in crate::drawings::DrawingTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::Drawing(tool),
                aliases: tool.aliases(),
            });
        }
        for tool in crate::exchange::ExportTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::Export(tool),
                aliases: tool.aliases(),
            });
        }
        for tool in crate::print::PrintTool::ALL {
            registry.tools.push(Tool {
                id: tool.id(),
                label: tool.label(),
                icon: tool.icon(),
                action: Command::Print(tool),
                aliases: tool.aliases(),
            });
        }
        registry.tools.push(Tool {
            id: "settings",
            label: "Settings",
            icon: Icon::Settings,
            action: Command::Extension("settings"),
            aliases: &[
                "preferences",
                "units",
                "inches",
                "millimeters",
                "workbenches",
            ],
        });
        for (id, label, icon) in [
            ("symbol", "Create symbol", Icon::Library),
            ("library", "Symbol library", Icon::Library),
            ("cut-list", "Cut list", Icon::Page),
            ("fastener", "Attached fastener", Icon::Link),
            ("flat", "Flat pattern", Icon::Page),
            ("bend-list", "Bend schedule", Icon::Page),
            ("weld-list", "Weld sequence", Icon::Page),
            ("materials", "Materials", Icon::Library),
            ("drawing-pdf", "Export as PDF", Icon::Export),
            ("text", "Text / leader", Icon::Sketch),
            ("appearance", "Capture render view", Icon::Box),
            ("render-view", "Show render view", Icon::Box),
            ("blender", "Blender handoff", Icon::Export),
            ("import-step", "Import STEP", Icon::Page),
            ("import-kicad", "Import KiCad board", Icon::Page),
            ("import-update", "Update import", Icon::Page),
        ] {
            registry.tools.push(Tool {
                id,
                label,
                icon,
                action: Command::Extension(id),
                aliases: if id == "drawing-pdf" {
                    &["drawingpdf", "export-drawing-pdf"]
                } else {
                    &[]
                },
            });
        }
        let tab = |title, tools: &[&'static str]| RibbonTab {
            title,
            tools: tools.to_vec(),
            requires_active_sketch: false,
        };
        registry
            .tools
            .extend(crate::architecture_navigation::tools());
        registry.tools.extend(crate::wood_navigation::tools());
        registry.workspaces = vec![
            Workspace {
                mode: Mode::Parameters,
                tabs: vec![
                    tab("Common", &PROJECT_ACTIONS),
                    tab("Materials", &["materials"]),
                ],
            },
            Workspace {
                mode: Mode::Sketch,
                tabs: vec![
                    tab(
                        "Common",
                        &[
                            "rectangle",
                            "line",
                            "circle",
                            "arc",
                            "polyline",
                            "spline",
                            "construction",
                            "dimension",
                            "delete",
                        ],
                    ),
                    RibbonTab {
                        title: "Edit",
                        tools: OperationTool::ALL
                            .into_iter()
                            .map(OperationTool::id)
                            .collect(),
                        requires_active_sketch: true,
                    },
                    RibbonTab {
                        title: "References",
                        tools: crate::external_references::ExternalTool::ALL
                            .into_iter()
                            .map(|tool| tool.id())
                            .chain(["preview", "apply", "discard"])
                            .collect(),
                        requires_active_sketch: true,
                    },
                    tab(
                        "Room",
                        &["wall", "door", "window-tool", "vertical-cutout", "level"],
                    ),
                ],
            },
            Workspace {
                mode: Mode::Model,
                tabs: vec![
                    RibbonTab {
                        title: "Common",
                        tools: ModelingTool::ALL
                            .into_iter()
                            .filter(|tool| {
                                !matches!(
                                    tool,
                                    ModelingTool::Copy
                                        | ModelingTool::Thread
                                        | ModelingTool::Gear
                                        | ModelingTool::MechanicalPart
                                        | ModelingTool::ScrewConnection
                                        | ModelingTool::Boolean
                                        | ModelingTool::Wall
                                        | ModelingTool::Fence
                                        | ModelingTool::Door
                                        | ModelingTool::Window
                                        | ModelingTool::Cutout
                                        | ModelingTool::Level
                                        | ModelingTool::Stair
                                        | ModelingTool::Plant
                                        | ModelingTool::Equipment
                                        | ModelingTool::Panel
                                        | ModelingTool::Cabinet
                                        | ModelingTool::WoodBox
                                        | ModelingTool::Joint
                                        | ModelingTool::Machining
                                        | ModelingTool::ShelfPins
                                        | ModelingTool::LooseTenon
                                        | ModelingTool::FdmAid
                                        | ModelingTool::FdmBridge
                                        | ModelingTool::FdmSupport
                                        | ModelingTool::Member
                                        | ModelingTool::Connection
                                        | ModelingTool::Weld
                                        | ModelingTool::Sheet
                                )
                            })
                            .map(ModelingTool::id)
                            .chain(BOOLEAN_OPERATIONS.map(|(id, ..)| id))
                            .chain(["symbol", "transfer"])
                            .collect(),
                        requires_active_sketch: false,
                    },
                    RibbonTab {
                        title: "Architecture",
                        tools: crate::architecture_navigation::tab_tools(),
                        requires_active_sketch: false,
                    },
                    RibbonTab {
                        title: "Wood",
                        tools: crate::wood_navigation::tab_tools(),
                        requires_active_sketch: false,
                    },
                    tab("Sheet metal", &["sheet", "fastener", "flat", "bend-list"]),
                    tab("Weld", &["tube", "cope", "weld-joint", "weld-list"]),
                    tab("Other", &["fdm-aid", "fdm-bridge", "fdm-support"]),
                    tab("Relationships", &["align"]),
                    tab(
                        "Mechanical",
                        &["thread", "gear", "mechanical-part", "screw-connection"],
                    ),
                ],
            },
            Workspace {
                mode: Mode::Motion,
                tabs: vec![RibbonTab {
                    title: "Common",
                    tools: crate::assembly::AssemblyTool::ALL
                        .into_iter()
                        .filter(|tool| tool.motion_workspace())
                        .map(|tool| tool.id())
                        .collect(),
                    requires_active_sketch: false,
                }],
            },
            Workspace {
                mode: Mode::Drawings,
                tabs: vec![
                    tab(
                        "Common",
                        &[
                            "drawing-new",
                            "drawing-pdf",
                            "sheet-page",
                            "page-duplicate",
                            "page-remove",
                            "page-earlier",
                            "page-later",
                            "page-setup",
                            "page-apply",
                            "page-discard",
                            "drawing-projection",
                            "drawing-section",
                            "drawing-detail",
                        ],
                    ),
                    tab(
                        "Annotate",
                        &[
                            "drawing-note",
                            "drawing-dimension",
                            "drawing-leader",
                            "drawing-weld",
                            "drawing-table",
                            "drawing-schedule",
                        ],
                    ),
                    tab(
                        "Drafting",
                        &[
                            "draft-line",
                            "draft-circle",
                            "draft-arc",
                            "draft-hatch",
                            "draft-dimension",
                        ],
                    ),
                ],
            },
            Workspace {
                mode: Mode::Export,
                tabs: vec![
                    tab(
                        "Common",
                        &[
                            "export-set",
                            "export-set-save",
                            "export-set-discard",
                            "export-preview",
                            "export-run",
                            "export-cancel",
                            "export-schedule-csv",
                        ],
                    ),
                    tab("Print", &["print-preview", "print-write", "print-cancel"]),
                    tab("Render", &["appearance", "render-view", "blender"]),
                ],
            },
        ];
        // Drawing modes follow their default tool on Sketch → Common; the
        // ribbon groups each family under one menu.
        if let Some(common) = registry
            .workspaces
            .iter_mut()
            .find(|w| w.mode == Mode::Sketch)
            .and_then(|w| w.tabs.iter_mut().find(|t| t.title == "Common"))
        {
            for family in [
                &DrawTool::RECTANGLE_MODES[..],
                &DrawTool::CIRCLE_MODES[..],
                &[
                    DrawTool::Spline,
                    DrawTool::Fillet,
                    DrawTool::Chamfer,
                    DrawTool::Text,
                ][..],
            ] {
                let base = family[0].id();
                let mut at = common
                    .tools
                    .iter()
                    .position(|id| *id == base)
                    .map_or(common.tools.len(), |i| i + 1);
                for mode in &family[1..] {
                    if !common.tools.contains(&mode.id()) {
                        common.tools.insert(at, mode.id());
                        at += 1;
                    }
                }
            }
        }
        registry
    }
}
/// Ribbon Boolean menu entries; each opens the Boolean form preset to an
/// implemented native operation.
pub const BOOLEAN_OPERATIONS: [(&str, &str, multicad_core::model::Operation, &[&str]); 3] = [
    (
        "boolean-join",
        "Join",
        multicad_core::model::Operation::Join,
        &["union", "join-bodies"],
    ),
    (
        "boolean-subtract",
        "Subtract",
        multicad_core::model::Operation::Cut,
        &["subtract", "subtract-bodies"],
    ),
    (
        "boolean-intersect",
        "Intersect",
        multicad_core::model::Operation::Intersect,
        &["intersect-bodies"],
    ),
];
impl Registry {
    pub fn tool(&self, id: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.id == id)
    }
    pub fn resolve(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| {
            t.id.eq_ignore_ascii_case(name)
                || t.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
        })
    }
    /// Tools matching `query` by id, label, alias or workbench, followed by the
    /// workflows that write an output format it names (CAD-157).
    pub fn search(&self, query: &str) -> Vec<&Tool> {
        let query = query.trim().to_lowercase();
        let mut found: Vec<&Tool> = self
            .tools
            .iter()
            .filter(|tool| {
                query.is_empty()
                    || tool.id.to_lowercase().contains(&query)
                    || tool.label.to_lowercase().contains(&query)
                    || tool
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
                    || self.workspaces.iter().any(|workspace| {
                        workspace.tabs.iter().any(|tab| {
                            tab.title.to_lowercase().contains(&query)
                                && tab.tools.contains(&tool.id)
                        })
                    })
            })
            .collect();
        for route in crate::tool_search::routes(&query) {
            for (id, _) in route.tools {
                if let Some(tool) = self.tool(id)
                    && !found.iter().any(|known| known.id == *id)
                {
                    found.push(tool);
                }
            }
        }
        // Prefer the visible name when a legacy command alias has the same word.
        let format_order: Vec<_> = crate::tool_search::routes(&query)
            .flat_map(|route| route.tools.iter().map(|(id, _)| *id))
            .collect();
        found.sort_by_key(|tool| {
            (
                !tool.label.eq_ignore_ascii_case(&query),
                format_order
                    .iter()
                    .position(|id| *id == tool.id)
                    .unwrap_or(usize::MAX),
            )
        });
        found
    }
    /// Workspace and ribbon tab that offer `command`, for finder context.
    pub fn location(&self, command: Command) -> Option<(Mode, &'static str)> {
        if let Some((mode, title)) = command_home(command)
            && self.workspace(mode).tabs.iter().any(|tab| {
                tab.title == title
                    && tab
                        .tools
                        .iter()
                        .any(|id| self.tool(id).is_some_and(|tool| tool.action == command))
            })
        {
            return Some((mode, title));
        }
        self.workspaces.iter().find_map(|workspace| {
            workspace
                .tabs
                .iter()
                .find(|tab| {
                    tab.tools
                        .iter()
                        .any(|id| self.tool(id).is_some_and(|tool| tool.action == command))
                })
                .map(|tab| (workspace.mode, tab.title))
        })
    }
    /// Ribbon tab of `mode` that offers `command`: `current` when it does
    /// (a tool listed on several tabs stays where it was started), else the
    /// canonical command home when declared, else the first tab listing it (CAD-162).
    pub fn workbench_tab(
        &self,
        mode: Mode,
        command: Command,
        current: Option<usize>,
    ) -> Option<usize> {
        let tabs = &self.workspace(mode).tabs;
        let offers = |tab: &RibbonTab| {
            tab.tools
                .iter()
                .any(|id| self.tool(id).is_some_and(|tool| tool.action == command))
        };
        current
            .filter(|index| tabs.get(*index).is_some_and(offers))
            .or_else(|| {
                command_home(command)
                    .filter(|(home, _)| *home == mode)
                    .and_then(|(_, title)| {
                        tabs.iter()
                            .position(|tab| tab.title == title && offers(tab))
                    })
            })
            .or_else(|| tabs.iter().position(offers))
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
    fn assembly_and_library_compatibility_commands_are_not_specialist_ribbon_homes() {
        let registry = Registry::default();
        for tab in &registry.workspace(Mode::Model).tabs {
            for id in ["create-assembly", "place-component"] {
                assert!(
                    !tab.tools.contains(&id),
                    "{id} belongs to tree/global Library, not {}",
                    tab.title
                );
            }
        }
        assert!(
            !registry
                .workspace(Mode::Model)
                .tabs
                .iter()
                .find(|tab| tab.title == "Mechanical")
                .unwrap()
                .tools
                .contains(&"library")
        );
        assert_eq!(
            registry.tool("create-assembly").unwrap().action,
            Command::Assembly(crate::assembly::AssemblyTool::CreateAssembly)
        );
        assert_eq!(
            registry.tool("place-component").unwrap().action,
            Command::Assembly(crate::assembly::AssemblyTool::Place)
        );
        assert_eq!(
            registry.tool("library").unwrap().action,
            Command::Extension("library")
        );
        let motion = registry.workspace(Mode::Motion);
        for tool in crate::assembly::AssemblyTool::ALL
            .into_iter()
            .filter(|tool| tool.motion_workspace())
        {
            assert!(motion.tabs.iter().any(|tab| tab.tools.contains(&tool.id())));
        }
    }
    #[test]
    fn associative_transfer_is_discoverable_in_model_common() {
        let registry = Registry::default();
        let model = registry.workspace(Mode::Model);
        let common = model
            .tabs
            .iter()
            .position(|tab| tab.title == "Common")
            .unwrap();
        assert!(model.tabs[common].tools.contains(&"transfer"));
        assert_eq!(
            registry.workbench_tab(Mode::Model, registry.tool("transfer").unwrap().action, None),
            Some(common)
        );
        assert!(
            !model
                .tabs
                .iter()
                .find(|tab| tab.title == "Relationships")
                .unwrap()
                .tools
                .contains(&"transfer")
        );
    }
    #[test]
    fn project_mode_keeps_legacy_commands_while_search_and_labels_use_project() {
        let registry = Registry::default();
        for name in ["project-mode", "Parameters", "parameters", "params"] {
            let tool = registry
                .resolve(name)
                .expect("Project command or retained alias");
            assert_eq!(tool.action, Command::Mode(Mode::Parameters));
            assert_eq!(tool.label, "Project");
        }
        assert!(
            registry
                .search("project")
                .iter()
                .any(|tool| tool.label == "Project")
        );
        assert_eq!(
            registry.search("project")[0].action,
            Command::Mode(Mode::Parameters)
        );
        assert_eq!(registry.tool("Parameters").unwrap().label, "Project");
        assert_eq!(
            registry.resolve("project").unwrap().action,
            Command::External(crate::external_references::ExternalTool::Project),
            "the existing project-reference command remains compatible"
        );
    }
    #[test]
    fn mechanical_reuses_global_library_and_assembly_commands_without_new_modes() {
        let registry = Registry::default();
        assert_eq!(
            Mode::ALL.map(Mode::title),
            ["Project", "Model", "Motion", "Drawings", "Export"]
        );
        let model = registry.workspace(Mode::Model);
        let mechanical = model
            .tabs
            .iter()
            .position(|tab| tab.title == "Mechanical")
            .unwrap();
        assert_eq!(
            model
                .tabs
                .iter()
                .find(|tab| tab.title == "Weld")
                .unwrap()
                .tools,
            ["tube", "cope", "weld-joint", "weld-list"]
        );
        let found = registry.search("  MECHANICAL ");
        for id in &model.tabs[mechanical].tools {
            let tool = registry.tool(id).unwrap();
            assert!(found.iter().any(|found| found.id == *id));
            assert_eq!(
                registry.workbench_tab(Mode::Model, tool.action, Some(mechanical)),
                Some(mechanical)
            );
            if *id == "thread" {
                assert_eq!(tool.action, Command::Modeling(ModelingTool::Thread));
            } else if *id == "gear" {
                assert_eq!(tool.action, Command::Modeling(ModelingTool::Gear));
            } else if *id == "mechanical-part" {
                assert_eq!(tool.action, Command::Modeling(ModelingTool::MechanicalPart));
            } else if *id == "screw-connection" {
                assert_eq!(
                    tool.action,
                    Command::Modeling(ModelingTool::ScrewConnection)
                );
            } else if *id == "library" {
                assert_eq!(tool.action, Command::Extension("library"));
            } else {
                assert!(matches!(tool.action, Command::Assembly(_)));
            }
        }
    }
    #[test]
    fn finder_matches_labels_and_aliases_independently_of_case_and_surrounding_space() {
        let registry = Registry::default();
        assert_eq!(
            registry
                .search("  RECT ")
                .iter()
                .map(|t| t.action)
                .collect::<Vec<_>>(),
            DrawTool::RECTANGLE_MODES.map(Command::Draw).to_vec(),
            "the finder lists every rectangle mode, default first"
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
                assert!(aliases.insert(*a), "Duplicate alias {a} on {}", t.id);
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
            (
                Command::Draw(DrawTool::Rectangle),
                vec!["0", "0", "80", "60"]
            )
        );
        assert!(r.parse_line("rm -rf /").is_err());
        assert!(
            r.workspace(Mode::Export)
                .tabs
                .iter()
                .any(|t| t.title == "Render")
        );
    }
    #[test]
    fn drawing_catalog_drives_command_aliases_and_all_common_ribbon_buttons() {
        let registry = Registry::default();
        let common = &registry.workspace(Mode::Sketch).tabs[0];
        for draw in DrawTool::ALL {
            let tool = registry.tool(draw.id()).unwrap();
            assert_eq!(tool.action, Command::Draw(draw));
            assert_eq!(tool.label, draw.label());
            assert!(common.tools.contains(&draw.id()));
            for alias in draw.aliases() {
                assert_eq!(registry.parse_line(alias).unwrap().0, Command::Draw(draw));
            }
        }
        assert!(common.tools.contains(&"construction"));
        assert_eq!(
            registry.resolve("construction").unwrap().action,
            Command::Construction
        );
        assert_eq!(
            registry.parse_line("rect 0 0 1 1").unwrap().0,
            Command::Draw(DrawTool::Rectangle)
        );
    }
    #[test]
    fn drawing_page_tools_share_the_catalog_across_ribbon_and_command_entry() {
        let registry = Registry::default();
        for tool in crate::drawings::DrawingTool::ALL {
            let registered = registry.tool(tool.id()).unwrap();
            assert_eq!(registered.action, Command::Drawing(tool));
            assert_eq!(registered.label, tool.label());
            assert!(
                registry
                    .workspace(Mode::Drawings)
                    .tabs
                    .iter()
                    .any(|tab| tab.tools.contains(&tool.id()))
            );
            for alias in tool.aliases() {
                assert_eq!(
                    registry.parse_line(alias).unwrap().0,
                    Command::Drawing(tool)
                );
            }
        }
        assert_eq!(
            registry.parse_line("projection").unwrap().0,
            Command::Drawing(crate::drawings::DrawingTool::Projection)
        );
    }
    #[test]
    fn external_reference_tools_share_active_sketch_ribbon_and_command_catalog() {
        let registry = Registry::default();
        let tab = registry
            .workspace(Mode::Sketch)
            .tabs
            .iter()
            .find(|tab| tab.title == "References")
            .unwrap();
        assert!(tab.requires_active_sketch);
        for tool in crate::external_references::ExternalTool::ALL {
            assert_eq!(
                registry.tool(tool.id()).unwrap().action,
                Command::External(tool)
            );
            assert!(tab.tools.contains(&tool.id()));
            for alias in tool.aliases() {
                assert_eq!(
                    registry.parse_line(alias).unwrap().0,
                    Command::External(tool)
                );
            }
        }
        for action in ["preview", "apply", "discard"] {
            assert!(tab.tools.contains(&action));
        }
    }
    #[test]
    fn sketch_operation_catalog_drives_edit_ribbon_and_unambiguous_aliases() {
        let registry = Registry::default();
        let edit = registry
            .workspace(Mode::Sketch)
            .tabs
            .iter()
            .find(|tab| tab.title == "Edit")
            .unwrap();
        assert!(edit.requires_active_sketch);
        assert_eq!(
            edit.tools,
            OperationTool::ALL
                .into_iter()
                .map(OperationTool::id)
                .collect::<Vec<_>>()
        );
        for operation in OperationTool::ALL {
            let tool = registry.tool(operation.id()).unwrap();
            assert_eq!(tool.action, Command::SketchOperation(operation));
            assert_eq!(tool.label, operation.label());
            for alias in operation.aliases() {
                assert_eq!(
                    registry.resolve(alias).unwrap().action,
                    Command::SketchOperation(operation)
                );
            }
        }
        assert_eq!(
            registry.resolve("move").unwrap().action,
            Command::Modeling(ModelingTool::Move)
        );
    }
    #[test]
    fn modeling_ribbon_and_aliases_use_the_shared_tool_catalog() {
        let registry = Registry::default();
        for tool in ModelingTool::ALL {
            let registered = registry.tool(tool.id()).unwrap();
            assert_eq!(registered.action, Command::Modeling(tool));
            assert_eq!(registered.label, tool.label());
            assert_eq!(
                registry.workspace(Mode::Model).tabs[0]
                    .tools
                    .contains(&tool.id()),
                !matches!(
                    tool,
                    ModelingTool::Copy
                        | ModelingTool::Thread
                        | ModelingTool::Gear
                        | ModelingTool::MechanicalPart
                        | ModelingTool::ScrewConnection
                        | ModelingTool::Boolean
                        | ModelingTool::Wall
                        | ModelingTool::Fence
                        | ModelingTool::Door
                        | ModelingTool::Window
                        | ModelingTool::Cutout
                        | ModelingTool::Level
                        | ModelingTool::Stair
                        | ModelingTool::Plant
                        | ModelingTool::Equipment
                        | ModelingTool::Panel
                        | ModelingTool::Cabinet
                        | ModelingTool::WoodBox
                        | ModelingTool::Joint
                        | ModelingTool::Machining
                        | ModelingTool::ShelfPins
                        | ModelingTool::LooseTenon
                        | ModelingTool::FdmAid
                        | ModelingTool::FdmBridge
                        | ModelingTool::FdmSupport
                        | ModelingTool::Member
                        | ModelingTool::Connection
                        | ModelingTool::Weld
                        | ModelingTool::Sheet
                ),
                "Copy shares the Move / Copy ribbon entry; Boolean is offered as Join / Subtract / Intersect; workbench tools live on their Architecture, Wood, Sheet metal, Weld and FDM tabs"
            );
            if matches!(
                tool,
                ModelingTool::Member | ModelingTool::Connection | ModelingTool::Weld
            ) {
                let weld = registry
                    .workspace(Mode::Model)
                    .tabs
                    .iter()
                    .find(|t| t.title == "Weld");
                assert!(weld.unwrap().tools.contains(&tool.id()));
            }
            if matches!(
                tool,
                ModelingTool::Wall
                    | ModelingTool::Fence
                    | ModelingTool::Door
                    | ModelingTool::Window
                    | ModelingTool::Cutout
                    | ModelingTool::Level
                    | ModelingTool::Stair
                    | ModelingTool::Plant
                    | ModelingTool::Equipment
            ) {
                let architecture = registry
                    .workspace(Mode::Model)
                    .tabs
                    .iter()
                    .find(|t| t.title == "Architecture");
                assert!(architecture.unwrap().tools.contains(&tool.id()));
            }
            if tool == ModelingTool::Sheet {
                let tab = registry
                    .workspace(Mode::Model)
                    .tabs
                    .iter()
                    .find(|t| t.title == "Sheet metal");
                assert!(tab.unwrap().tools.contains(&tool.id()));
            }
            if tool.fdm_kind().is_some() {
                let fdm = registry
                    .workspace(Mode::Model)
                    .tabs
                    .iter()
                    .find(|t| t.title == "Other");
                assert!(fdm.unwrap().tools.contains(&tool.id()));
                assert_eq!(registry.resolve(tool.aliases()[0]).unwrap().id, tool.id());
            }
            if matches!(
                tool,
                ModelingTool::Panel
                    | ModelingTool::Cabinet
                    | ModelingTool::WoodBox
                    | ModelingTool::Joint
                    | ModelingTool::Machining
                    | ModelingTool::ShelfPins
                    | ModelingTool::LooseTenon
            ) {
                let wood = registry
                    .workspace(Mode::Model)
                    .tabs
                    .iter()
                    .find(|t| t.title == "Wood");
                assert!(wood.unwrap().tools.contains(&tool.id()));
            }
            for alias in tool.aliases() {
                assert_eq!(
                    registry.resolve(alias).unwrap().action,
                    Command::Modeling(tool)
                );
            }
        }
    }
}
