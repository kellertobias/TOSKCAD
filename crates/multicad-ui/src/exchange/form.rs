use crate::egui;
use multicad_core::{
    commands::Session,
    document::{EntityData, EntityId},
    drawings,
    exchange::*,
    materials::TagQuery,
};
use std::collections::BTreeSet;
pub(super) struct Choice {
    pub id: EntityId,
    pub name: String,
    pub copy: bool,
}
pub(super) struct PageChoice {
    pub reference: PageRef,
    pub book_name: String,
    pub number: String,
    pub title: String,
}
#[derive(Default)]
pub(super) struct Catalog {
    pub sources: Vec<Choice>,
    pub assemblies: Vec<Choice>,
    pub pages: Vec<PageChoice>,
    pub errors: Vec<String>,
}
impl Catalog {
    pub fn new(s: &Session) -> Self {
        let mut value = Self::default();
        for e in s.document().entities() {
            match e.data {
                EntityData::Part { .. } | EntityData::Instance { .. } => {
                    value.sources.push(Choice {
                        id: e.id,
                        name: e.name.clone(),
                        copy: matches!(e.data, EntityData::Instance { .. }),
                    })
                }
                EntityData::Assembly { .. } => value.assemblies.push(Choice {
                    id: e.id,
                    name: e.name.clone(),
                    copy: false,
                }),
                _ => {}
            }
        }
        for book in drawings::books(s.document()) {
            match drawings::read(s.document(), book) {
                Ok(source) => {
                    for page in source.pages {
                        value.pages.push(PageChoice {
                            reference: PageRef {
                                book,
                                page: page.id,
                            },
                            book_name: s.document().get(book).expect("known book").name.clone(),
                            number: page.number,
                            title: page.info.title,
                        });
                    }
                }
                Err(error) => value.errors.push(error.to_string()),
            }
        }
        value
    }
}
#[derive(Clone, PartialEq)]
pub(super) struct Draft {
    pub id: EntityId,
    pub name: String,
    pub source: SetDefinition,
    pub document: EntityId,
    pub revision: u64,
    pub all: String,
    pub any: String,
    pub none: String,
}
impl Draft {
    pub fn new(s: &Session) -> Self {
        Self {
            id: EntityId::new(),
            name: "New export set".into(),
            source: Default::default(),
            document: s.document().id(),
            revision: s.revision(),
            all: String::new(),
            any: String::new(),
            none: String::new(),
        }
    }
    pub fn load(s: &Session, id: EntityId) -> Result<Self, String> {
        let source = read(s.document(), id).map_err(|e| e.to_string())?;
        let text = |tags: &BTreeSet<String>| tags.iter().cloned().collect::<Vec<_>>().join("\n");
        let (all, any, none) = source
            .scope
            .tags
            .as_ref()
            .map(|q| (text(&q.all), text(&q.any), text(&q.none)))
            .unwrap_or_default();
        Ok(Self {
            id,
            name: s
                .document()
                .get(id)
                .ok_or("Export set missing")?
                .name
                .clone(),
            source,
            document: s.document().id(),
            revision: s.revision(),
            all,
            any,
            none,
        })
    }
    pub fn sync_query(&mut self) {
        if self.source.scope.tags.is_some() {
            let parse = |s: &str| {
                s.lines()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect()
            };
            self.source.scope.tags = Some(TagQuery {
                all: parse(&self.all),
                any: parse(&self.any),
                none: parse(&self.none),
            });
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        s: &Session,
        captured: &[EntityId],
        catalog: &Catalog,
    ) {
        ui.label("Set name");
        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()))
            .on_hover_text(
                "Saved name; the set's stable identity is independent of its display name",
            );
        ui.separator();
        ui.strong("Part scope");
        let mut mode = match self.source.scope.root {
            ScopeRoot::Project => 0,
            ScopeRoot::Assembly { .. } => 1,
            ScopeRoot::Explicit { .. } => 2,
        };
        let old = mode;
        egui::ComboBox::from_id_salt("export-scope")
            .selected_text(["Whole project", "Assembly", "Explicit parts and copies"][mode])
            .show_ui(ui, |ui| {
                for (i, label) in ["Whole project", "Assembly", "Explicit parts and copies"]
                    .into_iter()
                    .enumerate()
                {
                    ui.selectable_value(&mut mode, i, label);
                }
            });
        if mode != old {
            self.source.scope.root = match mode {
                0 => ScopeRoot::Project,
                1 => ScopeRoot::Assembly {
                    assembly: catalog.assemblies.first().map_or_else(
                        || {
                            EntityId::parse("00000000-0000-0000-0000-000000000000")
                                .expect("nil UUID")
                        },
                        |e| e.id,
                    ),
                },
                _ => ScopeRoot::Explicit {
                    entities: captured.to_vec(),
                },
            };
        }
        match &mut self.source.scope.root {
            ScopeRoot::Assembly { assembly } => {
                let label = s
                    .document()
                    .get(*assembly)
                    .map_or("Choose an assembly", |e| e.name.as_str());
                egui::ComboBox::from_id_salt("export-assembly")
                    .selected_text(label)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for e in &catalog.assemblies {
                            ui.selectable_value(assembly, e.id, &e.name);
                        }
                    });
            }
            ScopeRoot::Explicit { entities } => {
                if ui.button("Use captured model selection").on_hover_text("Use the Part and occurrence identities captured when entering Export; visibility is ignored").clicked(){*entities=captured.to_vec();}
                egui::ScrollArea::vertical()
                    .id_salt("export-explicit-sources")
                    .max_height(130.)
                    .show(ui, |ui| {
                        for e in &catalog.sources {
                            let mut selected = entities.contains(&e.id);
                            let label = if e.copy {
                                format!("{} (copy)", e.name)
                            } else {
                                e.name.clone()
                            };
                            if ui.checkbox(&mut selected, label).changed() {
                                if selected {
                                    entities.push(e.id);
                                } else {
                                    entities.retain(|id| *id != e.id);
                                }
                            }
                        }
                    });
            }
            _ => {}
        }
        let mut query = self.source.scope.tags.is_some();
        if ui.checkbox(&mut query,"Filter by part tags").on_hover_text("Intersect this scope with exact case-sensitive tags; every copy uses its source Part's tags").changed(){self.source.scope.tags=if query{Some(TagQuery::default())}else{None};}
        if query {
            ui.small("One tag per line. All, Any and Exclude combine.");
            for (label, text) in [
                ("All tags", &mut self.all),
                ("Any tag", &mut self.any),
                ("Exclude tags", &mut self.none),
            ] {
                ui.label(label);
                ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(2)
                        .desired_width(ui.available_width()),
                );
            }
            self.sync_query();
        }
        ui.collapsing("Drawing pages", |ui| {
            let mut mode = match self.source.pages {
                Pages::None => 0,
                Pages::Project => 1,
                Pages::Selected { .. } => 2,
            };
            let old = mode;
            egui::ComboBox::from_id_salt("export-pages")
                .selected_text(["No pages", "All current pages", "Explicit pages"][mode])
                .show_ui(ui, |ui| {
                    for (i, label) in ["No pages", "All current pages", "Explicit pages"]
                        .into_iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut mode, i, label);
                    }
                });
            if mode != old {
                self.source.pages = match mode {
                    0 => Pages::None,
                    1 => Pages::Project,
                    _ => Pages::Selected { pages: vec![] },
                };
            }
            if let Pages::Selected { pages } = &mut self.source.pages {
                if ui
                    .button("Clear selected pages")
                    .on_hover_text("Remove the explicit page selection, then choose current pages")
                    .clicked()
                {
                    pages.clear();
                }
                for page in &catalog.pages {
                    let mut selected = pages.contains(&page.reference);
                    if ui
                        .checkbox(
                            &mut selected,
                            format!("{} / {} · {}", page.book_name, page.number, page.title),
                        )
                        .changed()
                    {
                        if selected {
                            pages.push(page.reference.clone());
                        } else {
                            pages.retain(|p| *p != page.reference);
                        }
                    }
                }
                ui.small(
                    "Explicit pages keep selection order; Preview lists their exact output order.",
                );
            }
            for error in &catalog.errors {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
        ui.separator();
        ui.strong("Output recipes");
        let mut remove = None;
        for (index, recipe) in self.source.recipes.iter_mut().enumerate() {
            ui.push_id(recipe.id, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut recipe.name).desired_width(150.));
                    if ui
                        .small_button("Remove recipe")
                        .on_hover_text("Remove this recipe from the saved set")
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
                output_controls(ui, &mut recipe.output);
                ui.separator();
            });
        }
        if let Some(i) = remove {
            self.source.recipes.remove(i);
        }
        if ui.add_enabled(self.source.recipes.len()<16,egui::Button::new("Add recipe")).on_hover_text("Add a recipe with a stable identity; unsupported providers remain explicit in Preview").clicked(){self.source.recipes.push(Recipe::step("STEP"));}
        ui.collapsing("File naming and revision", |ui| {
            ui.label("Filename template");
            ui.add(
                egui::TextEdit::singleline(&mut self.source.naming.template)
                    .desired_width(ui.available_width()),
            );
            ui.small("{project}, {set}, {part}, {id}, {occurrence}, {recipe}, {revision}, {page}");
            let mut custom = matches!(self.source.naming.revision, RevisionRule::Label(_));
            if ui
                .checkbox(&mut custom, "Use an authored revision label")
                .changed()
            {
                self.source.naming.revision = if custom {
                    RevisionRule::Label("A".into())
                } else {
                    RevisionRule::ContentHash
                };
            }
            if let RevisionRule::Label(label) = &mut self.source.naming.revision {
                ui.text_edit_singleline(label);
            } else {
                ui.small(
                    "Content-hash revision stays stable when the identical source is reopened.",
                );
            }
            ui.small("Preview shows the exact portable filenames. Collisions block the whole run.");
        });
    }
}
fn number(ui: &mut egui::Ui, label: &str, v: &mut f64, suffix: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).speed(0.1).suffix(suffix));
    });
}
fn output_controls(ui: &mut egui::Ui, output: &mut RecipeOutput) {
    let mut kind = match output {
        RecipeOutput::Step { .. } => 0,
        RecipeOutput::Mesh { format, .. } => match format {
            MeshFormat::Obj => 1,
            MeshFormat::Stl => 2,
            MeshFormat::ThreeMf => 3,
            MeshFormat::Glb => 4,
        },
        RecipeOutput::PlanarDxf { .. } => 5,
        RecipeOutput::DrawingDxf { .. } => 6,
        RecipeOutput::Pdf => 7,
    };
    let old = kind;
    egui::ComboBox::from_id_salt("recipe-format")
        .selected_text(
            [
                "STEP",
                "OBJ",
                "STL",
                "3MF",
                "GLB",
                "Planar DXF",
                "Drawing DXF",
                "Multi-page PDF",
            ][kind],
        )
        .show_ui(ui, |ui| {
            for (i, label) in [
                "STEP",
                "OBJ",
                "STL",
                "3MF",
                "GLB",
                "Planar DXF",
                "Drawing DXF",
                "Multi-page PDF",
            ]
            .into_iter()
            .enumerate()
            {
                ui.selectable_value(&mut kind, i, label);
            }
        });
    if kind != old {
        *output = match kind {
            0 => Recipe::step("STEP").output,
            1..=4 => RecipeOutput::Mesh {
                format: [
                    MeshFormat::Obj,
                    MeshFormat::Stl,
                    MeshFormat::ThreeMf,
                    MeshFormat::Glb,
                ][kind - 1],
                units: LengthUnit::Millimeter,
                hierarchy: HierarchyMode::SeparateParts,
                names: NameHandling::FileNamesOnly,
                aids: AidPolicy::IncludeAll,
                quality: Default::default(),
            },
            5 => RecipeOutput::PlanarDxf {
                units: LengthUnit::Millimeter,
                layers: Default::default(),
                aids: AidPolicy::IncludeAll,
            },
            6 => RecipeOutput::DrawingDxf {
                units: LengthUnit::Millimeter,
                layers: Default::default(),
            },
            _ => RecipeOutput::Pdf,
        };
    }
    if kind != 0 {
        ui.colored_label(
            egui::Color32::LIGHT_YELLOW,
            "This format is not available yet. The recipe can be saved, but it blocks Run.",
        );
    }
    match output {
        RecipeOutput::Step {
            units,
            hierarchy,
            names,
            aids,
        }
        | RecipeOutput::Mesh {
            units,
            hierarchy,
            names,
            aids,
            ..
        } => {
            units_control(ui, units);
            egui::ComboBox::from_id_salt("export-hierarchy")
                .selected_text(match hierarchy {
                    HierarchyMode::SeparateParts => "Separate part files",
                    HierarchyMode::PreserveAssembly => "Preserve assembly",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        hierarchy,
                        HierarchyMode::SeparateParts,
                        "Separate part files",
                    );
                    ui.selectable_value(
                        hierarchy,
                        HierarchyMode::PreserveAssembly,
                        "Preserve assembly",
                    );
                });
            let mut product = *names == NameHandling::ProductNames;
            if ui.checkbox(&mut product,"Write product-name attributes").on_hover_text("File naming is always separate. STEP product-name attributes are not yet supported").changed(){*names=if product{NameHandling::ProductNames}else{NameHandling::FileNamesOnly};}
            aids_control(ui, aids);
        }
        RecipeOutput::PlanarDxf { units, aids, .. } => {
            units_control(ui, units);
            aids_control(ui, aids);
        }
        RecipeOutput::DrawingDxf { units, .. } => units_control(ui, units),
        RecipeOutput::Pdf => {}
    }
    if let RecipeOutput::Mesh { quality, .. } = output {
        number(
            ui,
            "Chord deviation",
            &mut quality.chord_deviation_mm,
            " mm",
        );
        number(
            ui,
            "Angular deviation",
            &mut quality.angular_deviation_deg,
            "°",
        );
    }
    if let RecipeOutput::PlanarDxf { layers, .. } | RecipeOutput::DrawingDxf { layers, .. } = output
    {
        for (label, value) in [
            ("Cut layer", &mut layers.cut),
            ("Bend layer", &mut layers.bend),
            ("Construction layer", &mut layers.construction),
            ("Annotation layer", &mut layers.annotation),
        ] {
            ui.label(label);
            ui.text_edit_singleline(value);
        }
    }
}
fn units_control(ui: &mut egui::Ui, units: &mut LengthUnit) {
    egui::ComboBox::from_id_salt("export-units")
        .selected_text(format!("{units:?}"))
        .show_ui(ui, |ui| {
            for u in [
                LengthUnit::Millimeter,
                LengthUnit::Centimeter,
                LengthUnit::Meter,
                LengthUnit::Inch,
            ] {
                ui.selectable_value(units, u, format!("{u:?}"));
            }
        })
        .response
        .on_hover_text(
            "STEP currently supports millimeters; other units are explicitly rejected in Preview",
        );
}
fn aids_control(ui: &mut egui::Ui, aids: &mut AidPolicy) {
    let mut exclude = *aids == AidPolicy::ExcludeRemovable;
    if ui.checkbox(&mut exclude,"Exclude removable design aids").on_hover_text("Removable-aid exclusion is not available yet. Run reports Unsupported instead of guessing from names or tags").changed(){*aids=if exclude{AidPolicy::ExcludeRemovable}else{AidPolicy::IncludeAll};}
}
