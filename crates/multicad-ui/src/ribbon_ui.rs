//! Declarative sections and menus over the shared tool catalog.
use crate::{
    icons::{self, Icon},
    registry::{Command, Mode, Registry, RibbonTab},
};
use eframe::egui;
#[derive(Clone)]
pub enum Entry {
    Tool(&'static str),
    Menu {
        label: &'static str,
        icon: Icon,
        tools: Vec<&'static str>,
    },
}
pub struct Group {
    pub label: &'static str,
    pub entries: Vec<Entry>,
}
/// Sketch → Common keeps its order but folds each drawing-mode family into
/// one menu at the position of its first member.
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
                Entry::Tool("new-sketch"),
                Entry::Tool("engraving"),
                Entry::Menu {
                    label: "Add Primitive",
                    icon: Icon::Box,
                    tools: vec!["box", "cylinder"],
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
                        .to_vec(),
                },
                Entry::Tool("split"),
                Entry::Tool("edge-treatment"),
                Entry::Tool("shell"),
                Entry::Tool("draft"),
            ],
        },
        Group {
            label: "Transform",
            entries: vec![
                Entry::Tool("move"),
                Entry::Tool("align"),
                Entry::Tool("scale"),
            ],
        },
        Group {
            label: "Repeat",
            entries: vec![Entry::Menu {
                label: "Pattern",
                icon: Icon::CircularPattern,
                tools: vec![
                    "model-mirror",
                    "model-linear-pattern",
                    "model-circular-pattern",
                ],
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
    !matches!(command,Command::Extension(id) if !matches!(id,"materials"|"appearance"|"render-view"|"import-step"|"import-kicad"|"import-update"))
}
/// Session actions pinned at the right end of the ribbon while a sketch is
/// open, on every Sketch tab and outside the horizontal scroll.
pub const SKETCH_SESSION: [&str; 2] = ["finish-sketch", "abort-sketch"];
const BUTTON: f32 = icons::TOOL_TILE.x;
const SEPARATOR: f32 = 19.;
/// Fixed row under the tiles that carries the group caption.
const SECTION_LABEL: f32 = 16.;
/// Space kept below the tiles for the horizontal overflow scroll bar, so the
/// bar never covers a label and the height does not change on resize.
const SCROLL_BAR: f32 = 10.;
/// Height of the tool strip: tiles with their two-line labels, the group
/// captions and the overflow scroll bar. Every row is top-aligned so the
/// labels, hit targets and pinned session actions share one baseline.
pub const TOOLS_HEIGHT: f32 = icons::TOOL_TILE.y + SECTION_LABEL + SCROLL_BAR;
fn top_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), add)
}
fn separator(ui: &mut egui::Ui) {
    ui.add_space(9.);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(1., icons::TOOL_TILE.y + SECTION_LABEL),
        egui::Sense::hover(),
    );
    ui.painter().vline(
        rect.center().x,
        rect.shrink2(egui::vec2(0., 4.)).y_range(),
        egui::Stroke::new(1., egui::Color32::from_gray(65)),
    );
    ui.add_space(9.);
}
fn section_label(ui: &mut egui::Ui, label: &str) {
    let width = ui.available_width().max(1.);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, SECTION_LABEL), egui::Sense::hover());
    if !label.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::TextStyle::Small.resolve(ui.style()),
            egui::Color32::from_gray(160),
        );
    }
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
fn tool_button(
    ui: &mut egui::Ui,
    registry: &Registry,
    mode: Mode,
    id: &str,
    enabled: &impl Fn(Command) -> bool,
    hint: &impl Fn(Command, &str) -> String,
    selected: &impl Fn(Command) -> bool,
) -> Option<Command> {
    let tool = registry.tool(id).expect("catalog entry");
    let response = ui
        .add_enabled_ui(enabled(tool.action), |ui| {
            icons::tool_glyph(
                ui,
                tool_glyph(mode, id, tool.icon),
                tool.label,
                &with_description(id, hint(tool.action, tool.label)),
                selected(tool.action),
            )
        })
        .inner;
    response
        .on_disabled_hover_text(match tool.action {
            Command::Modeling(tool) => tool.requirements(),
            _ => "Unavailable in the current context",
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
    enabled: &impl Fn(Command) -> bool,
    hint: &impl Fn(Command, &str) -> String,
    selected: &impl Fn(Command) -> bool,
) -> Option<Command> {
    let is_selected = |id: &&str| registry.tool(id).is_some_and(|tool| selected(tool.action));
    // The active mode, else the last choice.
    let choice = tools
        .iter()
        .copied()
        .find(is_selected)
        .unwrap_or_else(|| menu_choice(ui.ctx(), mode, label, tools));
    let glyph = registry
        .tool(choice)
        .map(|tool| tool_glyph(mode, choice, tool.icon))
        .filter(|glyph| matches!(glyph, icons::Glyph::Art(..)))
        .unwrap_or(icons::Glyph::Vector(icon));
    let response = icons::tool_glyph(
        ui,
        glyph,
        label,
        &format!("{label} · choose a tool"),
        tools.iter().any(is_selected),
    );
    let center = response.rect.right_top() + egui::vec2(-9., 12.);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(-3., -2.),
            center + egui::vec2(3., -2.),
            center + egui::vec2(0., 2.),
        ],
        egui::Color32::LIGHT_GRAY,
        egui::Stroke::NONE,
    ));
    let mut action = None;
    egui::Popup::menu(&response).show(|ui| {
        for id in tools {
            let tool = registry.tool(id).expect("menu catalog entry");
            if ui
                .add_enabled(enabled(tool.action), egui::Button::new(tool.label))
                .on_hover_text(with_description(id, hint(tool.action, tool.label)))
                .clicked()
            {
                action = Some(tool.action);
                let id: &'static str = id;
                ui.ctx()
                    .data_mut(|data| data.insert_temp(menu_memory(mode, label), id));
                ui.close();
            }
        }
    });
    action
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
    let pinned: Vec<&str> = if mode == Mode::Sketch && editing {
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
                && if *id == "new-sketch" {
                    !editing
                } else {
                    mode != Mode::Sketch || editing
                }
        })
    };
    let mut sections = groups(mode, tab);
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
    let mut action = None;
    let pinned_width = if pinned.is_empty() {
        0.
    } else {
        pinned.len() as f32 * BUTTON + SEPARATOR
    };
    top_row(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let width = (ui.available_width() - pinned_width).max(0.);
        ui.allocate_ui(egui::vec2(width, TOOLS_HEIGHT), |ui| {
            ui.set_width(width);
            if sections.is_empty() {
                let message = if mode == Mode::Sketch && !editing && tab.requires_active_sketch {
                    "Create or open a sketch to see its tools."
                } else if mode == Mode::Sketch && editing {
                    "No further tools on this tab."
                } else {
                    "No tools available in this workspace yet."
                };
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, TOOLS_HEIGHT), egui::Sense::hover());
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
                return;
            }
            // A solid bar reserves its own strip below the captions instead
            // of floating over the labels when the ribbon overflows.
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            ui.spacing_mut().scroll.bar_inner_margin = SCROLL_BAR - ui.spacing().scroll.bar_width;
            egui::ScrollArea::horizontal()
                .id_salt(("workspace-ribbon", mode.title(), tab.title))
                .max_height(TOOLS_HEIGHT)
                .min_scrolled_height(TOOLS_HEIGHT)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    top_row(ui, |ui| {
                        for (index, section) in sections.iter().enumerate() {
                            if index > 0 {
                                separator(ui);
                            }
                            ui.vertical(|ui| {
                                ui.set_width(section.entries.len() as f32 * BUTTON);
                                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                                top_row(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 0.;
                                    for entry in &section.entries {
                                        let chosen = match entry {
                                            Entry::Tool(id) => tool_button(
                                                ui, registry, mode, id, &enabled, &hint, &selected,
                                            ),
                                            Entry::Menu { label, icon, tools } => menu_button(
                                                ui, registry, mode, label, *icon, tools, &enabled,
                                                &hint, &selected,
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
                });
        });
        if !pinned.is_empty() {
            separator(ui);
            ui.vertical(|ui| {
                ui.set_width(pinned.len() as f32 * BUTTON);
                top_row(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    for id in &pinned {
                        if let Some(command) =
                            tool_button(ui, registry, mode, id, &enabled, &hint, &selected)
                        {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        run: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 800.),
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
                    if tools == &["boolean-join", "boolean-subtract", "boolean-intersect"]))
        );
        assert!(
            modify
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Tool("split")))
        );
        let ctx = egui::Context::default();
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
        let menu = text(&output, "Add Primitive");
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
        assert!((center.y - 62.).abs() < 16.);
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
        let tab = &registry.workspace(Mode::Parameters).tabs[0];
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
                    (Mode::Model, "Wood") => &["Panel / carcass", "Drill / pocket", "Panel joint"],
                    (Mode::Model, "Sheet metal") => &["Sheet part / flanges"],
                    (Mode::Export, "Print") => &["Preview package", "Write 3MF package"],
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
                for placeholder in ["Cut list", "Flat pattern", "Bend schedule"] {
                    assert!(!shown.iter().any(|shown| shown == placeholder));
                }
            }
        }
    }
    /// CAD-154: every catalog label fits a tile in two lines without
    /// elision, so labels and their padding stay inside `TOOLS_HEIGHT`.
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
                        let galley = ui.painter().layout(
                            label.into(),
                            egui::FontId::proportional(10.),
                            egui::Color32::WHITE,
                            icons::TOOL_TILE.x - 6.,
                        );
                        assert!(galley.rows.len() <= 2, "{label} needs elision");
                        let shown = icons::tile_label(ui, label, egui::Color32::WHITE);
                        assert!(shown.rows.len() <= 2, "{label}");
                        assert!(
                            icons::TOOL_LABEL_TOP + shown.size().y + 4. <= icons::TOOL_TILE.y,
                            "{label} leaves no bottom padding"
                        );
                    }
                }
            }
        })
        .drop_without_applying_deltas();
        const { assert!(icons::TOOL_TILE.y + SECTION_LABEL <= TOOLS_HEIGHT) };
    }
    #[test]
    fn sketch_drawing_modes_fold_into_rectangle_and_circle_menus_in_place() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Sketch).tabs[0];
        let entries = &groups(Mode::Sketch, tab)[0].entries;
        let menu = |name: &str| {
            entries.iter().find_map(|entry| match entry {
                Entry::Menu { label, tools, .. } if *label == name => Some(tools.clone()),
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
    fn model_common_starts_a_new_sketch_from_the_create_group() {
        let registry = Registry::default();
        let tab = &registry.workspace(Mode::Model).tabs[0];
        let create = groups(Mode::Model, tab);
        assert!(matches!(
            create[0].entries.first(),
            Some(Entry::Tool("new-sketch"))
        ));
        assert!(matches!(
            create[0].entries.get(1),
            Some(Entry::Tool("engraving"))
        ));
        assert_eq!(
            click(&registry, Mode::Model, tab, false, 1200., "New sketch"),
            Some(Command::NewSketch)
        );
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
                assert!(abort.x < width && abort.x > width - BUTTON, "{}", tab.title);
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
        // Before a plane is chosen only New sketch is offered.
        let common = &registry.workspace(Mode::Sketch).tabs[0];
        let ctx = egui::Context::default();
        let (output, _) = ribbon(&registry, Mode::Sketch, common, false, 1200., vec![], &ctx);
        let labels = positioned_labels(&output);
        output.drop_without_applying_deltas();
        assert!(labels.iter().any(|(l, _)| l == "New sketch"));
        assert!(
            !labels
                .iter()
                .any(|(l, _)| l == "Abort" || l == "Finish sketch")
        );
    }
}
