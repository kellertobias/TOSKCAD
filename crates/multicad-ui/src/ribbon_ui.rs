//! Declarative sections and menus over the shared tool catalog.
use crate::{
    icons::{self, Icon},
    registry::{Command, METAL_GROUPS, METAL_TITLE, Mode, Registry, RibbonTab},
};
use eframe::egui;

pub(crate) fn workspace_tab_icon(mode: Mode, title: &str) -> Option<Icon> {
    Some(match (mode, title) {
        (Mode::Parameters, "General") => Icon::Page,
        (Mode::Parameters, "Parameters") => Icon::Grid,
        (Mode::Parameters, "Materials") => Icon::Materials,
        (Mode::Parameters, "Export") => Icon::Export,
        (Mode::Model, "Architecture") => Icon::ArchitectureTower,
        (Mode::Model, "Wood") => Icon::Timber,
        (Mode::Model, METAL_TITLE) => Icon::Thickness,
        (Mode::Model, "Mechanical") => Icon::MechanicalScrew,
        (Mode::Model, "Other") => Icon::Grid,
        (Mode::Model, "Relations") => Icon::Link,
        (Mode::Model, _) => Icon::Box,
        _ => return None,
    })
}

#[derive(Clone)]
pub enum Entry {
    Tool(&'static str),
    Menu {
        label: &'static str,
        icon: Icon,
        tools: Vec<&'static str>,
        unavailable: Option<(&'static str, &'static str)>,
        split: bool,
    },
}
pub struct Group {
    pub label: &'static str,
    pub entries: Vec<Entry>,
}

const SKETCH_SPLIT_FOCUS: &str = "sketch-split-focus";

pub(crate) fn sketch_split_focused(ctx: &egui::Context) -> bool {
    let focus = ctx.memory(|memory| memory.focused());
    focus.is_some_and(|focus| {
        ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(SKETCH_SPLIT_FOCUS))) == Some(focus)
    })
}
/// Sketch → Common replaces each drawing-mode family with a split action
/// at the position of its first member.
fn sketch_entries(tab: &RibbonTab) -> Vec<Entry> {
    use crate::sketch_editor::DrawTool;
    let families: [(&'static str, Icon, Vec<&'static str>); 2] = [
        (
            "Rectangle",
            Icon::Rectangle,
            DrawTool::RECTANGLE_MODES.map(DrawTool::id).to_vec(),
        ),
        (
            "Circle",
            Icon::Circle,
            DrawTool::CIRCLE_MODES.map(DrawTool::id).to_vec(),
        ),
    ];
    let mut entries = vec![];
    for id in &tab.tools {
        match families.iter().find(|(_, _, tools)| tools.contains(id)) {
            Some((label, icon, tools)) => {
                if !entries
                    .iter()
                    .any(|e| matches!(e, Entry::Menu { label: l, .. } if l == label))
                {
                    let present: Vec<_> = tools
                        .iter()
                        .copied()
                        .filter(|tool| tab.tools.contains(tool))
                        .collect();
                    entries.push(if present.len() == 1 {
                        Entry::Tool(present[0])
                    } else {
                        Entry::Menu {
                            label,
                            icon: *icon,
                            tools: present,
                            unavailable: None,
                            split: true,
                        }
                    });
                }
            }
            None => entries.push(Entry::Tool(id)),
        }
    }
    entries
}
pub fn groups(mode: Mode, tab: &RibbonTab) -> Vec<Group> {
    if mode == Mode::Model && tab.title == "Wood" {
        return crate::wood_navigation::groups();
    }
    if mode == Mode::Model && tab.title == METAL_TITLE {
        return METAL_GROUPS
            .into_iter()
            .map(|(label, tools)| Group {
                label,
                entries: tools.iter().copied().map(Entry::Tool).collect(),
            })
            .collect();
    }
    if mode == Mode::Model && tab.title == "Other" {
        return vec![
            Group {
                label: "FDM design aids",
                entries: ["fdm-aid", "fdm-bridge", "fdm-support"]
                    .into_iter()
                    .map(Entry::Tool)
                    .collect(),
            },
            Group {
                label: "Surface",
                entries: vec![Entry::Tool("surface-structure")],
            },
        ];
    }
    if mode == Mode::Model && tab.title == "Architecture" {
        return crate::architecture_navigation::groups(None);
    }
    if mode == Mode::Sketch && tab.title == "Common" {
        return vec![Group {
            label: "",
            entries: sketch_entries(tab),
        }];
    }
    if mode != Mode::Model || tab.title != "Common" {
        return vec![Group {
            label: "",
            entries: tab.tools.iter().map(|id| Entry::Tool(id)).collect(),
        }];
    }
    let mut sections = vec![
        Group {
            label: "Create",
            entries: vec![
                Entry::Menu {
                    label: "Add Primitive",
                    icon: Icon::Box,
                    tools: vec!["box", "cylinder", "sphere"],
                    unavailable: None,
                    split: true,
                },
                Entry::Tool("extrude"),
                Entry::Tool("revolve"),
                Entry::Tool("sweep"),
                Entry::Tool("loft"),
            ],
        },
        Group {
            label: "Modify",
            entries: vec![
                Entry::Menu {
                    label: "Boolean",
                    icon: Icon::Boolean,
                    tools: crate::registry::BOOLEAN_OPERATIONS
                        .map(|(id, ..)| id)
                        .into_iter()
                        .chain(["split"])
                        .collect(),
                    unavailable: None,
                    split: true,
                },
                Entry::Tool("edge-treatment"),
                Entry::Tool("shell"),
                Entry::Tool("draft"),
            ],
        },
        Group {
            label: "Transform",
            entries: vec![
                Entry::Menu {
                    label: "Move / Copy",
                    icon: Icon::Transform,
                    tools: vec!["move", "duplicate", "align"],
                    unavailable: None,
                    split: true,
                },
                Entry::Tool("scale"),
            ],
        },
        Group {
            label: "Repeat",
            entries: vec![Entry::Menu {
                label: "Pattern",
                icon: Icon::CircularPattern,
                tools: vec![
                    "model-linear-pattern",
                    "model-circular-pattern",
                    "model-mirror",
                ],
                unavailable: None,
                split: true,
            }],
        },
        Group {
            label: "Library",
            entries: vec![Entry::Tool("symbol")],
        },
    ];
    for section in &mut sections {
        section.entries.retain_mut(|entry| match entry {
            Entry::Tool(id) => tab.tools.contains(id),
            Entry::Menu { tools, .. } => {
                tools.retain(|id| tab.tools.contains(id));
                !tools.is_empty()
            }
        });
    }
    sections.retain(|group| !group.entries.is_empty());
    let used: std::collections::BTreeSet<_> = sections
        .iter()
        .flat_map(|group| {
            group.entries.iter().flat_map(|entry| match entry {
                Entry::Tool(id) => vec![*id],
                Entry::Menu { tools, .. } => tools.clone(),
            })
        })
        .collect();
    let extra: Vec<_> = tab
        .tools
        .iter()
        .filter(|id| !used.contains(**id))
        .map(|id| Entry::Tool(id))
        .collect();
    if !extra.is_empty() {
        sections.push(Group {
            label: "More",
            entries: extra,
        });
    }
    sections
}
fn implemented(command: Command) -> bool {
    !matches!(command,Command::Extension(id) if !matches!(id,"duplicate"|"library"|"materials"|"drawing-pdf"|"print-plate"|"appearance"|"render-view"|"import-step"|"import-kicad"|"import-update"))
}
/// Session actions pinned at the right end of the ribbon while a sketch is
/// open, on every Sketch tab and outside the horizontal scroll.
pub const SKETCH_SESSION: [&str; 2] = ["finish-sketch", "abort-sketch"];
const COMPACT_TILE_WIDTH: f32 = 68.;
const COMPACT_SEPARATOR: f32 = 9.;
const SEPARATOR: f32 = 19.;
const SECTION_LABEL: f32 = 14.;
const EMPTY_HEIGHT: f32 = 30.;
const SCROLL_CONTROL_WIDTH: f32 = 28.;

struct Contents {
    sections: Vec<Group>,
    pinned: Vec<&'static str>,
}

impl Contents {
    fn new(
        registry: &Registry,
        mode: Mode,
        tab: &RibbonTab,
        editing: bool,
        category: Option<crate::architecture_navigation::Category>,
    ) -> Self {
        let pinned = if mode == Mode::Sketch && editing {
            SKETCH_SESSION
                .into_iter()
                .filter(|id| registry.tool(id).is_some())
                .collect()
        } else {
            vec![]
        };
        let visible = |id: &&str| {
            registry.tool(id).is_some_and(|tool| {
                implemented(tool.action)
                    && !SKETCH_SESSION.contains(id)
                    && (mode != Mode::Sketch || editing)
            })
        };
        let mut sections = if mode == Mode::Model && tab.title == "Architecture" {
            crate::architecture_navigation::groups(category)
        } else {
            groups(mode, tab)
        };
        for group in &mut sections {
            group.entries.retain_mut(|entry| match entry {
                Entry::Tool(id) => visible(id),
                Entry::Menu { tools, .. } => {
                    tools.retain(|id| visible(id));
                    !tools.is_empty()
                }
            });
        }
        sections.retain(|group| !group.entries.is_empty());
        Self { sections, pinned }
    }

    fn height(&self) -> f32 {
        if self.sections.is_empty() && self.pinned.is_empty() {
            return EMPTY_HEIGHT;
        }
        let captions = !self.pinned.is_empty() || self.sections.iter().any(|g| !g.label.is_empty());
        icons::TOOL_TILE.y + if captions { SECTION_LABEL } else { 0. }
    }

    fn sections_width(&self, tile: f32, separator: f32) -> f32 {
        let tiles: usize = self.sections.iter().map(|group| group.entries.len()).sum();
        tiles as f32 * tile + self.sections.len().saturating_sub(1) as f32 * separator
    }

    fn pinned_width(&self, tile: f32, separator: f32) -> f32 {
        if self.pinned.is_empty() {
            0.
        } else {
            self.pinned.len() as f32 * tile + separator
        }
    }
}

pub(crate) fn tools_height(
    registry: &Registry,
    mode: Mode,
    tab: &RibbonTab,
    editing: bool,
    category: Option<crate::architecture_navigation::Category>,
) -> f32 {
    let height = Contents::new(registry, mode, tab, editing, category).height();
    if mode == Mode::Model {
        height.max(icons::TOOL_TILE.y + SECTION_LABEL)
    } else {
        height
    }
}

fn top_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), add)
}
fn separator(ui: &mut egui::Ui, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.shrink2(egui::vec2(0., 4.)).y_range(),
        egui::Stroke::new(1., egui::Color32::from_gray(65)),
    );
}
fn section_label(ui: &mut egui::Ui, label: &str) {
    if label.is_empty() {
        return;
    }
    let width = ui.available_width().max(1.);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, SECTION_LABEL), egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Small.resolve(ui.style()),
        egui::Color32::from_gray(160),
    );
}
pub fn show(
    ui: &mut egui::Ui,
    registry: &Registry,
    mode: Mode,
    tab: &RibbonTab,
    editing: bool,
    enabled: impl Fn(Command) -> bool,
    hint: impl Fn(Command, &str) -> String,
) -> Option<Command> {
    show_selectable(ui, registry, mode, tab, editing, enabled, hint, |_| false)
}
/// Glyph for a registered tool: reviewed catalog artwork when bound,
/// otherwise its retained vector symbol.
pub fn tool_glyph(mode: Mode, id: &str, fallback: Icon) -> icons::Glyph {
    crate::tool_artwork::tool_artwork(mode, id)
        .map(|(art, svg)| icons::Glyph::Art(art, svg))
        .unwrap_or(icons::Glyph::Vector(fallback))
}
fn menu_memory(mode: Mode, label: &str) -> egui::Id {
    egui::Id::new(("ribbon-menu-choice", mode.title(), label))
}
/// The tool a menu button currently represents: the last choice made from
/// that menu while it is still offered, otherwise the first entry.
pub fn menu_choice(
    ctx: &egui::Context,
    mode: Mode,
    label: &str,
    tools: &[&'static str],
) -> &'static str {
    let remembered = ctx.data(|data| data.get_temp::<&'static str>(menu_memory(mode, label)));
    remembered
        .and_then(|id| tools.iter().copied().find(|tool| *tool == id))
        .or_else(|| tools.first().copied())
        .unwrap_or_default()
}
fn with_description(id: &str, text: String) -> String {
    match crate::tool_artwork::description(id) {
        Some(more) => format!("{text}\n{more}"),
        None => text,
    }
}
#[allow(clippy::too_many_arguments)]
fn tool_button(
    ui: &mut egui::Ui,
    registry: &Registry,
    mode: Mode,
    id: &str,
    tile_width: f32,
    enabled: &impl Fn(Command) -> bool,
    hint: &impl Fn(Command, &str) -> String,
    selected: &impl Fn(Command) -> bool,
) -> Option<Command> {
    let tool = registry.tool(id).expect("catalog entry");
    let response = ui
        .add_enabled_ui(enabled(tool.action), |ui| {
            icons::tool_glyph_at_width(
                ui,
                tool_glyph(mode, id, tool.icon),
                tool.label,
                &with_description(id, hint(tool.action, tool.label)),
                selected(tool.action),
                tile_width,
            )
        })
        .inner;
    response
        .on_disabled_hover_text(match tool.action {
            Command::Modeling(tool) => tool.requirements().to_string(),
            Command::SheetFlange | Command::SheetFasteners | Command::SheetView => {
                hint(tool.action, tool.label)
            }
            _ => "Unavailable in the current context".into(),
        })
        .clicked()
        .then_some(tool.action)
}
#[allow(clippy::too_many_arguments)]
fn menu_button(
    ui: &mut egui::Ui,
    registry: &Registry,
    mode: Mode,
    label: &'static str,
    icon: Icon,
    tools: &[&'static str],
    unavailable: Option<(&'static str, &'static str)>,
    split: bool,
    tile_width: f32,
    enabled: &impl Fn(Command) -> bool,
    hint: &impl Fn(Command, &str) -> String,
    selected: &impl Fn(Command) -> bool,
) -> Option<Command> {
    let is_selected = |id: &&str| registry.tool(id).is_some_and(|tool| selected(tool.action));
    let mut action = None;
    let response = if split {
        const ARROW_WIDTH: f32 = 18.;
        let default = registry.tool(tools[0]).expect("menu default");
        top_row(ui, |ui| {
            let (rect, slot) = ui.allocate_exact_size(
                egui::vec2(tile_width, icons::TOOL_TILE.y),
                egui::Sense::hover(),
            );
            let caret = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(2., 45.),
                egui::vec2(ARROW_WIDTH - 2., 21.),
            );
            let arrow = ui.interact(caret, slot.id.with("options"), egui::Sense::click());
            let main = ui
                .add_enabled_ui(enabled(default.action), |ui| {
                    let hit =
                        egui::Rect::from_min_max(rect.min + egui::vec2(ARROW_WIDTH, 0.), rect.max);
                    let main = ui.interact(hit, slot.id.with("default"), egui::Sense::click());
                    icons::paint_tool_glyph(
                        ui,
                        rect,
                        main,
                        tool_glyph(mode, default.id, default.icon),
                        label,
                        &with_description(default.id, hint(default.action, default.label)),
                        tools.iter().any(is_selected),
                        ARROW_WIDTH,
                    )
                })
                .inner
                .on_disabled_hover_text(
                    "The default is unavailable in this context. Open the options for other tools.",
                );
            if arrow.hovered() || arrow.has_focus() {
                ui.painter()
                    .rect_filled(caret, 3, egui::Color32::from_rgb(56, 67, 84));
            }
            if arrow.has_focus() {
                ui.painter().rect_stroke(
                    caret,
                    3,
                    egui::Stroke::new(1., egui::Color32::LIGHT_BLUE),
                    egui::StrokeKind::Inside,
                );
            }
            paint_menu_arrow(ui, caret.center());
            let options = format!("{label} options");
            arrow.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &options)
            });
            if main.clicked() {
                if mode == Mode::Sketch {
                    main.request_focus();
                }
                action = Some(default.action);
            }
            if mode == Mode::Sketch
                && let Some(focused) = [&arrow, &main]
                    .into_iter()
                    .find(|target| target.has_focus())
            {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(SKETCH_SPLIT_FOCUS), focused.id);
                });
            }
            arrow.on_hover_text(options)
        })
        .inner
    } else {
        let glyph = if mode == Mode::Model {
            icons::Glyph::Vector(icon)
        } else {
            let choice = tools
                .iter()
                .copied()
                .find(is_selected)
                .unwrap_or_else(|| menu_choice(ui.ctx(), mode, label, tools));
            registry
                .tool(choice)
                .map(|tool| tool_glyph(mode, choice, tool.icon))
                .filter(|glyph| matches!(glyph, icons::Glyph::Art(..)))
                .unwrap_or(icons::Glyph::Vector(icon))
        };
        let response = icons::tool_glyph_at_width(
            ui,
            glyph,
            label,
            &format!("{label} · choose a tool"),
            tools.iter().any(is_selected),
            tile_width,
        );
        let caret = if mode == Mode::Model {
            response.rect.left_top() + egui::vec2(9., 12.)
        } else {
            response.rect.right_top() + egui::vec2(-9., 12.)
        };
        paint_menu_arrow(ui, caret);
        response
    };
    let compact = split || mode == Mode::Model;
    let mut focus_first = compact && (!split || mode == Mode::Sketch) && response.clicked();
    egui::Popup::menu(&response).show(|ui| {
        if compact {
            ui.set_width(178.);
            ui.spacing_mut().item_spacing.y = 1.;
        }
        for id in tools {
            let tool = registry.tool(id).expect("menu catalog entry");
            let available = enabled(tool.action);
            let option = ui
                .add_enabled_ui(available, |ui| {
                    icons::labeled_glyph_button_sized(
                        ui,
                        if mode == Mode::Sketch {
                            tool_glyph(mode, tool.id, tool.icon)
                        } else {
                            icons::Glyph::Vector(tool.icon)
                        },
                        tool.label,
                        egui::vec2(
                            ui.available_width().max(160.),
                            if compact { 24. } else { 28. },
                        ),
                    )
                })
                .inner
                .on_hover_text(with_description(id, hint(tool.action, tool.label)))
                .on_disabled_hover_text(if *id == "duplicate" {
                    "Select current Parts or Instances and finish any active tool before duplicating"
                } else {
                    "Unavailable in the current context"
                });
            if focus_first && available {
                option.request_focus();
                focus_first = false;
            }
            if option.clicked() {
                action = Some(tool.action);
                let id: &'static str = id;
                if !split {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(menu_memory(mode, label), id));
                }
                ui.close();
            }
        }
        if let Some((label, reason)) = unavailable {
            ui.separator();
            ui.add_enabled(false, egui::Button::new(label))
                .on_disabled_hover_text(reason);
            ui.add(egui::Label::new(reason).wrap());
        }
    });
    action
}

fn paint_menu_arrow(ui: &egui::Ui, center: egui::Pos2) {
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(-3., -2.),
            center + egui::vec2(3., -2.),
            center + egui::vec2(0., 2.),
        ],
        egui::Color32::LIGHT_GRAY,
        egui::Stroke::NONE,
    ));
}
/// Ribbon with per-command selected state (for example the active drawing
/// tool or construction mode).
#[allow(clippy::too_many_arguments)]
pub fn show_selectable(
    ui: &mut egui::Ui,
    registry: &Registry,
    mode: Mode,
    tab: &RibbonTab,
    editing: bool,
    enabled: impl Fn(Command) -> bool,
    hint: impl Fn(Command, &str) -> String,
    selected: impl Fn(Command) -> bool,
) -> Option<Command> {
    show_selectable_with_category(
        ui, registry, mode, tab, editing, enabled, hint, selected, None,
    )
}
#[allow(clippy::too_many_arguments)]
pub fn show_selectable_with_category(
    ui: &mut egui::Ui,
    registry: &Registry,
    mode: Mode,
    tab: &RibbonTab,
    editing: bool,
    enabled: impl Fn(Command) -> bool,
    hint: impl Fn(Command, &str) -> String,
    selected: impl Fn(Command) -> bool,
    category: Option<crate::architecture_navigation::Category>,
) -> Option<Command> {
    if mode == Mode::Sketch {
        ui.ctx().data_mut(|data| {
            data.remove::<egui::Id>(egui::Id::new(SKETCH_SPLIT_FOCUS));
        });
    }
    let contents = Contents::new(registry, mode, tab, editing, category);
    let height = contents.height();
    let width = ui.available_width();
    let normal_width = contents.sections_width(icons::TOOL_TILE.x, SEPARATOR)
        + contents.pinned_width(icons::TOOL_TILE.x, SEPARATOR);
    let (tile_width, separator_width) = if normal_width <= width {
        (icons::TOOL_TILE.x, SEPARATOR)
    } else {
        (COMPACT_TILE_WIDTH, COMPACT_SEPARATOR)
    };
    let pinned_width = contents.pinned_width(tile_width, separator_width);
    let section_width = (width - pinned_width).max(0.);
    let overflows = contents.sections_width(tile_width, separator_width) > section_width;
    let scroll_width = (section_width - if overflows { SCROLL_CONTROL_WIDTH } else { 0. }).max(0.);
    let mut action = None;

    top_row(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let scrolled = ui.allocate_ui(egui::vec2(scroll_width, height), |ui| {
            ui.set_width(scroll_width);
            if contents.sections.is_empty() {
                let message = if mode == Mode::Sketch && !editing {
                    "Choose a plane in the viewer, or open an existing sketch, to see its tools."
                } else if mode == Mode::Sketch && editing {
                    "No further tools on this tab."
                } else {
                    "No tools available in this workspace yet."
                };
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(scroll_width, height),
                    egui::Sense::hover(),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    message,
                    egui::FontId::proportional(12.),
                    egui::Color32::GRAY,
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), message)
                });
                return None;
            }
            Some(egui::ScrollArea::horizontal()
                .id_salt(("workspace-ribbon", mode.title(), tab.title, category.map(|c| c.id())))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    top_row(ui, |ui| {
                        for (index, section) in contents.sections.iter().enumerate() {
                            if index > 0 {
                                separator(ui, separator_width, height);
                            }
                            ui.vertical(|ui| {
                                ui.set_width(section.entries.len() as f32 * tile_width);
                                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                                top_row(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 0.;
                                    for entry in &section.entries {
                                        let chosen = match entry {
                                            Entry::Tool(id) => tool_button(
                                                ui, registry, mode, id, tile_width, &enabled, &hint, &selected,
                                            ),
                                            Entry::Menu {
                                                label,
                                                icon,
                                                tools,
                                                unavailable,
                                                split,
                                            } => menu_button(
                                                ui, registry, mode, label, *icon, tools, *unavailable, *split,
                                                tile_width, &enabled, &hint, &selected,
                                            ),
                                        };
                                        if chosen.is_some() {
                                            action = chosen;
                                        }
                                    }
                                });
                                section_label(ui, section.label);
                            });
                        }
                    });
                }))
        }).inner;
        if let Some(output) = scrolled.filter(|_| overflows) {
            show_scroll_controls(ui, output, height, tile_width);
        }
        if !contents.pinned.is_empty() {
            separator(ui, separator_width, height);
            ui.vertical(|ui| {
                ui.set_width(contents.pinned.len() as f32 * tile_width);
                top_row(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    for id in &contents.pinned {
                        if let Some(command) = tool_button(
                            ui, registry, mode, id, tile_width, &enabled, &hint, &selected,
                        ) {
                            action = Some(command);
                        }
                    }
                });
                section_label(ui, "Sketch");
            });
        }
    });
    action
}

fn show_scroll_controls(
    ui: &mut egui::Ui,
    output: egui::scroll_area::ScrollAreaOutput<()>,
    height: f32,
    tile_width: f32,
) {
    let mut state = output.state;
    let max = (output.content_size.x - output.inner_rect.width()).max(0.);
    let step = (output.inner_rect.width() - tile_width).max(tile_width);
    let mut offset = state.offset.x;
    ui.allocate_ui_with_layout(
        egui::vec2(SCROLL_CONTROL_WIDTH, height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.;
            ui.add_space((height - 56.) * 0.5);
            for (glyph, label, available, delta) in [
                ("‹", "Previous ribbon tools", offset > 0.5, -step),
                ("›", "Next ribbon tools", offset < max - 0.5, step),
            ] {
                let response = ui.add_enabled(
                    available,
                    egui::Button::new(glyph).min_size(egui::vec2(24., 26.)),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, available, label)
                });
                if response.on_hover_text(label).clicked() {
                    offset = (offset + delta).clamp(0., max);
                }
            }
        },
    );
    if offset != state.offset.x {
        state.offset.x = offset;
        state.store(ui.ctx(), output.id);
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        run: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        frame_at(ctx, 1200., events, run)
    }

    fn frame_at(
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
        run: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.),
                )),
                events,
                ..Default::default()
            },
            run,
        );
        output.textures_delta.clear();
        output
    }
    fn text(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().replace('\n', " ") == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing ribbon label {label}"))
    }
    fn events(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    }
    fn fully_visible(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().replace('\n', " ") == label => {
                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                shape.clip_rect.expand(0.5).contains_rect(rect)
            }
            _ => false,
        })
    }

    #[test]
    fn workbench_and_sheet_fastener_symbols_describe_their_tools() {
        assert!(matches!(
            workspace_tab_icon(Mode::Model, "Architecture"),
            Some(Icon::ArchitectureTower)
        ));
        assert!(matches!(
            workspace_tab_icon(Mode::Model, "Wood"),
            Some(Icon::Timber)
        ));
        assert!(matches!(
            workspace_tab_icon(Mode::Model, "Mechanical"),
            Some(Icon::MechanicalScrew)
        ));
        let registry = Registry::default();
        let fastener = registry.tool("fastener").unwrap();
        assert!(matches!(
            tool_glyph(Mode::Model, fastener.id, fastener.icon),
            icons::Glyph::Vector(Icon::SheetFasteners)
        ));
    }

    #[test]
    fn sheet_view_and_bend_schedule_are_properties_instead_of_ribbon_tiles() {
        let registry = Registry::default();
        let metal = registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .find(|tab| tab.title == METAL_TITLE)
            .unwrap();
        assert!(!metal.tools.contains(&"flat"));
        assert!(!metal.tools.contains(&"bend-list"));
        assert_eq!(
            registry.resolve("unfold").unwrap().action,
            Command::SheetView
        );
        assert_eq!(registry.resolve("fold").unwrap().label, "Fold/Unfold");
    }

    #[test]
    fn model_workbenches_and_contextual_sketch_fit_at_1024_logical_points() {
        let registry = Registry::default();
        for mode in [Mode::Model, Mode::Sketch] {
            for tab in &registry.workspace(mode).tabs {
                let categories = if mode == Mode::Model && tab.title == "Architecture" {
                    std::iter::once(None)
                        .chain(
                            crate::architecture_navigation::Category::ALL
                                .into_iter()
                                .map(Some),
                        )
                        .collect::<Vec<_>>()
                } else {
                    vec![None]
                };
                for category in categories {
                    let editing = mode == Mode::Sketch;
                    let contents = Contents::new(&registry, mode, tab, editing, category);
                    let ctx = egui::Context::default();
                    let run = |ui: &mut egui::Ui| {
                        show_selectable_with_category(
                            ui,
                            &registry,
                            mode,
                            tab,
                            editing,
                            |_| true,
                            |_, label| label.into(),
                            |_| false,
                            category,
                        );
                    };
                    let width = 1024. - 2. * f32::from(crate::theme::GUTTER);
                    frame_at(&ctx, width, vec![], run).drop_without_applying_deltas();
                    let output = frame_at(&ctx, width, vec![], run);
                    let labels = contents
                        .sections
                        .iter()
                        .flat_map(|group| {
                            group.entries.iter().map(|entry| match entry {
                                Entry::Tool(id) => registry.tool(id).unwrap().label,
                                Entry::Menu { label, .. } => label,
                            })
                        })
                        .chain(
                            contents
                                .pinned
                                .iter()
                                .map(|id| registry.tool(id).unwrap().label),
                        );
                    for label in labels {
                        assert!(
                            fully_visible(&output, label),
                            "{} / {} / {category:?}: {label} clipped",
                            mode.title(),
                            tab.title
                        );
                    }
                    assert!(!fully_visible(&output, "›"));
                    output.drop_without_applying_deltas();
                }
            }
        }
    }

    #[test]
    fn overflow_buttons_reach_sketch_tools_while_session_actions_stay_pinned() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Sketch).tabs[0];
        let ctx = egui::Context::default();
        let mut chosen = None;
        let mut draw = |events| {
            frame_at(&ctx, 550., events, |ui| {
                if let Some(command) = show(
                    ui,
                    &registry,
                    Mode::Sketch,
                    tab,
                    true,
                    |_| true,
                    |_, label| label.into(),
                ) {
                    chosen = Some(command);
                }
            })
        };
        let finish = registry.tool("finish-sketch").unwrap();
        let abort = registry.tool("abort-sketch").unwrap();
        let last = registry.tool("delete").unwrap();
        draw(vec![]).drop_without_applying_deltas();
        let mut output = draw(vec![]);
        assert!(fully_visible(&output, "Rectangle"));
        assert!(!fully_visible(&output, last.label));
        let pinned = text(&output, finish.label);
        // The real Common catalog can span more than one overflow page.
        for _ in 0..4 {
            assert!(fully_visible(&output, finish.label));
            assert!(fully_visible(&output, abort.label));
            assert_eq!(text(&output, finish.label), pinned);
            if fully_visible(&output, last.label) {
                break;
            }
            let next = text(&output, "›");
            output.drop_without_applying_deltas();
            draw(events(next, true)).drop_without_applying_deltas();
            draw(events(next, false)).drop_without_applying_deltas();
            output = draw(vec![]);
        }
        assert!(fully_visible(&output, last.label));
        assert!(!fully_visible(&output, "Rectangle"));
        let position = text(&output, last.label);
        output.drop_without_applying_deltas();
        draw(events(position, true)).drop_without_applying_deltas();
        draw(events(position, false)).drop_without_applying_deltas();
        assert_eq!(chosen, Some(last.action));
    }

    #[test]
    fn primitive_menu_routes_a_real_choice_and_modifiers_share_a_section() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Model).tabs[0];
        let sections = groups(Mode::Model, tab);
        let modify = sections.iter().find(|g| g.label == "Modify").unwrap();
        assert!(
            modify
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Menu { label: "Boolean", tools, .. }
                    if tools == &["boolean-join", "boolean-subtract", "boolean-intersect", "split"]))
        );
        assert!(
            !modify
                .entries
                .iter()
                .any(|entry| matches!(entry, Entry::Tool("split")))
        );
        let transform = sections
            .iter()
            .find(|group| group.label == "Transform")
            .unwrap();
        assert!(transform.entries.iter().any(|entry| matches!(entry, Entry::Menu { label: "Move / Copy", tools, .. } if tools == &["move", "duplicate", "align"])));
        assert!(
            !transform
                .entries
                .iter()
                .any(|entry| matches!(entry, Entry::Tool("align")))
        );
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut chosen = None;
        let mut draw = |ui: &mut egui::Ui| {
            if let Some(command) = show(
                ui,
                &registry,
                Mode::Model,
                tab,
                false,
                |_| true,
                |_, label| label.into(),
            ) {
                chosen = Some(command);
            }
        };
        frame(&ctx, vec![], &mut draw).drop_without_applying_deltas();
        let output = frame(&ctx, vec![], &mut draw);
        let menu = accessible_button(&output, "Add Primitive options");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(&ctx, events(menu, pressed), &mut draw).drop_without_applying_deltas();
        }
        frame(&ctx, vec![], &mut draw).drop_without_applying_deltas();
        let output = frame(&ctx, vec![], &mut draw);
        let cylinder = text(&output, "Cylinder");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(&ctx, events(cylinder, pressed), &mut draw).drop_without_applying_deltas();
        }
        assert_eq!(
            chosen,
            Some(Command::Modeling(crate::modeling::ModelingTool::Cylinder))
        );
    }
    #[test]
    fn move_copy_options_dispatch_duplicate_only_for_compatible_selection() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Model).tabs[0];
        let command = Command::Extension("duplicate");
        for available in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut chosen = None;
            let mut draw = |ui: &mut egui::Ui| {
                if let Some(action) = show(
                    ui,
                    &registry,
                    Mode::Model,
                    tab,
                    false,
                    |action| action != command || available,
                    |_, label| label.into(),
                ) {
                    chosen = Some(action);
                }
            };
            frame(&ctx, vec![], &mut draw).drop_without_applying_deltas();
            let output = frame(&ctx, vec![], &mut draw);
            let arrow = accessible_button(&output, "Move / Copy options");
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                frame(&ctx, events(arrow, pressed), &mut draw).drop_without_applying_deltas();
            }
            frame(&ctx, vec![], &mut draw).drop_without_applying_deltas();
            let output = frame(&ctx, vec![], &mut draw);
            let (_, node) = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Duplicate"))
                .expect("Duplicate option");
            assert_eq!(node.is_disabled(), !available);
            let duplicate = text(&output, "Duplicate");
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                frame(&ctx, events(duplicate, pressed), &mut draw).drop_without_applying_deltas();
            }
            assert_eq!(chosen, available.then_some(command));
        }
    }
    #[test]
    fn model_split_buttons_run_each_default_directly_at_compact_width() {
        let registry = Registry::default();
        let model = registry.workspace(Mode::Model);
        for (tab, label, id) in [
            ("Common", "Add Primitive", "box"),
            ("Common", "Boolean", "boolean-join"),
            ("Common", "Move / Copy", "move"),
            ("Common", "Pattern", "model-linear-pattern"),
            ("Wood", "Screw / Hardware", "wood-hardware"),
        ] {
            let tab = model
                .tabs
                .iter()
                .find(|candidate| candidate.title == tab)
                .unwrap();
            assert_eq!(
                click(&registry, Mode::Model, tab, false, 1024., label),
                Some(registry.tool(id).unwrap().action),
                "{label} must dispatch its default without opening options"
            );
        }
    }
    #[test]
    fn split_arrows_are_left_of_their_main_action_at_both_widths() {
        let registry = Registry::default();
        for (mode, labels) in [
            (Mode::Model, ["Add Primitive", "Move / Copy"]),
            (Mode::Sketch, ["Rectangle", "Circle"]),
        ] {
            let tab = &registry.workspace(mode).tabs[0];
            for width in [1024., 1360.] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let draw = || {
                    ribbon(
                        &registry,
                        mode,
                        tab,
                        mode == Mode::Sketch,
                        width,
                        vec![],
                        &ctx,
                    )
                    .0
                };
                draw().drop_without_applying_deltas();
                let out = draw();
                for label in labels {
                    let options = accessible_button(&out, &format!("{label} options"));
                    assert!(options.x < text(&out, label).x, "{label} at {width}");
                }
                out.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn model_split_arrow_is_keyboard_focusable_and_escape_dismisses_its_options() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Model).tabs[0];
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let draw = |events| ribbon(&registry, Mode::Model, tab, false, 1024., events, &ctx);
        draw(vec![]).0.drop_without_applying_deltas();
        let (out, _) = draw(vec![]);
        let (id, _) = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Add Primitive options"))
            .unwrap();
        let focus = egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: *id,
            data: None,
        });
        out.drop_without_applying_deltas();
        draw(vec![focus]).0.drop_without_applying_deltas();
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (out, command) = draw(vec![key(egui::Key::Space)]);
        assert!(command.is_none());
        out.drop_without_applying_deltas();
        let (out, _) = draw(vec![]);
        assert!(labels(&out).contains(&"Cylinder".into()));
        out.drop_without_applying_deltas();
        draw(vec![key(egui::Key::Escape)])
            .0
            .drop_without_applying_deltas();
        let (out, command) = draw(vec![]);
        assert!(command.is_none());
        assert!(!labels(&out).contains(&"Cylinder".into()));
        out.drop_without_applying_deltas();
    }
    #[test]
    fn empty_workspace_has_centered_guidance_instead_of_an_empty_strip() {
        let registry = Registry::default();
        let tab = RibbonTab {
            title: "Empty",
            tools: vec![],
            requires_active_sketch: false,
        };
        let ctx = egui::Context::default();
        let output = frame(&ctx, vec![], |ui| {
            assert!(
                show(
                    ui,
                    &registry,
                    Mode::Model,
                    &tab,
                    false,
                    |_| true,
                    |_, label| label.into()
                )
                .is_none()
            );
        });
        let center = text(&output, "No tools available in this workspace yet.");
        assert!((center.x - 600.).abs() < 16.);
        assert!((center.y - EMPTY_HEIGHT / 2.).abs() < 1.);
        output.drop_without_applying_deltas();
    }
    fn textured_meshes(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(&shape.shape, egui::Shape::Mesh(mesh)
                    if mesh.texture_id != egui::TextureId::default())
            })
            .count()
    }
    #[test]
    fn reviewed_artwork_is_painted_as_tinted_textures_with_state_colors() {
        let registry = Registry::default();
        let tab = registry
            .workspace(Mode::Parameters)
            .tabs
            .iter()
            .find(|tab| tab.title == "Parameters")
            .unwrap();
        let ctx = egui::Context::default();
        let mut draw = |ui: &mut egui::Ui| {
            show_selectable(
                ui,
                &registry,
                Mode::Parameters,
                tab,
                false,
                |command| command != Command::Discard,
                |_, label| label.into(),
                |command| command == Command::Apply,
            );
        };
        let output = frame(&ctx, vec![], &mut draw);
        // Apply, Discard and Preview use catalog artwork; Undo/Redo keep
        // their retained vector motifs.
        assert_eq!(textured_meshes(&output), 3);
        let tints: Vec<egui::Color32> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                    mesh.vertices.first().map(|v| v.color)
                }
                _ => None,
            })
            .collect();
        assert!(tints.contains(&icons::TOOL_SELECTED), "selected Apply");
        // egui additionally fades disabled widgets; the tint stays neutral grey.
        assert!(
            tints
                .iter()
                .any(|c| c.r() == c.g() && c.g() == c.b() && c.r() <= icons::TOOL_DISABLED.r()),
            "disabled Discard: {tints:?}"
        );
        assert!(tints.contains(&icons::TOOL_COLOR), "enabled Preview");
        output.drop_without_applying_deltas();
        let finish = crate::tool_artwork::artwork("finish").unwrap();
        let glyph = icons::Glyph::Art("finish", finish);
        assert_eq!(
            icons::glyph_color(glyph, icons::ToolState::Hovered),
            egui::Color32::from_rgb(0x24, 0xaa, 0x60)
        );
        assert_eq!(
            icons::glyph_color(glyph, icons::ToolState::Disabled),
            icons::TOOL_DISABLED
        );
    }
    #[test]
    fn menu_button_represents_the_last_offered_choice() {
        let ctx = egui::Context::default();
        let tools = ["boolean-join", "boolean-subtract", "boolean-intersect"];
        assert_eq!(
            menu_choice(&ctx, Mode::Model, "Boolean", &tools),
            "boolean-join"
        );
        ctx.data_mut(|data| {
            data.insert_temp(menu_memory(Mode::Model, "Boolean"), "boolean-intersect")
        });
        assert_eq!(
            menu_choice(&ctx, Mode::Model, "Boolean", &tools),
            "boolean-intersect"
        );
        assert_eq!(
            menu_choice(&ctx, Mode::Model, "Boolean", &tools[..2]),
            "boolean-join",
            "a choice no longer offered falls back to the first entry"
        );
    }
    fn labels(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().replace('\n', " ")),
                _ => None,
            })
            .collect()
    }
    fn accessible_button(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| {
                let bounds = node.bounds()?;
                (node.label() == Some(label)).then(|| {
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) * 0.5) as f32,
                        ((bounds.y0 + bounds.y1) * 0.5) as f32,
                    )
                })
            })
            .unwrap_or_else(|| panic!("Missing ribbon button {label}"))
    }
    /// Regression for the reported empty Wood / Sheet metal workspaces: in
    /// builds between eff1e2d and 5dffdb3 / f9988f9 every entry on those tabs
    /// was an unimplemented extension placeholder, so the ribbon showed only
    /// the empty-workspace guidance. Every built-in tab must now render at
    /// least one implemented tool.
    #[test]
    fn every_workspace_tab_renders_implemented_tools_including_wood_and_sheet_metal() {
        let registry = Registry::default();
        for workspace in &registry.workspaces {
            for tab in &workspace.tabs {
                if workspace.mode == Mode::Parameters && matches!(tab.title, "General" | "Export") {
                    continue;
                }
                let ctx = egui::Context::default();
                let output = frame(&ctx, vec![], |ui| {
                    show(
                        ui,
                        &registry,
                        workspace.mode,
                        tab,
                        true,
                        |_| false,
                        |_, label| label.into(),
                    );
                });
                let shown = labels(&output);
                output.drop_without_applying_deltas();
                assert!(
                    !shown
                        .iter()
                        .any(|label| label == "No tools available in this workspace yet."),
                    "{} / {} renders no tools",
                    workspace.mode.title(),
                    tab.title
                );
                let expected: &[&str] = match (workspace.mode, tab.title) {
                    (Mode::Model, "Wood") => &[
                        "Create",
                        "Modify",
                        "Assistance",
                        "Panel",
                        "Batten",
                        "Screw / Hardware",
                        "Drill",
                        "Pocket",
                        "Joint",
                    ],
                    (Mode::Model, METAL_TITLE) => &[
                        "Create sheet part",
                        "Add flange",
                        "Attached fasteners",
                        "Tube / profile",
                        "Miter / cope",
                        "Weld joint",
                    ],
                    (Mode::Model, "Other") => &["Surface Structure"],
                    (Mode::Export, "Common") => &["Add export", "Add 3D print plate"],
                    (Mode::Export, "Print") => &["Write 3MF package"],
                    _ => &[],
                };
                for label in expected {
                    assert!(
                        shown.iter().any(|shown| shown == label),
                        "{label} missing from {}: {shown:?}",
                        tab.title
                    );
                }
                // Placeholders stay hidden instead of appearing implemented.
                for placeholder in ["Cut list", "Weld sequence"] {
                    assert!(!shown.iter().any(|shown| shown == placeholder));
                }
            }
        }
    }
    /// CAD-154: every catalog label fits a tile in two lines without
    /// elision, so labels and their padding stay inside the tile.
    #[test]
    fn every_catalog_label_fits_two_lines_inside_its_tile() {
        let registry = Registry::default();
        let ctx = egui::Context::default();
        frame(&ctx, vec![], |ui| {
            for workspace in &registry.workspaces {
                for tab in &workspace.tabs {
                    let tiles = groups(workspace.mode, tab).into_iter().flat_map(|group| {
                        group.entries.into_iter().map(|entry| match entry {
                            Entry::Tool(id) => registry.tool(id).unwrap().label,
                            Entry::Menu { label, .. } => label,
                        })
                    });
                    for label in tiles {
                        for width in [icons::TOOL_TILE.x, COMPACT_TILE_WIDTH] {
                            let galley = ui.painter().layout(
                                label.into(),
                                egui::FontId::proportional(10.),
                                egui::Color32::WHITE,
                                width - 6.,
                            );
                            assert!(
                                galley.rows.len() <= 2,
                                "{label} needs elision at {width} pt"
                            );
                            let shown =
                                icons::tile_label_at_width(ui, label, egui::Color32::WHITE, width);
                            assert!(shown.rows.len() <= 2, "{label}");
                            if label == "Construction" {
                                assert_eq!(shown.rows.len(), 1, "Construction at {width} pt");
                            }
                            assert!(
                                icons::TOOL_LABEL_TOP + shown.size().y + 4. <= icons::TOOL_TILE.y,
                                "{label} leaves no bottom padding"
                            );
                        }
                    }
                }
            }
        })
        .drop_without_applying_deltas();
    }
    #[test]
    fn sketch_split_actions_keep_all_variants_and_dispatch_fixed_defaults() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Sketch).tabs[0];
        let entries = &groups(Mode::Sketch, tab)[0].entries;
        let menu = |name: &str| {
            entries.iter().find_map(|entry| match entry {
                Entry::Menu {
                    label,
                    tools,
                    split: true,
                    ..
                } if *label == name => Some(tools.clone()),
                _ => None,
            })
        };
        assert_eq!(
            menu("Rectangle").unwrap(),
            vec![
                "rectangle",
                "rectangle-center",
                "rectangle-center-two",
                "rectangle-three"
            ]
        );
        assert_eq!(
            menu("Circle").unwrap(),
            vec!["circle", "circle-two-radius", "circle-three"]
        );
        assert!(
            !entries
                .iter()
                .any(|e| matches!(e, Entry::Tool("rectangle" | "circle-three")))
        );
        assert!(entries.iter().any(|e| matches!(e, Entry::Tool("line"))));
        for (label, tool) in [
            ("Rectangle", crate::sketch_editor::DrawTool::Rectangle),
            ("Circle", crate::sketch_editor::DrawTool::Circle),
        ] {
            for width in [1024., 1360.] {
                assert_eq!(
                    click(&registry, Mode::Sketch, tab, true, width, label),
                    Some(Command::Draw(tool))
                );
            }
        }
    }
    #[test]
    fn additional_catalog_tools_are_not_lost_by_common_grouping() {
        let tab = RibbonTab {
            title: "Common",
            tools: vec!["box", "cylinder", "new-module-tool"],
            requires_active_sketch: false,
        };
        assert!(groups(Mode::Model, &tab).iter().any(|group| {
            group
                .entries
                .iter()
                .any(|entry| matches!(entry, Entry::Tool("new-module-tool")))
        }));
    }
    /// Labels whose centre is visible (not scrolled out of the ribbon).
    fn positioned_labels(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if shape
                        .clip_rect
                        .contains(text.pos + text.galley.size() * 0.5) =>
                {
                    Some((
                        text.galley.text().replace('\n', " "),
                        text.pos + text.galley.size() * 0.5,
                    ))
                }
                _ => None,
            })
            .collect()
    }
    fn ribbon(
        registry: &Registry,
        mode: Mode,
        tab: &RibbonTab,
        editing: bool,
        width: f32,
        events: Vec<egui::Event>,
        ctx: &egui::Context,
    ) -> (egui::FullOutput, Option<Command>) {
        let mut chosen = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                chosen = show(ui, registry, mode, tab, editing, |_| true, |_, l| l.into());
            },
        );
        (output, chosen)
    }
    fn click(
        registry: &Registry,
        mode: Mode,
        tab: &RibbonTab,
        editing: bool,
        width: f32,
        label: &str,
    ) -> Option<Command> {
        let ctx = egui::Context::default();
        ribbon(registry, mode, tab, editing, width, vec![], &ctx)
            .0
            .drop_without_applying_deltas();
        let (output, _) = ribbon(registry, mode, tab, editing, width, vec![], &ctx);
        let pos = text(&output, label);
        output.drop_without_applying_deltas();
        let mut chosen = None;
        for pressed in [true, false] {
            let (output, command) = ribbon(
                registry,
                mode,
                tab,
                editing,
                width,
                events(pos, pressed),
                &ctx,
            );
            output.drop_without_applying_deltas();
            chosen = chosen.or(command);
        }
        chosen
    }
    #[test]
    fn wood_hardware_menu_routes_parts_library_and_canonical_connections() {
        let registry = Registry::default();
        let tab = registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .find(|tab| tab.title == "Wood")
            .unwrap();
        for (label, expected) in [
            (
                "Configure screw / hardware",
                Command::WoodPreset(crate::wood_navigation::Preset::Hardware),
            ),
            ("Browse Library", Command::Extension("library")),
            (
                "Attach screw connection",
                Command::Modeling(crate::modeling::ModelingTool::ScrewConnection),
            ),
        ] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            ribbon(&registry, Mode::Model, tab, false, 1360., vec![], &ctx)
                .0
                .drop_without_applying_deltas();
            let output = ribbon(&registry, Mode::Model, tab, false, 1360., vec![], &ctx).0;
            let pos = accessible_button(&output, "Screw / Hardware options");
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                ribbon(
                    &registry,
                    Mode::Model,
                    tab,
                    false,
                    1360.,
                    events(pos, pressed),
                    &ctx,
                )
                .0
                .drop_without_applying_deltas();
            }
            let output = ribbon(&registry, Mode::Model, tab, false, 1360., vec![], &ctx).0;
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Attach screw connection")
                        && !node.is_disabled())
            );
            let pos = text(&output, label);
            output.drop_without_applying_deltas();
            let mut chosen = None;
            for pressed in [true, false] {
                let (output, command) = ribbon(
                    &registry,
                    Mode::Model,
                    tab,
                    false,
                    1360.,
                    events(pos, pressed),
                    &ctx,
                );
                output.drop_without_applying_deltas();
                chosen = chosen.or(command);
            }
            assert_eq!(chosen, Some(expected));
        }
    }
    #[test]
    fn wood_assistance_opens_without_dispatch_and_routes_all_three_choices() {
        let registry = Registry::default();
        let tab = registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .find(|tab| tab.title == "Wood")
            .unwrap();
        let choices = [
            ("System 32 shelf pins", "system32"),
            ("Cabinet", "wood-cabinet"),
            ("Box", "wood-box"),
        ];
        for (label, id) in choices {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let draw = |events| ribbon(&registry, Mode::Model, tab, false, 1024., events, &ctx);
            let press = |point| {
                let mut chosen = None;
                for pressed in [true, false] {
                    let (out, command) = draw(events(point, pressed));
                    chosen = chosen.or(command);
                    out.drop_without_applying_deltas();
                }
                chosen
            };
            draw(vec![]).0.drop_without_applying_deltas();
            let (closed, _) = draw(vec![]);
            let closed_labels = labels(&closed);
            assert_eq!(
                closed_labels
                    .iter()
                    .filter(|label| *label == "Assistance")
                    .count(),
                1
            );
            assert!(
                choices
                    .iter()
                    .all(|(label, _)| !closed_labels.iter().any(|text| text == label))
            );
            let tile = text(&closed, "Assistance");
            closed.drop_without_applying_deltas();
            assert_eq!(press(tile), None);

            let (open, _) = draw(vec![]);
            let tree = open.platform_output.accesskit_update.as_ref().unwrap();
            assert!(tree.nodes.iter().any(|(id, node)| {
                *id == tree.focus && node.label() == Some("System 32 shelf pins")
            }));
            let option = text(&open, label);
            open.drop_without_applying_deltas();
            let tool = registry.tool(id).unwrap();
            assert_eq!(press(option), Some(tool.action));
            assert!(registry.search(label).iter().any(|result| result.id == id));

            let (closed, _) = draw(vec![]);
            let tile = text(&closed, "Assistance");
            closed.drop_without_applying_deltas();
            assert_eq!(press(tile), None);
            draw(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }])
            .0
            .drop_without_applying_deltas();
            let (closed, command) = draw(vec![]);
            assert!(command.is_none());
            assert!(!labels(&closed).contains(&label.to_owned()));
            closed.drop_without_applying_deltas();
        }
    }
    #[test]
    fn sketch_creation_is_global_instead_of_duplicated_in_the_model_ribbon() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Model).tabs[0];
        assert!(!tab.tools.contains(&"new-sketch"));
        assert!(!tab.tools.contains(&"engraving"));
        assert_eq!(
            registry.resolve("sketch").unwrap().action,
            Command::NewSketch
        );
        assert!(registry.tool("engraving").is_none());
    }
    #[test]
    fn active_sketch_pins_finish_and_abort_at_the_right_and_hides_new_sketch() {
        let registry = Registry::default();
        for tab in &registry.workspace(Mode::Sketch).tabs {
            for width in [1200., 640.] {
                let ctx = egui::Context::default();
                ribbon(&registry, Mode::Sketch, tab, true, width, vec![], &ctx)
                    .0
                    .drop_without_applying_deltas();
                let (output, _) = ribbon(&registry, Mode::Sketch, tab, true, width, vec![], &ctx);
                let labels = positioned_labels(&output);
                output.drop_without_applying_deltas();
                assert!(
                    !labels.iter().any(|(l, _)| l == "New sketch"),
                    "{}",
                    tab.title
                );
                let finish = labels.iter().find(|(l, _)| l == "Finish sketch").unwrap().1;
                let abort = labels.iter().find(|(l, _)| l == "Abort").unwrap().1;
                assert!(finish.x < abort.x);
                assert!(
                    abort.x < width && abort.x > width - icons::TOOL_TILE.x,
                    "{}",
                    tab.title
                );
                assert!((finish.y - abort.y).abs() < 1.);
                let others = labels
                    .iter()
                    .filter(|(l, p)| {
                        !matches!(l.as_str(), "Finish sketch" | "Abort" | "Sketch")
                            && (p.y - finish.y).abs() < 1.
                    })
                    .map(|(_, p)| p.x)
                    .fold(f32::MIN, f32::max);
                assert!(others < finish.x, "{} at {width}", tab.title);
            }
            assert_eq!(
                click(&registry, Mode::Sketch, tab, true, 640., "Abort"),
                Some(Command::AbortSketch)
            );
            assert_eq!(
                click(&registry, Mode::Sketch, tab, true, 640., "Finish sketch"),
                Some(Command::FinishSketch)
            );
        }
        // Before a plane is chosen no sketch-editing or duplicate creation controls appear.
        let common = &registry.workspace(Mode::Sketch).tabs[0];
        let ctx = egui::Context::default();
        let (output, _) = ribbon(&registry, Mode::Sketch, common, false, 1200., vec![], &ctx);
        let labels = positioned_labels(&output);
        output.drop_without_applying_deltas();
        assert!(!labels.iter().any(|(l, _)| l == "Sketch"));
        assert!(
            !labels
                .iter()
                .any(|(l, _)| l == "Abort" || l == "Finish sketch")
        );
    }
}
