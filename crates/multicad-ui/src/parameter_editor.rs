//! Parameter draft editor. Widgets never mutate the committed document directly.
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Vec2};
use multicad_core::{
    commands::{Actor, Channel, CommandError, ErrorCode, Prepared, Request, Session, SessionStamp},
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
    project_file::info::{self, FileInfo},
};
use std::collections::BTreeSet;
const ORDER_TYPE: &str = "core.parameter_table";
const EXPOSE_HELP: &str = "Make this parameter available to symbols and external clients. A supplied value overrides the formula.";

#[derive(Clone, Copy)]
struct ParameterDrag {
    table: egui::Id,
    row: EntityId,
}
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
            // New rows default to the Settings length unit (CAD-187); the
            // unit is stored explicitly with the parameter.
            unit: crate::preferences::length_unit().symbol().into(),
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
            && crate::preferences::LengthUnit::from_symbol(&self.unit).is_some()
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
/// Role, plus the Settings-unit equivalent of a length in another unit.
fn value_help(value: &multicad_core::parameters::ParameterValue) -> String {
    let preferred = crate::preferences::length_unit();
    match value
        .quantity
        .in_unit(multicad_core::parameters::Unit::Millimeter)
    {
        Some(mm) if value.unit != preferred.unit() => {
            format!("{:?} · {}", value.role, preferred.format_mm(mm))
        }
        _ => format!("{:?}", value.role),
    }
}
impl ParameterEditor {
    /// Blank, unsaved rows adopt a changed default length unit; authored
    /// parameters keep their declared units.
    pub fn adopt_length_unit(&mut self, unit: crate::preferences::LengthUnit) {
        for row in self.rows.iter_mut().chain(self.baseline.iter_mut()) {
            if row.empty() {
                row.unit = unit.symbol().into();
            }
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
    guard: SessionStamp,
    /// File name, description and preview draft, applied with the table.
    pub info: FileInfo,
    info_baseline: FileInfo,
    /// Set when the file information was written by a newer schema.
    pub info_locked: Option<String>,
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
            guard: session.stamp(),
            info: FileInfo::default(),
            info_baseline: FileInfo::default(),
            info_locked: None,
        };
        value.reload(session);
        value
    }
    pub fn reload(&mut self, session: &Session) {
        let symbol_owned =
            multicad_core::symbols::owned_entities_in_session(session).unwrap_or_default();
        let mut ordered = Vec::new();
        if let Some(e)=session.document().entities().find(|e|matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==ORDER_TYPE)) && let EntityData::Extension{definition,..}=&e.data {ordered=serde_json::from_value::<Vec<EntityId>>(definition.data.clone()).unwrap_or_default();}
        let mut entities: Vec<_> = session
            .document()
            .entities()
            .filter(|e| {
                matches!(e.data, EntityData::Parameter { .. }) && !symbol_owned.contains(&e.id)
            })
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
        match info::read(session.document()) {
            Ok(value) => {
                self.info = value;
                self.info_locked = None;
            }
            Err(error) => {
                self.info = FileInfo {
                    name: session.document().name().into(),
                    ..Default::default()
                };
                self.info_locked = Some(error);
            }
        }
        self.info_baseline = self.info.clone();
        self.error = None;
        self.preview = None;
        self.revision = session.revision();
        self.guard = session.stamp();
    }
    /// Navigation needs fresh source rows, but an unchanged session already
    /// has them. Its stamp checks both session identity and revision without
    /// preparing/validating another complete document.
    pub(crate) fn sync_clean_session(&mut self, session: &Session) {
        if !self.dirty() && session.validate_stamp(&self.guard).is_err() {
            self.reload(session);
        }
    }
    pub fn dirty(&self) -> bool {
        self.info_dirty() || self.rows_dirty()
    }
    pub fn info_dirty(&self) -> bool {
        self.info != self.info_baseline
    }
    pub(crate) fn commit_file_info(&mut self, session: &mut Session) -> Result<(), String> {
        if !self.info_dirty() {
            return Ok(());
        }
        if let Some(reason) = &self.info_locked {
            return Err(reason.clone());
        }
        session
            .validate_stamp(&self.guard)
            .map_err(|error| error.to_string())?;
        let edits =
            info::edits(session.document(), &self.info).map_err(|error| error.to_string())?;
        if !edits.is_empty() {
            let edit = session
                .prepare(Request {
                    expected_revision: session.revision(),
                    actor: Actor::local(Channel::Ui),
                    label: "Edit project details".into(),
                    edits,
                })
                .map_err(|error| error.to_string())?;
            session.commit(edit).map_err(|error| error.to_string())?;
        }
        self.info_baseline = self.info.clone();
        self.revision = session.revision();
        self.guard = session.stamp();
        self.preview = None;
        self.error = None;
        Ok(())
    }
    fn rows_dirty(&self) -> bool {
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
        session.validate_stamp(&self.guard)?;
        let authored = |r: &&Row| !r.empty() || session.document().get(r.id).is_some();
        for row in self.rows.iter().filter(authored) {
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
        let symbol_owned = multicad_core::symbols::owned_entities(session.document())
            .map_err(CommandError::from)?;
        for row in self.rows.iter().filter(authored) {
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
            if !present.contains(&entity.id) && !symbol_owned.contains(&entity.id) {
                edits.push(Mutation::Remove(entity.id));
            }
        }
        let order: Vec<_> = self.rows.iter().filter(authored).map(|r| r.id).collect();
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
        let info_error = |message: String| CommandError {
            code: ErrorCode::InvalidParameter,
            message,
            involved: vec![],
            parameter: None,
        };
        let info_changed = self.info_dirty();
        if info_changed {
            if let Some(locked) = &self.info_locked {
                return Err(info_error(locked.clone()));
            }
            edits.extend(info::edits(session.document(), &self.info).map_err(info_error)?);
        }
        session.prepare(Request {
            expected_revision: self.revision,
            actor: Actor::local(channel),
            label: if info_changed {
                "Edit parameters and file information"
            } else {
                "Edit parameters"
            }
            .into(),
            edits,
        })
    }
    pub fn error_text(&self, error: &CommandError) -> String {
        let entity = error.parameter.as_ref().and_then(|p| p.entity);
        let name = self
            .rows
            .iter()
            .find(|r| entity.map_or_else(|| error.involved.contains(&r.id), |id| r.id == id))
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
    pub fn can_apply(&self, session: &Session) -> bool {
        self.dirty()
            && self.error.is_none()
            && self
                .preview
                .as_ref()
                .is_some_and(|p| session.validate_preview(p).is_ok())
    }
    pub fn apply(&mut self, session: &mut Session, channel: Channel) -> Result<(), CommandError> {
        let prepared = self.prepare(session, channel)?;
        session.commit(prepared)?;
        self.reload(session);
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, session: &Session) {
        self.show_with_min_table_width(ui, session, 0.);
    }
    pub(crate) fn sync_preview(&mut self, session: &Session) {
        if let Err(error) = session.validate_stamp(&self.guard) {
            if self.dirty() {
                self.preview = None;
                self.error = Some(error);
            } else {
                self.reload(session);
            }
        }
        if self.dirty() && self.preview.is_none() && self.error.is_none() {
            self.refresh_preview(session);
        }
    }
    pub(crate) fn show_with_min_table_width(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        minimum: f32,
    ) {
        self.sync_preview(session);
        ui.horizontal(|ui| {
            ui.heading("Parameters");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let width = ui.available_width().min(240.);
                let response = ui
                    .add_sized(
                        [width, 28.],
                        egui::TextEdit::singleline(&mut self.search)
                            .min_size(Vec2::new(width, 28.))
                            .vertical_align(egui::Align::Center)
                            .frame(crate::theme::input_frame())
                            .margin(egui::Margin {
                                left: 26,
                                right: 6,
                                top: 3,
                                bottom: 3,
                            })
                            .hint_text("Search parameters…"),
                    )
                    .on_hover_text("Search by parameter name or description");
                let center = response.rect.left_center() + Vec2::new(13., -1.);
                let stroke = egui::Stroke::new(1.4, ui.visuals().weak_text_color());
                ui.painter().circle_stroke(center, 4., stroke);
                ui.painter().line_segment(
                    [center + Vec2::splat(2.8), center + Vec2::splat(6.)],
                    stroke,
                );
            });
        });
        ui.label("Enter a value or a formula such as =600 - $height. Comparisons return 1 or 0 (ratio). Complete a row to add another.");
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, self.error_text(error));
        }
        let table_id = ui.make_persistent_id(("parameter-table", session.document().id()));
        if ui.available_width() < minimum {
            ui.scope(|ui| {
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::horizontal()
                    .id_salt("project-table-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(minimum);
                        self.show_table(ui, session, table_id);
                    });
            });
        } else {
            self.show_table(ui, session, table_id);
        }
    }
    fn show_table(&mut self, ui: &mut egui::Ui, session: &Session, table_id: egui::Id) {
        let width = ui.available_width();
        let field_width = width - 70.;
        let columns = [
            30.,
            0.15 * field_width,
            0.23 * field_width,
            0.25 * field_width,
            0.09 * field_width,
            0.20 * field_width,
            0.08 * field_width,
            40.,
        ];
        let headers = [
            "",
            "Name",
            "Description",
            "Value / expression",
            "Unit",
            "Evaluated value",
            "Expose",
            "",
        ];
        let (head, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(head, 0, Color32::from_rgb(40, 50, 66));
        let mut x = head.left();
        for (column, (title, w)) in headers.iter().zip(columns).enumerate() {
            if column == 6 {
                let cell =
                    Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(w, head.height()));
                ui.interact(cell, table_id.with("expose-help"), egui::Sense::hover())
                    .on_hover_text(EXPOSE_HELP);
            }
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
        let visible: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                query.is_empty()
                    || row.empty()
                    || session.document().get(row.id).is_none_or(|source| {
                        format!("{} {}", source.name, source.description)
                            .to_lowercase()
                            .contains(&query)
                    })
                    || format!("{} {}", row.name, row.description)
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        let values = self
            .preview
            .as_ref()
            .filter(|p| session.validate_preview(p).is_ok())
            .map(|p| p.parameters())
            .or_else(|| (!self.dirty()).then(|| session.parameters()));
        egui::ScrollArea::vertical().show_rows(ui, 36.0, visible.len(), |ui, range| {
            for visible_index in range {
                let index = visible[visible_index];
                let can_move_down = self
                    .rows
                    .get(index + 1)
                    .is_some_and(|r| !r.empty() || session.document().get(r.id).is_some());
                let row = &mut self.rows[index];
                let authored = !row.empty() || session.document().get(row.id).is_some();
                let (rect, drop_zone) =
                    ui.allocate_exact_size(Vec2::new(width, 36.0), egui::Sense::hover());
                let color = if self.error.as_ref().is_some_and(|e| {
                    e.parameter
                        .as_ref()
                        .is_some_and(|p| p.entity == Some(row.id))
                }) {
                    Color32::from_rgb(71, 33, 39)
                } else if visible_index % 2 == 0 {
                    Color32::from_rgb(29, 35, 45)
                } else {
                    Color32::from_rgb(33, 40, 51)
                };
                ui.painter().rect_filled(rect, 0, color);
                if let Some(drag) = drop_zone.dnd_hover_payload::<ParameterDrag>()
                    && drag.table == table_id
                    && drag.row != row.id
                {
                    let insert_after = authored
                        && ui.input(|i| {
                            i.pointer
                                .hover_pos()
                                .is_some_and(|p| p.y >= rect.center().y)
                        });
                    let y = if insert_after {
                        rect.bottom()
                    } else {
                        rect.top()
                    };
                    ui.painter()
                        .hline(rect.x_range(), y, ui.visuals().selection.stroke);
                    if drop_zone.dnd_release_payload::<ParameterDrag>().is_some() {
                        movement = Some((drag.row, index + usize::from(insert_after)));
                    }
                }
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
                            .id(table_id.with((row.id, column)))
                            .max_rect(cell)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| match column {
                            0 if authored => {
                                let handle = ui.interact(
                                    cell,
                                    table_id.with(("reorder", row.id)),
                                    egui::Sense::click_and_drag(),
                                );
                                let label = format!("Reorder {}", row.name);
                                handle.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        &label,
                                    )
                                });
                                let color = if handle.hovered() || handle.dragged() {
                                    ui.visuals().text_color()
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                for x in [-2.5, 2.5] {
                                    for y in [-5., 0., 5.] {
                                        ui.painter().circle_filled(
                                            cell.center() + Vec2::new(x, y),
                                            1.2,
                                            color,
                                        );
                                    }
                                }
                                handle.dnd_set_drag_payload(ParameterDrag {
                                    table: table_id,
                                    row: row.id,
                                });
                                egui::Popup::menu(&handle).show(|ui| {
                                    if ui
                                        .add_enabled(index > 0, egui::Button::new("Move up"))
                                        .clicked()
                                    {
                                        movement = Some((row.id, index - 1));
                                    }
                                    if ui
                                        .add_enabled(can_move_down, egui::Button::new("Move down"))
                                        .clicked()
                                    {
                                        movement = Some((row.id, index + 2));
                                    }
                                });
                                handle
                                    .on_hover_cursor(egui::CursorIcon::Grab)
                                    .on_hover_text("Drag to reorder. Click for move options.");
                            }
                            1 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.name)
                                            .min_size(cell.size())
                                            .vertical_align(egui::Align::Center)
                                            .hint_text("name")
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column - 1))
                                    .changed()
                            }
                            2 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.description)
                                            .min_size(cell.size())
                                            .vertical_align(egui::Align::Center)
                                            .hint_text("Description")
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column - 1))
                                    .changed()
                            }
                            3 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.expression)
                                            .min_size(cell.size())
                                            .vertical_align(egui::Align::Center)
                                            .font(egui::TextStyle::Monospace)
                                            .hint_text("Value or expression")
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column - 1))
                                    .changed()
                            }
                            4 => {
                                changed |= ui
                                    .add_sized(
                                        cell.size(),
                                        egui::TextEdit::singleline(&mut row.unit)
                                            .min_size(cell.size())
                                            .vertical_align(egui::Align::Center)
                                            .frame(crate::theme::input_frame()),
                                    )
                                    .on_hover_text(cell_help(column - 1))
                                    .changed()
                            }
                            5 => {
                                if let Some(value) = values.and_then(|v| v.values.get(&row.id)) {
                                    show_evaluated_value(ui, cell, value);
                                } else {
                                    ui.label(egui::RichText::new("—").monospace());
                                }
                            }
                            6 => {
                                changed |= ui
                                    .checkbox(&mut row.exposed, "")
                                    .on_hover_text(EXPOSE_HELP)
                                    .changed()
                            }
                            7 if authored => {
                                let response = crate::icons::compact_button(
                                    ui,
                                    crate::icons::Icon::Trash,
                                    &format!("Delete parameter {}", row.name),
                                );
                                if response.clicked() {
                                    delete = Some(index);
                                }
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
        if let Some((id, insertion)) = movement
            && let Some(from) = self.rows.iter().position(|row| row.id == id)
            && insertion != from
            && insertion != from + 1
        {
            let row = self.rows.remove(from);
            let to = insertion - usize::from(insertion > from);
            self.rows.insert(to.min(self.rows.len()), row);
            changed = true;
        }
        self.ensure_blank_row();
        if changed {
            self.refresh_preview(session);
            ui.ctx().request_repaint();
        }
    }
}

fn show_evaluated_value(
    ui: &mut egui::Ui,
    cell: Rect,
    value: &multicad_core::parameters::ParameterValue,
) {
    let text = format!(
        "{:.6} {}",
        value.quantity.in_unit(value.unit).unwrap_or(0.),
        value.unit.symbol()
    );
    let font = FontId::monospace(11.);
    let color = ui.visuals().text_color();
    let painter = ui.painter().with_clip_rect(cell);
    let integer_width = text.split_once('.').map_or(0., |(integer, _)| {
        painter
            .layout_no_wrap(integer.to_owned(), font.clone(), color)
            .size()
            .x
    });
    let galley = painter.layout_no_wrap(text.clone(), font, color);
    let decimal_x = cell.left() + cell.width() * 0.34;
    painter.galley(
        Pos2::new(
            decimal_x - integer_width,
            cell.center().y - galley.size().y / 2.,
        ),
        galley,
        color,
    );
    let response = ui.allocate_rect(cell, egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
    response.on_hover_text(format!("{text} · {}", value_help(value)));
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
#[path = "parameter_editor_workflow_tests.rs"]
mod workflow_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_navigation_retains_rows_but_refreshes_changed_or_foreign_sessions() {
        let first = Session::new(Document::new("First").unwrap()).unwrap();
        let mut editor = ParameterEditor::new(&first);
        let blank = editor.rows[0].id;
        editor.search = "width".into();
        editor.sync_clean_session(&first);
        assert_eq!(editor.rows[0].id, blank);
        assert_eq!(editor.search, "width");
        let mut second = Session::new(Document::new("Second").unwrap()).unwrap();
        assert_eq!(first.revision(), second.revision());
        editor.sync_clean_session(&second);
        assert_eq!(editor.info.name, "Second");
        second
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Mcp),
                label: "External rename".into(),
                edits: vec![Mutation::RenameDocument("Updated".into())],
            })
            .unwrap();
        editor.sync_clean_session(&second);
        assert_eq!(editor.info.name, "Updated");
        assert!(second.validate_stamp(&editor.guard).is_ok());
    }
    #[test]
    fn navigation_keeps_dirty_rows_and_their_stale_revision_guard() {
        let mut session = Session::new(Document::new("First").unwrap()).unwrap();
        let mut editor = ParameterEditor::new(&session);
        editor.rows[0].name = "width".into();
        editor.rows[0].expression = "600".into();
        session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Mcp),
                label: "External rename".into(),
                edits: vec![Mutation::RenameDocument("Updated".into())],
            })
            .unwrap();
        editor.sync_clean_session(&session);
        assert_eq!(editor.rows[0].expression, "600");
        assert!(editor.apply(&mut session, Channel::Ui).is_err());
        assert_eq!(session.document().name(), "Updated");
        assert_eq!(session.revision(), 1);
    }
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
