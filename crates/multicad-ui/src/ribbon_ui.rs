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
pub fn groups(mode: Mode, tab: &RibbonTab) -> Vec<Group> {
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
                Entry::Tool("boolean"),
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
    !matches!(command,Command::Extension(id) if !matches!(id,"materials"|"appearance"|"render-view"))
}
/// Session actions pinned at the right end of the ribbon while a sketch is
/// open, on every Sketch tab and outside the horizontal scroll.
pub const SKETCH_SESSION: [&str; 2] = ["finish-sketch", "abort-sketch"];
const BUTTON: f32 = 76.;
const SEPARATOR: f32 = 19.;
fn separator(ui: &mut egui::Ui) {
    ui.add_space(9.);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1., 90.), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1., egui::Color32::from_gray(65)),
    );
    ui.add_space(9.);
}
fn tool_button(
    ui: &mut egui::Ui,
    registry: &Registry,
    id: &str,
    enabled: &impl Fn(Command) -> bool,
    hint: &impl Fn(Command, &str) -> String,
) -> Option<Command> {
    let tool = registry.tool(id).expect("catalog entry");
    let response = ui
        .add_enabled_ui(enabled(tool.action), |ui| {
            icons::tool_with_hint(ui, tool.icon, tool.label, &hint(tool.action, tool.label))
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
fn section_label(ui: &mut egui::Ui, label: &str) {
    if !label.is_empty() {
        ui.add_space(2.);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.small(egui::RichText::new(label).color(egui::Color32::from_gray(160)));
        });
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let width = (ui.available_width() - pinned_width).max(0.);
        ui.allocate_ui(egui::vec2(width, 108.), |ui| {
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
                    ui.allocate_exact_size(egui::vec2(width, 108.), egui::Sense::hover());
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
            egui::ScrollArea::horizontal()
                .id_salt(("workspace-ribbon", mode.title(), tab.title))
                .max_height(108.)
                .min_scrolled_height(108.)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, section) in sections.iter().enumerate() {
                            if index > 0 {
                                separator(ui);
                            }
                            ui.vertical(|ui| {
                                ui.set_width(section.entries.len() as f32 * BUTTON);
                                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.;
                                    for entry in &section.entries {
                                        match entry {
                                            Entry::Tool(id) => {
                                                if let Some(command) =
                                                    tool_button(ui, registry, id, &enabled, &hint)
                                                {
                                                    action = Some(command);
                                                }
                                            }
                                            Entry::Menu { label, icon, tools } => {
                                                let response = icons::tool_with_hint(
                                                    ui,
                                                    *icon,
                                                    label,
                                                    &format!("{label} · choose a tool"),
                                                );
                                                let rect = response.rect;
                                                let center =
                                                    rect.right_top() + egui::vec2(-9., 12.);
                                                ui.painter().add(egui::Shape::convex_polygon(
                                                    vec![
                                                        center + egui::vec2(-3., -2.),
                                                        center + egui::vec2(3., -2.),
                                                        center + egui::vec2(0., 2.),
                                                    ],
                                                    egui::Color32::LIGHT_GRAY,
                                                    egui::Stroke::NONE,
                                                ));
                                                egui::Popup::menu(&response).show(|ui| {
                                                    for id in tools {
                                                        let tool = registry
                                                            .tool(id)
                                                            .expect("menu catalog entry");
                                                        if ui
                                                            .add_enabled(
                                                                enabled(tool.action),
                                                                egui::Button::new(tool.label),
                                                            )
                                                            .on_hover_text(hint(
                                                                tool.action,
                                                                tool.label,
                                                            ))
                                                            .clicked()
                                                        {
                                                            action = Some(tool.action);
                                                            ui.close();
                                                        }
                                                    }
                                                });
                                            }
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
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    for id in &pinned {
                        if let Some(command) = tool_button(ui, registry, id, &enabled, &hint) {
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
                .any(|e| matches!(e, Entry::Tool("boolean")))
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
    fn labels(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().replace('\n', " "),
                    text.pos + text.galley.size() * 0.5,
                )),
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
                let labels = labels(&output);
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
        let labels = labels(&output);
        output.drop_without_applying_deltas();
        assert!(labels.iter().any(|(l, _)| l == "New sketch"));
        assert!(
            !labels
                .iter()
                .any(|(l, _)| l == "Abort" || l == "Finish sketch")
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
}
