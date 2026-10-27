//! Parameter draft editor. Widgets never mutate the committed document directly.
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Vec2};
use multicad_core::{
    commands::{Actor, Channel, CommandError, ErrorCode, Prepared, Request, Session},
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
};
use std::collections::BTreeSet;
const ORDER_TYPE: &str = "core.parameter_table";
#[derive(Clone, Debug)]
pub struct Row {
    pub id: EntityId,
    pub name: String,
    pub description: String,
    pub expression: String,
    pub unit: String,
    pub exposed: bool,
}
impl Default for Row {
    fn default() -> Self {
        Self {
            id: EntityId::new(),
            name: String::new(),
            description: String::new(),
            expression: String::new(),
            unit: "mm".into(),
            exposed: false,
        }
    }
}
impl Row {
    fn complete(&self) -> bool {
        !self.name.trim().is_empty() && !self.expression.trim().is_empty()
    }
    fn empty(&self) -> bool {
        self.name.is_empty()
            && self.description.is_empty()
            && self.expression.is_empty()
            && !self.exposed
    }
    fn entity(&self, document: &Document) -> Entity {
        let inputs = document
            .get(self.id)
            .map(|e| e.references())
            .unwrap_or_default();
        Entity {
            id: self.id,
            name: self.name.clone(),
            description: self.description.clone(),
            data: EntityData::Parameter {
                expression: self.expression.clone(),
                unit: self.unit.clone(),
                exposed: self.exposed,
                inputs,
            },
        }
    }
}
pub struct ParameterEditor {
    pub rows: Vec<Row>,
    baseline: Vec<Row>,
    pub search: String,
    pub error: Option<CommandError>,
    pub preview: Option<Prepared>,
    revision: u64,
}
impl ParameterEditor {
    pub fn new(session: &Session) -> Self {
        let mut value = Self {
            rows: vec![],
            baseline: vec![],
            search: String::new(),
            error: None,
            preview: None,
            revision: session.revision(),
        };
        value.reload(session);
        value
    }
    pub fn reload(&mut self, session: &Session) {
        let mut ordered = Vec::new();
        if let Some(e)=session.document().entities().find(|e|matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==ORDER_TYPE)) && let EntityData::Extension{definition,..}=&e.data {ordered=serde_json::from_value::<Vec<EntityId>>(definition.data.clone()).unwrap_or_default();}
        let mut entities: Vec<_> = session
            .document()
            .entities()
            .filter(|e| matches!(e.data, EntityData::Parameter { .. }))
            .collect();
        entities.sort_by_key(|e| {
            ordered
                .iter()
                .position(|id| *id == e.id)
                .unwrap_or(usize::MAX)
        });
        self.rows = entities
            .into_iter()
            .map(|e| {
                let EntityData::Parameter {
                    expression,
                    unit,
                    exposed,
                    ..
                } = &e.data
                else {
                    unreachable!()
                };
                Row {
                    id: e.id,
                    name: e.name.clone(),
                    description: e.description.clone(),
                    expression: multicad_core::parameters::display_input(expression),
                    unit: unit.clone(),
                    exposed: *exposed,
                }
            })
            .collect();
        self.rows.push(Row::default());
        self.baseline = self.rows.clone();
        self.error = None;
        self.preview = None;
        self.revision = session.revision();
    }
    pub fn dirty(&self) -> bool {
        let mut rows = self.rows.iter().filter(|r| !r.empty());
        let mut before = self.baseline.iter().filter(|r| !r.empty());
        loop {
            match (rows.next(), before.next()) {
                (None, None) => return false,
                (Some(a), Some(b))
                    if a.id == b.id
                        && a.name == b.name
                        && a.description == b.description
                        && a.expression == b.expression
                        && a.unit == b.unit
                        && a.exposed == b.exposed => {}
                _ => return true,
            }
        }
    }
    pub fn ensure_blank_row(&mut self) {
        if self.rows.last().is_none_or(Row::complete) {
            self.rows.push(Row::default());
        }
    }
    pub fn prepare(&self, session: &Session, channel: Channel) -> Result<Prepared, CommandError> {
        for row in self.rows.iter().filter(|r| !r.empty()) {
            if !row.complete() || row.unit.trim().is_empty() {
                return Err(CommandError {
                    code: ErrorCode::InvalidParameter,
                    message: "Complete the name, expression and unit before applying".into(),
                    involved: vec![row.id],
                    parameter: None,
                });
            }
        }
        let mut edits = Vec::new();
        let mut present = BTreeSet::new();
        for row in self.rows.iter().filter(|r| !r.empty()) {
            multicad_core::parameters::validate_input(&row.expression).map_err(|mut e| {
                e.entity = Some(row.id);
                CommandError::from(e)
            })?;
            let entity = row.entity(session.document());
            present.insert(row.id);
            edits.push(if session.document().get(row.id).is_some() {
                Mutation::Replace(entity)
            } else {
                Mutation::Insert(entity)
            });
        }
        for entity in session
            .document()
            .entities()
            .filter(|e| matches!(e.data, EntityData::Parameter { .. }))
        {
            if !present.contains(&entity.id) {
                edits.push(Mutation::Remove(entity.id));
            }
        }
        let order: Vec<_> = self
            .rows
            .iter()
            .filter(|r| !r.empty())
            .map(|r| r.id)
            .collect();
        let previous=session.document().entities().find(|e|matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==ORDER_TYPE));
        if !order.is_empty() || previous.is_some() {
            let mut entity = previous.cloned().unwrap_or_else(|| {
                Entity::new(
                    "Parameter table",
                    EntityData::Extension {
                        definition: Definition {
                            type_id: ORDER_TYPE.into(),
                            schema_version: 1,
                            data: serde_json::Value::Null,
                        },
                        inputs: vec![],
                    },
                )
            });
            entity.data = EntityData::Extension {
                definition: Definition {
                    type_id: ORDER_TYPE.into(),
                    schema_version: 1,
                    data: serde_json::json!(order),
                },
                inputs: order.iter().map(|id| Link::new(*id, "row")).collect(),
            };
            edits.push(if previous.is_some() {
                Mutation::Replace(entity)
            } else {
                Mutation::Insert(entity)
            });
        }
        session.prepare(Request {
            expected_revision: self.revision,
            actor: Actor::local(channel),
            label: "Edit parameters".into(),
            edits,
        })
    }
    pub fn error_text(&self, error: &CommandError) -> String {
        let name = self
            .rows
            .iter()
            .find(|r| error.involved.contains(&r.id))
            .map(|r| {
                if r.name.is_empty() {
                    "New row"
                } else {
                    &r.name
                }
            });
        let message = error
            .parameter
            .as_ref()
            .map(|p| p.message.as_str())
            .unwrap_or(&error.message);
        name.map(|name| format!("{name}: {message}"))
            .unwrap_or_else(|| message.into())
    }
    pub fn refresh_preview(&mut self, session: &Session) {
        match self.prepare(session, Channel::Ui) {
            Ok(value) => {
                self.preview = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
                self.preview = None;
            }
        }
    }
    pub fn apply(&mut self, session: &mut Session, channel: Channel) -> Result<(), CommandError> {
        let prepared = self.prepare(session, channel)?;
        session.commit(prepared)?;
        self.reload(session);
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, session: &Session) {
        if self.revision != session.revision() && !self.dirty() {
            self.reload(session);
        }
        ui.horizontal(|ui| {
            ui.heading("Parameters");
            ui.add_space(20.0);
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Filter parameters…")
                    .desired_width(240.),
            )
            .on_hover_text("Filter rows by parameter name or description");
        });
        ui.label("Enter a value or a formula such as =600 - $height. Comparisons return 1 or 0 (ratio). Complete a row to add another.");
        let width = ui.available_width();
        let columns = [0.14, 0.22, 0.23, 0.07, 0.13, 0.07, 0.14].map(|f| f * width);
        let headers = [
            "Name",
            "Description",
            "Value / expression",
            "Unit",
            "Evaluated value",
            "Expose",
            "Row",
        ];
        let (head, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(head, 0, Color32::from_rgb(40, 50, 66));
        let mut x = head.left();
        for (title, w) in headers.iter().zip(columns) {
            ui.painter().text(
                Pos2::new(x + 6.0, head.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                FontId::proportional(12.0),
                Color32::WHITE,
            );
            x += w;
        }
        let mut changed = false;
        let mut delete = None;
        let mut movement = None;
        let query = self.search.to_lowercase();
        egui::ScrollArea::vertical().show_rows(ui, 36.0, self.rows.len(), |ui, range| {
            for index in range {
                let row = &mut self.rows[index];
                if !query.is_empty()
                    && !row.empty()
                    && !format!("{} {}", row.name, row.description)
                        .to_lowercase()
                        .contains(&query)
                {
                    continue;
                }
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(width, 36.0), egui::Sense::hover());
                let color = if self.error.as_ref().is_some_and(|e| {
                    e.parameter
                        .as_ref()
                        .is_some_and(|p| p.entity == Some(row.id))
                }) {
                    Color32::from_rgb(71, 33, 39)
                } else if index % 2 == 0 {
                    Color32::from_rgb(29, 35, 45)
                } else {
                    Color32::from_rgb(33, 40, 51)
                };
                ui.painter().rect_filled(rect, 0, color);
                let mut x = rect.left();
                for (column, w) in columns.into_iter().enumerate() {
                    let cell = Rect::from_min_max(
                        Pos2::new(x + 4.0, rect.top() + 3.0),
                        Pos2::new(x + w - 4.0, rect.bottom() - 3.0),
                    );
                    x += w;
                    ui.painter().line_segment(
                        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                        egui::Stroke::new(0.5, Color32::from_gray(60)),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt((row.id, column))
                            .max_rect(cell)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| match column {
                            0 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.name)
                                            .hint_text("name")
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column))
                                    .changed()
                            }
                            1 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.description)
                                            .hint_text("Description")
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column))
                                    .changed()
                            }
                            2 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.expression)
                                            .hint_text("Value or expression")
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column))
                                    .changed()
                            }
                            3 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.unit)
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column))
                                    .changed()
                            }
                            4 => {
                                let values = self
                                    .preview
                                    .as_ref()
                                    .map(|p| p.parameters())
                                    .unwrap_or_else(|| session.parameters());
                                if let Some(value) = values.values.get(&row.id) {
                                    ui.label(format!(
                                        "{:.6} {}",
                                        value.quantity.in_unit(value.unit).unwrap_or(0.0),
                                        value.unit.symbol()
                                    ))
                                    .on_hover_text(format!("{:?}", value.role));
                                } else {
                                    ui.label("-");
                                }
                            }
                            5 => changed |= ui.checkbox(&mut row.exposed, "").on_hover_text("Expose this default to symbols and external clients. A supplied value replaces its formula.").changed(),
                            6 if !row.empty() => {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing=Vec2::new(4.,0.);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y=0.;
                                        if ui.add_enabled_ui(index>0,|ui|crate::icons::mini_button(ui,crate::icons::Icon::ArrowUp,"Move parameter up")).inner.clicked(){movement=Some((index,index-1));}
                                        if crate::icons::mini_button(ui,crate::icons::Icon::ArrowDown,"Move parameter down").clicked(){movement=Some((index,index+1));}
                                    });
                                    if crate::icons::compact_button(ui,crate::icons::Icon::Trash,"Delete parameter").clicked(){delete=Some(index);}
                                });
                            }
                            _ => {}
                        },
                    );
                }
            }
        });
        if let Some(index) = delete {
            self.rows.remove(index);
            changed = true;
        }
        if let Some((a, b)) = movement
            && b < self.rows.len()
            && !self.rows[b].empty()
        {
            self.rows.swap(a, b);
            changed = true;
        }
        self.ensure_blank_row();
        if changed {
            self.refresh_preview(session);
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, self.error_text(error));
        }
    }
}

fn cell_help(column: usize) -> &'static str {
    match column {
        0 => "Stable parameter name used by $references",
        1 => "Describe the parameter",
        2 => "Value, or =formula with $parameter references",
        _ => "Unit: mm, cm, m, in, ft, deg, rad, count, ratio or %",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completing_a_row_adds_one_blank_and_commits_exposure_with_undo() {
        let mut session = Session::new(Document::new("Test").unwrap()).unwrap();
        let mut editor = ParameterEditor::new(&session);
        editor.rows[0].name = "width".into();
        editor.rows[0].expression = "600".into();
        editor.rows[0].exposed = true;
        let id = editor.rows[0].id;
        editor.ensure_blank_row();
        editor.ensure_blank_row();
        assert_eq!(editor.rows.len(), 2);
        assert!(session.document().get(id).is_none());
        editor.apply(&mut session, Channel::Ui).unwrap();
        assert!(session.parameters().values[&id].exposed);
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert!(session.document().get(id).is_none());
    }
    #[test]
    fn invalid_drafts_and_stale_editors_do_not_replace_the_model() {
        let mut session = Session::new(Document::new("Test").unwrap()).unwrap();
        let mut editor = ParameterEditor::new(&session);
        editor.rows[0].name = "width".into();
        editor.rows[0].expression = "1 mm + 2 deg".into();
        assert!(editor.apply(&mut session, Channel::Ui).is_err());
        assert_eq!(session.revision(), 0);
        assert_eq!(editor.rows[0].expression, "1 mm + 2 deg");
        editor.rows[0].expression = "600".into();
        session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Mcp),
                label: "Remote rename".into(),
                edits: vec![Mutation::RenameDocument("New".into())],
            })
            .unwrap();
        assert!(editor.apply(&mut session, Channel::Ui).is_err());
    }
}
