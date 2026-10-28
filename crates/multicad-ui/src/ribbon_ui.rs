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
                Entry::Menu {
                    label: "Add Primitive",
                    icon: Icon::Box,
                    tools: vec!["box", "cylinder"],
                },
                Entry::Tool("extrude"),
                Entry::Tool("revolve"),
            ],
        },
        Group {
            label: "Modify",
            entries: vec![
                Entry::Tool("boolean"),
                Entry::Tool("split"),
                Entry::Tool("edge"),
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
    !matches!(command,Command::Extension(id) if id!="materials")
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
    let visible = |id: &&str| {
        registry.tool(id).is_some_and(|tool| {
            implemented(tool.action) && (mode != Mode::Sketch || editing || *id == "new-sketch")
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
    if sections.is_empty() {
        let message = if mode == Mode::Sketch && !editing && tab.requires_active_sketch {
            "Create or open a sketch to see its tools."
        } else {
            "No tools available in this workspace yet."
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 108.), egui::Sense::hover());
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
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt(("workspace-ribbon", mode.title(), tab.title))
        .max_height(108.)
        .min_scrolled_height(108.)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (index, section) in sections.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(9.);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(1., 90.), egui::Sense::hover());
                        ui.painter().vline(
                            rect.center().x,
                            rect.y_range(),
                            egui::Stroke::new(1., egui::Color32::from_gray(65)),
                        );
                        ui.add_space(9.);
                    }
                    ui.vertical(|ui| {
                        ui.set_width(section.entries.len() as f32 * 76.);
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.;
                            for entry in &section.entries {
                                match entry {
                                    Entry::Tool(id) => {
                                        let tool = registry.tool(id).expect("catalog entry");
                                        let response = ui
                                            .add_enabled_ui(enabled(tool.action), |ui| {
                                                icons::tool_with_hint(
                                                    ui,
                                                    tool.icon,
                                                    tool.label,
                                                    &hint(tool.action, tool.label),
                                                )
                                            })
                                            .inner;
                                        if response
                                            .on_disabled_hover_text(match tool.action {
                                                Command::Modeling(tool) => tool.requirements(),
                                                _ => "Unavailable in the current context",
                                            })
                                            .clicked()
                                        {
                                            action = Some(tool.action);
                                        }
                                    }
                                    Entry::Menu { label, icon, tools } => {
                                        let response = icons::tool_with_hint(
                                            ui,
                                            *icon,
                                            label,
                                            "Add a Box or Cylinder primitive",
                                        );
                                        let rect = response.rect;
                                        let center = rect.right_top() + egui::vec2(-9., 12.);
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
                                                let tool =
                                                    registry.tool(id).expect("menu catalog entry");
                                                if ui
                                                    .add_enabled(
                                                        enabled(tool.action),
                                                        egui::Button::new(tool.label),
                                                    )
                                                    .on_hover_text(hint(tool.action, tool.label))
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
                        if !section.label.is_empty() {
                            ui.add_space(2.);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                ui.small(
                                    egui::RichText::new(section.label)
                                        .color(egui::Color32::from_gray(160)),
                                );
                            });
                        }
                    });
                }
            });
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
