use super::{
    library::choice,
    tag_input::{self, Tags},
};
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{Document, EntityData, EntityId, Mutation},
    materials::*,
};
use std::collections::BTreeSet;
struct Draft {
    document: EntityId,
    revision: u64,
    guard: Prepared,
    parts: Vec<EntityId>,
    assignment: Assignment,
    original: Assignment,
    tags: Tags,
    original_tags: BTreeSet<String>,
    tag_mode: usize,
    original_mode: usize,
}
type DisplayCache = ((EntityId, u64, Vec<EntityId>), Vec<String>);
#[derive(Default)]
pub struct PartEditor {
    draft: Option<Draft>,
    error: Option<String>,
    cached: Option<DisplayCache>,
}
fn guard(session: &Session) -> Result<Prepared, String> {
    session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Open Part Inspector".into(),
            edits: vec![],
        })
        .map_err(|e| e.to_string())
}
impl PartEditor {
    pub fn dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| {
            d.assignment != d.original
                || !d.tags.text.trim().is_empty()
                || d.tags.values() != d.original_tags
                || d.tag_mode != d.original_mode
        })
    }
    fn load(&mut self, s: &Session, parts: &[EntityId]) {
        self.draft = None;
        self.error = None;
        let Some(part) = parts.first() else {
            return;
        };
        let loaded = (|| {
            let assignment =
                Assignment::from_part(s.document(), *part).map_err(|e| e.to_string())?;
            let original_tags = if parts.len() == 1 {
                match &s
                    .document()
                    .get(*part)
                    .ok_or("Selected Part is missing")?
                    .data
                {
                    EntityData::Part { tags, .. } => tags.clone(),
                    _ => BTreeSet::new(),
                }
            } else {
                BTreeSet::new()
            };
            let mode = if parts.len() == 1 { 2 } else { 0 };
            Ok::<_, String>(Draft {
                document: s.document().id(),
                revision: s.revision(),
                guard: guard(s)?,
                parts: parts.to_vec(),
                original: assignment.clone(),
                assignment,
                tags: Tags::from_values(original_tags.clone()),
                original_tags,
                tag_mode: mode,
                original_mode: mode,
            })
        })();
        match loaded {
            Ok(draft) => self.draft = Some(draft),
            Err(error) => self.error = Some(error),
        }
    }
    fn current(&self, s: &Session, parts: &[EntityId]) -> Result<(), String> {
        let d = self.draft.as_ref().ok_or("Select a Part")?;
        if d.document != s.document().id()
            || d.revision != s.revision()
            || d.parts != parts
            || s.validate_preview(&d.guard).is_err()
        {
            return Err("Source or selection changed. Discard tag edits before editing the current selection.".into());
        }
        Ok(())
    }
    fn apply_properties(&mut self, s: &mut Session, parts: &[EntityId]) -> Result<(), String> {
        self.current(s, parts)?;
        let d = self.draft.as_ref().expect("current draft");
        let chosen = &d.assignment;
        let previous = &d.original;
        let mut edits = Vec::new();
        for part in &d.parts {
            let mut assignment =
                Assignment::from_part(s.document(), *part).map_err(|e| e.to_string())?;
            for (value, old, target) in [
                (chosen.material, previous.material, &mut assignment.material),
                (chosen.stock, previous.stock, &mut assignment.stock),
                (
                    chosen.appearance,
                    previous.appearance,
                    &mut assignment.appearance,
                ),
                (chosen.hatch, previous.hatch, &mut assignment.hatch),
            ] {
                if value != old {
                    *target = value;
                }
            }
            let candidate = prepare_assignment(
                s,
                d.revision,
                Actor::local(Channel::Ui),
                &[*part],
                &assignment,
            )
            .map_err(|e| e.to_string())?;
            edits.push(Mutation::Replace(
                candidate
                    .document()
                    .get(*part)
                    .expect("prepared Part")
                    .clone(),
            ));
            let (id, mut properties) = part_properties(candidate.document(), *part)
                .map_err(|e| e.to_string())?
                .ok_or("Prepared Part properties are missing")?;
            // Only the edited assignment acknowledges its library content.
            // Existing review diagnostics on other definitions stay visible.
            if let Some((_, old)) =
                part_properties(s.document(), *part).map_err(|e| e.to_string())?
            {
                for (changed, target, original) in [
                    (
                        chosen.material != previous.material,
                        &mut properties.material,
                        old.material,
                    ),
                    (
                        chosen.stock != previous.stock,
                        &mut properties.stock,
                        old.stock,
                    ),
                    (
                        chosen.appearance != previous.appearance,
                        &mut properties.appearance,
                        old.appearance,
                    ),
                    (
                        chosen.hatch != previous.hatch,
                        &mut properties.hatch,
                        old.hatch,
                    ),
                ] {
                    if !changed {
                        *target = original;
                    }
                }
            }
            let entity = LibraryDefinition::PartProperties(properties)
                .entity(
                    id,
                    &candidate
                        .document()
                        .get(id)
                        .expect("prepared metadata")
                        .name,
                )
                .map_err(|e| e.to_string())?;
            edits.push(if s.document().get(id).is_some() {
                Mutation::Replace(entity)
            } else {
                Mutation::Insert(entity)
            });
        }
        let prepared = s
            .prepare(Request {
                expected_revision: d.revision,
                actor: Actor::local(Channel::Ui),
                label: "Assign part properties".into(),
                edits,
            })
            .map_err(|e| e.to_string())?;
        s.commit(prepared).map_err(|e| e.to_string())?;
        let next = guard(s)?;
        let d = self.draft.as_mut().expect("current draft");
        d.revision = s.revision();
        d.guard = next;
        d.original = d.assignment.clone();
        Ok(())
    }
    fn apply_tags(&mut self, s: &mut Session, parts: &[EntityId]) -> Result<(), String> {
        self.current(s, parts)?;
        let d = self.draft.as_ref().expect("current draft");
        let values = d.tags.values();
        let edit = match d.tag_mode {
            0 => TagEdit::Add(values.clone()),
            1 => TagEdit::Remove(values.clone()),
            _ => TagEdit::Replace(values.clone()),
        };
        let prepared = prepare_tags(s, d.revision, Actor::local(Channel::Ui), &d.parts, &edit)
            .map_err(|e| e.to_string())?;
        s.commit(prepared).map_err(|e| e.to_string())?;
        let next = guard(s)?;
        let d = self.draft.as_mut().expect("current draft");
        d.revision = s.revision();
        d.guard = next;
        let values = if d.tag_mode == 2 {
            values
        } else {
            BTreeSet::new()
        };
        d.tags = Tags::from_values(values.clone());
        d.original_tags = values;
        d.original_mode = d.tag_mode;
        Ok(())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        s: &mut Session,
        selection: &[EntityId],
        enabled: bool,
    ) {
        let parts = selected_parts(s.document(), selection);
        let changed = self.current(s, &parts).is_err();
        if changed && !self.dirty() {
            self.load(s, &parts);
        }
        let Some(d) = self.draft.as_ref() else {
            return;
        };
        let stale = self.current(s, &parts).is_err();
        let names = d
            .parts
            .iter()
            .map(|id| {
                s.document()
                    .get(*id)
                    .map_or("Missing Part", |e| e.name.as_str())
            })
            .collect::<Vec<_>>()
            .join(", ");
        let key = (s.document().id(), s.revision(), parts.clone());
        if self.cached.as_ref().is_none_or(|c| c.0 != key) {
            let mut lines = Vec::new();
            for id in &parts {
                let name = &s.document().get(*id).expect("selected Part").name;
                match resolve(s.document(), *id) {
                    Ok(properties) => {
                        let material = properties
                            .material
                            .and_then(|m| s.document().get(m))
                            .map_or("None", |e| e.name.as_str());
                        lines.push(format!("{name} · {material}"));
                        if let Some((_, stock)) = properties.stock {
                            lines.push(stock_summary(&stock.specification));
                        }
                        if let Some(h) = properties.hatch {
                            lines.push(format!(
                                "Section hatch: {:?}, {}°, {} mm spacing on paper",
                                h.pattern, h.angle_deg, h.spacing_mm
                            ));
                        }
                        for diagnostic in properties.diagnostics {
                            lines.push(format!("Review: {}", diagnostic.message));
                        }
                    }
                    Err(error) => lines.push(format!("Unresolved: {error}")),
                }
            }
            self.cached = Some((key, lines));
        }
        ui.separator();
        ui.strong("Manufacturing properties");
        if let Some((_, lines)) = &self.cached {
            for line in lines {
                ui.label(line);
            }
        }
        if stale {
            ui.colored_label(Color32::LIGHT_YELLOW,format!("Tag draft for {names}. Source or selection changed; discard to load the current selection."));
        }
        let mut properties_changed = false;
        ui.add_enabled_ui(enabled&&!stale,|ui|{let d=self.draft.as_mut().expect("loaded");ui.small(format!("{} selected part(s)",d.parts.len()));if d.parts.len()>1{ui.small("Each changed property applies immediately to all selected Parts; other assignments are kept.");}properties_changed|=choice(ui,"Manufacturing material",&mut d.assignment.material,s,0);properties_changed|=choice(ui,"Stock",&mut d.assignment.stock,s,1);properties_changed|=choice(ui,"Appearance override",&mut d.assignment.appearance,s,2);properties_changed|=choice(ui,"Section hatch override",&mut d.assignment.hatch,s,3);});
        if properties_changed {
            match self.apply_properties(s, &parts) {
                Ok(()) => self.error = None,
                Err(error) => {
                    let d = self.draft.as_mut().expect("loaded");
                    d.assignment = d.original.clone();
                    self.error = Some(error);
                }
            }
        }
        ui.separator();
        ui.label("Part tags");
        ui.small("Comma creates a tag. Use the checkmark to apply; tags are case-sensitive.");
        ui.add_enabled_ui(enabled && !stale, |ui| {
            let d = self.draft.as_mut().expect("loaded");
            egui::ComboBox::from_id_salt("tag-mode")
                .selected_text(["Add tags", "Remove tags", "Replace tags"][d.tag_mode])
                .show_ui(ui, |ui| {
                    for (i, label) in ["Add tags", "Remove tags", "Replace tags"]
                        .into_iter()
                        .enumerate()
                    {
                        if ui.selectable_value(&mut d.tag_mode, i, label).clicked() {
                            ui.close();
                        }
                    }
                });
        });
        let dirty = self.dirty();
        let d = self.draft.as_mut().expect("loaded");
        let actions = tag_input::show(
            ui,
            &mut d.tags,
            enabled && !stale,
            enabled && !stale && dirty,
            dirty || stale,
        );
        if actions.discard {
            self.load(s, &parts);
        } else if actions.apply {
            self.error = self.apply_tags(s, &parts).err();
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
}
fn selected_parts(doc: &Document, ids: &[EntityId]) -> Vec<EntityId> {
    let mut result = BTreeSet::new();
    for id in ids {
        match doc.get(*id).map(|e| &e.data) {
            Some(EntityData::Part { .. }) => {
                result.insert(*id);
            }
            Some(EntityData::Instance { component, .. })
                if matches!(
                    doc.get(*component).map(|e| &e.data),
                    Some(EntityData::Part { .. })
                ) =>
            {
                result.insert(*component);
            }
            _ => {}
        }
    }
    result.into_iter().collect()
}

fn stock_summary(stock: &StockKind) -> String {
    match stock {
        StockKind::Panel {
            thickness_mm,
            length_mm,
            width_mm,
            grain,
        } => format!(
            "Panel · {thickness_mm} mm · {length_mm} × {width_mm} mm · grain {}",
            match grain {
                Grain::None => "none",
                Grain::AlongLength => "along length",
                Grain::AcrossLength => "across length",
            }
        ),
        StockKind::Sheet {
            thickness_mm,
            length_mm,
            width_mm,
            inside_bend_radius_mm,
            k_factor,
        } => format!(
            "Sheet · {thickness_mm} mm · {length_mm} × {width_mm} mm · bend R{inside_bend_radius_mm} mm · K {k_factor}"
        ),
        StockKind::Profile {
            designation,
            length_mm,
            width_mm,
            height_mm,
            wall_mm,
        } => format!(
            "{designation} · {length_mm} mm long · {width_mm} × {height_mm} mm{}",
            wall_mm.map_or(String::new(), |v| format!(" · wall {v} mm"))
        ),
        StockKind::Print {
            process,
            nozzle_mm,
            layer_height_mm,
        } => format!(
            "{process} · layer {layer_height_mm} mm{}",
            nozzle_mm.map_or(String::new(), |v| format!(" · nozzle {v} mm"))
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{document::Entity, project_file::ContentHash};
    use std::collections::BTreeMap;
    struct Harness {
        ctx: egui::Context,
        editor: PartEditor,
        session: Session,
        selection: Vec<EntityId>,
        shapes: Vec<egui::epaint::ClippedShape>,
        bounds: BTreeMap<String, egui::Rect>,
    }
    fn fixture() -> (Harness, Vec<EntityId>) {
        let mut document = Document::new("Part Inspector").unwrap();
        let mut ids = vec![];
        for name in ["Side", "Shelf"] {
            let part = Entity::new(
                name,
                EntityData::Part {
                    features: vec![],
                    material: None,
                    tags: BTreeSet::new(),
                },
            );
            ids.push(part.id);
            document.insert(part).unwrap();
        }
        let mut library = vec![];
        for name in ["Birch", "Steel"] {
            let material = Entity::new(
                name,
                EntityData::Material {
                    density_kg_m3: Some(650.),
                    color_rgba: [150, 170, 190, 255],
                    section_hatch: None,
                },
            );
            library.push(material.id);
            document.insert(material).unwrap();
        }
        for (name, definition) in [
            (
                "Panel stock",
                LibraryDefinition::Stock(Stock {
                    material: library[0],
                    specification: StockKind::Panel {
                        thickness_mm: 18.,
                        length_mm: 2440.,
                        width_mm: 1220.,
                        grain: Grain::AlongLength,
                    },
                }),
            ),
            (
                "Blue appearance",
                LibraryDefinition::Appearance(Appearance::default()),
            ),
            (
                "Fine hatch",
                LibraryDefinition::Hatch(HatchStyle::default()),
            ),
        ] {
            let id = EntityId::new();
            document
                .insert(definition.entity(id, name).unwrap())
                .unwrap();
            library.push(id);
        }
        let mut h = Harness {
            ctx: egui::Context::default(),
            editor: PartEditor::default(),
            session: Session::new(document).unwrap(),
            selection: ids,
            shapes: vec![],
            bounds: BTreeMap::new(),
        };
        h.frame(vec![]);
        h.frame(vec![]);
        (h, library)
    }
    impl Harness {
        fn frame(&mut self, events: Vec<egui::Event>) {
            self.ctx.enable_accesskit();
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(250., 900.),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    self.editor
                        .show(ui, &mut self.session, &self.selection, true)
                },
            );
            output.textures_delta.clear();
            if let Some(update) = &output.platform_output.accesskit_update {
                for (_, node) in &update.nodes {
                    if let (Some(label), Some(b)) = (node.label(), node.bounds()) {
                        self.bounds.insert(
                            label.into(),
                            egui::Rect::from_min_max(
                                egui::pos2(b.x0 as f32, b.y0 as f32),
                                egui::pos2(b.x1 as f32, b.y1 as f32),
                            ),
                        );
                    }
                }
            }
            self.shapes = output.shapes;
        }
        fn text_point(&self, label: &str) -> egui::Pos2 {
            fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                    _ => None,
                }
            }
            self.shapes
                .iter()
                .find_map(|s| find(&s.shape, label))
                .unwrap_or_else(|| panic!("Missing text {label}; {:?}", self.editor.error))
        }
        fn click(&mut self, point: egui::Pos2) {
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            self.frame(vec![]);
        }
        fn button(&mut self, label: &str) {
            let point = self
                .bounds
                .get(label)
                .unwrap_or_else(|| panic!("Missing button {label}"))
                .center();
            self.click(point);
        }
        fn enter(&mut self, text: &str) {
            self.frame(vec![egui::Event::Text(text.into())]);
            self.frame(vec![]);
        }
        fn input(&mut self) {
            let point = self.text_point("Add a tag…");
            self.click(point);
        }
        fn choose(&mut self, label: &str, option: &str) {
            let point = self.text_point(label) + egui::vec2(0., 23.);
            self.click(point);
            let point = self.text_point(option);
            self.click(point);
            self.click(egui::pos2(245., 3.));
        }
        fn mode(&mut self, mode: &str) {
            let current = ["Add tags", "Remove tags", "Replace tags"]
                [self.editor.draft.as_ref().unwrap().tag_mode];
            let point = self.text_point(current);
            self.click(point);
            let point = self.text_point(mode);
            self.click(point);
        }
        fn tags(&self, id: EntityId) -> BTreeSet<String> {
            match &self.session.document().get(id).unwrap().data {
                EntityData::Part { tags, .. } => tags.clone(),
                _ => panic!(),
            }
        }
    }
    #[test]
    fn comma_paste_tokens_continued_typing_dedup_removal_and_inline_actions() {
        let (mut h, _) = fixture();
        h.selection.truncate(1);
        h.frame(vec![]);
        let part = h.selection[0];
        let before = h.session.document().clone();
        h.input();
        h.enter("cabinet,");
        assert_eq!(
            h.editor.draft.as_ref().unwrap().tags.tokens,
            vec!["cabinet"]
        );
        assert!(h.editor.draft.as_ref().unwrap().tags.text.is_empty());
        h.enter("cn");
        h.enter("c,");
        assert_eq!(
            h.editor.draft.as_ref().unwrap().tags.tokens,
            vec!["cabinet", "cnc"]
        );
        h.frame(vec![egui::Event::Paste("cnc, CNC, export".into())]);
        h.frame(vec![]);
        assert_eq!(
            h.editor.draft.as_ref().unwrap().tags.tokens,
            vec!["cabinet", "cnc", "CNC"]
        );
        assert_eq!(h.editor.draft.as_ref().unwrap().tags.text, " export");
        assert_eq!(h.session.document(), &before);
        h.button("Remove tag cnc");
        assert!(
            !h.editor
                .draft
                .as_ref()
                .unwrap()
                .tags
                .tokens
                .contains(&"cnc".into())
        );
        h.button("Apply tags");
        assert_eq!(
            h.tags(part),
            BTreeSet::from(["cabinet".into(), "CNC".into(), "export".into()])
        );
        assert!(!h.editor.dirty());
        h.input();
        h.enter("discard-me,");
        h.button("Discard tag edits");
        assert!(!h.tags(part).contains("discard-me"));
        assert!(!h.editor.dirty());
        for tag in ["cabinet", "CNC", "export"] {
            h.button(&format!("Remove tag {tag}"));
        }
        h.button("Apply tags");
        assert!(h.tags(part).is_empty());
        h.session
            .undo(h.session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(h.tags(part).len(), 3);
    }
    #[test]
    fn immediate_property_dropdowns_preserve_pending_tags_and_other_mixed_assignments() {
        let (mut h, library) = fixture();
        let second = h.selection[1];
        let mut assignment = Assignment::from_part(h.session.document(), second).unwrap();
        assignment.material = Some(library[1]);
        h.session
            .commit(
                prepare_assignment(
                    &h.session,
                    0,
                    Actor::local(Channel::Ui),
                    &[second],
                    &assignment,
                )
                .unwrap(),
            )
            .unwrap();
        h.frame(vec![]);
        h.input();
        h.enter("maker,");
        let before = h.session.document().clone();
        h.choose("Appearance override", "Blue appearance");
        assert_eq!(h.session.revision(), 2);
        assert!(h.editor.dirty());
        assert_eq!(h.editor.draft.as_ref().unwrap().revision, 2);
        assert_eq!(h.tags(second), BTreeSet::new());
        assert_eq!(
            Assignment::from_part(h.session.document(), second)
                .unwrap()
                .material,
            Some(library[1])
        );
        for id in &h.selection {
            assert_eq!(
                Assignment::from_part(h.session.document(), *id)
                    .unwrap()
                    .appearance,
                Some(library[3])
            );
        }
        h.button("Apply tags");
        assert!(h.tags(second).contains("maker"));
        h.session
            .undo(h.session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        h.session
            .undo(h.session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(h.session.document(), &before);
        h.frame(vec![]);
        h.choose("Manufacturing material", "Birch");
        assert!(h.selection.iter().all(|id| {
            Assignment::from_part(h.session.document(), *id)
                .unwrap()
                .material
                == Some(library[0])
        }));
        h.choose("Stock", "Panel stock");
        assert!(h.selection.iter().all(|id| {
            Assignment::from_part(h.session.document(), *id)
                .unwrap()
                .stock
                == Some(library[2])
        }));
        h.choose("Section hatch override", "Fine hatch");
        assert!(h.selection.iter().all(|id| {
            Assignment::from_part(h.session.document(), *id)
                .unwrap()
                .hatch
                == Some(library[4])
        }));
    }
    #[test]
    fn multiselect_add_remove_replace_empty_modes_are_atomic_and_undoable() {
        let (mut h, _) = fixture();
        let ids = h.selection.clone();
        h.input();
        h.enter("cnc,CNC,cnc,");
        h.button("Apply tags");
        for id in &ids {
            assert_eq!(h.tags(*id), BTreeSet::from(["cnc".into(), "CNC".into()]));
        }
        h.mode("Remove tags");
        h.input();
        h.enter("cnc,");
        h.button("Apply tags");
        for id in &ids {
            assert_eq!(h.tags(*id), BTreeSet::from(["CNC".into()]));
        }
        h.mode("Replace tags");
        h.button("Apply tags");
        assert!(ids.iter().all(|id| h.tags(*id).is_empty()));
        h.session
            .undo(h.session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert!(
            ids.iter()
                .all(|id| h.tags(*id) == BTreeSet::from(["CNC".into()]))
        );
    }
    #[test]
    fn stale_selection_revision_and_foreign_session_never_receive_pending_tags() {
        let (mut h, _) = fixture();
        let parts = h.selection.clone();
        h.selection = vec![parts[0]];
        h.frame(vec![]);
        h.input();
        h.enter("first-only,");
        let before = h.session.document().clone();
        h.selection = vec![parts[1]];
        h.frame(vec![]);
        h.button("Apply tags");
        assert_eq!(h.session.document(), &before);
        assert_eq!(h.editor.draft.as_ref().unwrap().parts, vec![parts[0]]);
        h.button("Discard tag edits");
        assert_eq!(h.editor.draft.as_ref().unwrap().parts, vec![parts[1]]);
        h.input();
        h.enter("second-only,");
        h.button("Apply tags");
        assert!(h.tags(parts[0]).is_empty());
        assert!(h.tags(parts[1]).contains("second-only"));
        h.input();
        h.enter("stale,");
        h.session
            .undo(h.session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        h.frame(vec![]);
        h.button("Apply tags");
        assert!(h.tags(parts[1]).is_empty());
        let source = ContentHash::document(h.session.document()).unwrap();
        let mut foreign = Session::new(h.session.document().clone()).unwrap();
        assert!(h.editor.apply_tags(&mut foreign, &h.selection).is_err());
        assert_eq!(ContentHash::document(foreign.document()).unwrap(), source);
        h.button("Discard tag edits");
        assert!(!h.editor.dirty());
    }
    #[test]
    fn inline_actions_stay_inside_the_outlined_tag_field() {
        let (mut h, _) = fixture();
        h.input();
        h.enter("long-label-one,second,third,");
        let apply = h.bounds["Apply tags"];
        let cancel = h.bounds["Discard tag edits"];
        let outline = h
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(r) if r.stroke.color == Color32::from_rgb(78, 98, 122) => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("outlined field");
        assert!(outline.contains_rect(apply));
        assert!(outline.contains_rect(cancel));
        assert!(apply.right() <= cancel.left());
        assert!(!h.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t)if t.galley.text()=="Apply part properties")));
    }
    #[test]
    fn changing_appearance_does_not_acknowledge_unedited_stock_content() {
        let (mut h, library) = fixture();
        h.selection.truncate(1);
        h.frame(vec![]);
        let part = h.selection[0];
        h.choose("Manufacturing material", "Birch");
        h.choose("Stock", "Panel stock");
        let before = part_properties(h.session.document(), part)
            .unwrap()
            .unwrap()
            .1
            .stock
            .unwrap();
        let mut stock = match read(h.session.document().get(library[2]).unwrap())
            .unwrap()
            .unwrap()
        {
            LibraryDefinition::Stock(stock) => stock,
            _ => panic!(),
        };
        if let StockKind::Panel { thickness_mm, .. } = &mut stock.specification {
            *thickness_mm = 19.;
        }
        let entity = LibraryDefinition::Stock(stock)
            .entity(library[2], "Panel stock")
            .unwrap();
        h.session
            .commit(
                prepare_library(
                    &h.session,
                    h.session.revision(),
                    Actor::local(Channel::Ui),
                    &[entity],
                )
                .unwrap(),
            )
            .unwrap();
        h.frame(vec![]);
        h.choose("Appearance override", "Blue appearance");
        let properties = part_properties(h.session.document(), part)
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(properties.stock, Some(before));
        assert!(
            resolve(h.session.document(), part)
                .unwrap()
                .diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::DefinitionChanged)
        );
    }
    #[test]
    fn same_document_and_revision_in_a_new_session_cannot_receive_old_draft() {
        let (mut h, _) = fixture();
        h.selection.truncate(1);
        h.frame(vec![]);
        h.input();
        h.enter("pending,");
        let source = h.session.document().clone();
        let mut other = Session::new(source.clone()).unwrap();
        assert_eq!(other.revision(), h.session.revision());
        assert_eq!(other.document().id(), h.session.document().id());
        assert!(h.editor.apply_tags(&mut other, &h.selection).is_err());
        assert_eq!(other.document(), &source);
        assert!(h.editor.dirty());
    }
    #[test]
    fn invalid_tag_keeps_source_unchanged_and_can_be_removed_and_repaired() {
        let (mut h, _) = fixture();
        h.selection.truncate(1);
        h.frame(vec![]);
        let source = h.session.document().clone();
        h.input();
        let long = "x".repeat(129);
        h.enter(&format!("{long},"));
        h.button("Apply tags");
        assert_eq!(h.session.document(), &source);
        assert!(h.editor.error.as_deref().unwrap().contains("1–128"));
        h.button(&format!("Remove tag {long}"));
        h.input();
        h.enter("valid,");
        h.button("Apply tags");
        assert_eq!(h.tags(h.selection[0]), BTreeSet::from(["valid".into()]));
        assert!(h.editor.error.is_none());
    }
}
