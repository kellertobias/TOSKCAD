use super::library::choice;
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, EntityId},
    materials::*,
};
use std::collections::BTreeSet;
#[derive(Clone, PartialEq)]
struct Draft {
    document: EntityId,
    revision: u64,
    parts: Vec<EntityId>,
    assignment: Assignment,
    original: Assignment,
    tags: String,
    original_tags: String,
    tag_mode: usize,
}
type DisplayCache = ((EntityId, u64, Vec<EntityId>), Vec<String>);
#[derive(Default)]
pub struct PartEditor {
    draft: Option<Draft>,
    error: Option<String>,
    cached: Option<DisplayCache>,
}
impl PartEditor {
    pub fn dirty(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|d| d.assignment != d.original || d.tags != d.original_tags)
    }
    fn load(&mut self, s: &Session, parts: &[EntityId]) {
        self.draft = parts
            .first()
            .and_then(|id| Assignment::from_part(s.document(), *id).ok())
            .map(|assignment| {
                let tags = if parts.len() == 1 {
                    match &s.document().get(parts[0]).expect("known Part").data {
                        EntityData::Part { tags, .. } => {
                            tags.iter().cloned().collect::<Vec<_>>().join("\n")
                        }
                        _ => String::new(),
                    }
                } else {
                    String::new()
                };
                Draft {
                    document: s.document().id(),
                    revision: s.revision(),
                    parts: parts.to_vec(),
                    original: assignment.clone(),
                    assignment,
                    tags: tags.clone(),
                    original_tags: tags,
                    tag_mode: if parts.len() == 1 { 2 } else { 0 },
                }
            });
        self.error = None;
    }
    fn apply_properties(&mut self, s: &mut Session) -> Result<(), String> {
        let d = self.draft.as_mut().ok_or("Select a Part")?;
        if d.document != s.document().id() {
            return Err("Part draft belongs to another document".into());
        }
        let p = prepare_assignment(
            s,
            d.revision,
            Actor::local(Channel::Ui),
            &d.parts,
            &d.assignment,
        )
        .map_err(|e| e.to_string())?;
        s.commit(p).map_err(|e| e.to_string())?;
        d.revision = s.revision();
        d.original = d.assignment.clone();
        Ok(())
    }
    fn apply_tags(&mut self, s: &mut Session) -> Result<(), String> {
        let d = self.draft.as_mut().ok_or("Select a Part")?;
        if d.document != s.document().id() {
            return Err("Tag draft belongs to another document".into());
        }
        let values = d
            .tags
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        let edit = match d.tag_mode {
            0 => TagEdit::Add(values),
            1 => TagEdit::Remove(values),
            _ => TagEdit::Replace(values),
        };
        let p = prepare_tags(s, d.revision, Actor::local(Channel::Ui), &d.parts, &edit)
            .map_err(|e| e.to_string())?;
        s.commit(p).map_err(|e| e.to_string())?;
        d.revision = s.revision();
        d.original_tags = d.tags.clone();
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
        let changed = self.draft.as_ref().is_none_or(|d| {
            d.document != s.document().id() || d.revision != s.revision() || d.parts != parts
        });
        if changed && !self.dirty() {
            self.load(s, &parts);
        }
        if self.draft.is_none() {
            return;
        }
        let key = (s.document().id(), s.revision(), parts.clone());
        if self.cached.as_ref().is_none_or(|c| c.0 != key) {
            let mut lines = Vec::new();
            for id in &parts {
                let name = &s.document().get(*id).expect("known Part").name;
                match resolve(s.document(), *id) {
                    Ok(p) => {
                        let material = p
                            .material
                            .and_then(|m| s.document().get(m))
                            .map_or("None", |e| e.name.as_str());
                        lines.push(format!("{name} · {material}"));
                        if let Some((_, stock)) = p.stock {
                            lines.push(stock_summary(&stock.specification));
                        }
                        if let Some(h) = p.hatch {
                            lines.push(format!(
                                "Section hatch: {:?}, {}°, {} mm spacing on paper",
                                h.pattern, h.angle_deg, h.spacing_mm
                            ));
                        }
                        for d in p.diagnostics {
                            lines.push(format!("Review: {}", d.message));
                        }
                    }
                    Err(e) => lines.push(format!("Unresolved: {e}")),
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
        let mut apply = false;
        let mut tags = false;
        let mut discard = false;
        ui.add_enabled_ui(enabled,|ui|{
   let d=self.draft.as_mut().expect("loaded");
   ui.small(format!("{} selected part(s)",d.parts.len()));
   if d.parts.len()>1 {ui.colored_label(Color32::LIGHT_YELLOW,"Applying properties replaces these assignments on every selected Part. Tags have a separate explicit mode.");}
   choice(ui,"Manufacturing material",&mut d.assignment.material,s,0);
   choice(ui,"Stock",&mut d.assignment.stock,s,1);
   choice(ui,"Appearance override",&mut d.assignment.appearance,s,2);
   choice(ui,"Section hatch override",&mut d.assignment.hatch,s,3);
   if ui.button("Apply part properties").on_hover_text("Assign current definitions and acknowledge changed library content in one Undo step").clicked(){apply=true;}
   ui.separator();ui.label("Part tags");
   ui.small("Exact, case-sensitive tags; one tag per line.");
   egui::ComboBox::from_id_salt("tag-mode").selected_text(["Add tags","Remove tags","Replace tags"][d.tag_mode]).show_ui(ui,|ui|{for (i,label) in ["Add tags","Remove tags","Replace tags"].into_iter().enumerate(){ui.selectable_value(&mut d.tag_mode,i,label);}});
   ui.add(egui::TextEdit::multiline(&mut d.tags).hint_text("cabinet\ncnc").desired_rows(2).desired_width(ui.available_width())).on_hover_text("Tags select Parts independently of their names or display visibility");
   ui.horizontal_wrapped(|ui|{if ui.button("Apply tags").on_hover_text("Apply the selected tag mode to all selected Parts in one Undo step").clicked(){tags=true;}
if ui.button("Discard property edits").on_hover_text("Reload canonical assignments and tags").clicked(){discard=true;}});
  });
        if apply {
            self.error = self.apply_properties(s).err();
        }
        if tags {
            self.error = self.apply_tags(s).err();
        }
        if discard {
            self.load(s, &parts);
        }
        if changed && self.dirty() {
            ui.colored_label(
                Color32::LIGHT_YELLOW,
                "Source or selection changed. Discard before starting a different edit.",
            );
        }
        if let Some(e) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, e);
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
    use multicad_core::{
        commands::Request,
        document::{Entity, Mutation},
    };
    fn fixture() -> (Session, Vec<EntityId>, EntityId) {
        let mut doc = Document::new("Part properties UI").unwrap();
        let mut parts = vec![];
        for name in ["Side", "Shelf"] {
            let e = Entity::new(
                name,
                EntityData::Part {
                    features: vec![],
                    material: None,
                    tags: BTreeSet::new(),
                },
            );
            parts.push(e.id);
            doc.insert(e).unwrap();
        }
        let material = Entity::new(
            "Birch",
            EntityData::Material {
                density_kg_m3: Some(650.),
                color_rgba: [170, 130, 70, 255],
                section_hatch: None,
            },
        );
        let id = material.id;
        doc.insert(material).unwrap();
        (Session::new(doc).unwrap(), parts, id)
    }
    fn frame(
        ctx: &egui::Context,
        editor: &mut PartEditor,
        s: &mut Session,
        parts: &[EntityId],
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240., 850.),
                )),
                events,
                ..Default::default()
            },
            |ui| editor.show(ui, s, parts, true),
        );
        output.textures_delta.clear();
        output.shapes
    }
    fn find(shapes: &[egui::epaint::ClippedShape], label: &str) -> egui::Pos2 {
        fn shape(s: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
            match s {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| shape(s, label)),
                _ => None,
            }
        }
        shapes
            .iter()
            .find_map(|s| shape(&s.shape, label))
            .expect("button rendered")
    }
    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    }
    #[test]
    fn selected_parts_apply_properties_and_tags_through_real_inspector_buttons() {
        let (mut s, parts, material) = fixture();
        let before = s.document().clone();
        let mut editor = PartEditor::default();
        editor.load(&s, &parts);
        editor.draft.as_mut().unwrap().assignment.material = Some(material);
        let ctx = egui::Context::default();
        let shapes = frame(&ctx, &mut editor, &mut s, &parts, vec![]);
        assert_eq!(s.document(), &before);
        assert!(editor.dirty());
        let pos = find(&shapes, "Apply part properties");
        frame(&ctx, &mut editor, &mut s, &parts, click(pos));
        assert_eq!(s.revision(), 1);
        for id in &parts {
            assert_eq!(resolve(s.document(), *id).unwrap().material, Some(material));
        }
        editor.draft.as_mut().unwrap().tags = "cabinet\ncnc".into();
        let shapes = frame(&ctx, &mut editor, &mut s, &parts, vec![]);
        let pos = find(&shapes, "Apply tags");
        frame(&ctx, &mut editor, &mut s, &parts, click(pos));
        assert_eq!(s.revision(), 2);
        assert_eq!(
            TagQuery {
                all: ["cnc".into()].into(),
                ..Default::default()
            }
            .select(s.document())
            .unwrap()
            .len(),
            2
        );
        s.undo(2, Actor::local(Channel::Ui)).unwrap();
        assert!(
            TagQuery {
                all: ["cnc".into()].into(),
                ..Default::default()
            }
            .select(s.document())
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn stale_selection_and_wrong_document_drafts_do_not_apply() {
        let (mut s, parts, material) = fixture();
        let mut editor = PartEditor::default();
        editor.load(&s, &parts);
        editor.draft.as_mut().unwrap().assignment.material = Some(material);
        s.apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Rename source".into(),
            edits: vec![Mutation::RenameDocument("Changed".into())],
        })
        .unwrap();
        let before = s.document().clone();
        assert!(editor.apply_properties(&mut s).is_err());
        assert_eq!(s.document(), &before);
        let mut other = Session::new(Document::new("Other").unwrap()).unwrap();
        assert!(editor.apply_tags(&mut other).is_err());
    }
}
