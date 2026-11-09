use super::{
    SymbolAction, SymbolTool,
    fields::{self, entity_choice, entity_label},
    forms::{Form, Operation},
};
use crate::{controls, egui, library::facets::SearchQuery};
use multicad_core::{
    commands::Session,
    document::{Document, EntityData, EntityId},
    project_file::ContentHash,
    symbols::{self, SymbolDefinition, SymbolKind, SymbolPlan},
};
use std::collections::{BTreeMap, BTreeSet};
enum Next {
    Place(EntityId),
    PlaceLibrary(Box<SymbolDefinition>),
    Import(Box<SymbolDefinition>),
    Update(EntityId),
    Replacement(Box<SymbolDefinition>),
    Edit(EntityId),
    AuthorConstruction(EntityId),
}
pub(super) struct Catalogue<'a> {
    pub embedded: &'a [(EntityId, SymbolDefinition)],
    pub embedded_pins: &'a BTreeMap<EntityId, ContentHash>,
    pub embedded_search: &'a BTreeMap<EntityId, super::installed::SearchIndex>,
    pub library: &'a [SymbolDefinition],
    pub library_pins: &'a [ContentHash],
    pub library_search: &'a [super::installed::SearchIndex],
    pub installed: &'a mut super::installed::Installed,
}
pub(super) fn show(
    form: &mut Form,
    ui: &mut egui::Ui,
    session: &Session,
    catalogue: Catalogue<'_>,
    enabled: bool,
) -> (Option<SymbolAction>, Option<String>) {
    // Reserve the trailing gutter for the Inspector's overlay scrollbar.
    let width = controls::inspector_width(ui);
    ui.set_max_width((width - 8.).max(0.));
    controls::inspector_layout(ui);
    ui.strong(form.tool.label());
    let document = session.document();
    let definitions = catalogue.embedded;
    let library = catalogue.library;
    let mut changed = false;
    let mut action = None;
    let mut next = None;
    let mut error = None;
    ui.add_enabled_ui(enabled, |ui| { match &mut form.operation {
        Operation::Library => {}
        Operation::ChooseEdit{selected}=>{
            let entries=definitions.iter().flat_map(|(id,_)|symbols::instances(document,*id).unwrap_or_default().into_iter().map(|(_,instance)|instance.instance).chain(symbols::architecture::architecture_instances(document,*id).unwrap_or_default().into_iter().map(|(_,instance)|instance.output.anchors()[1]))).map(|id|(id,entity_label(document,id))).collect::<Vec<_>>();
            if entity_choice(ui,"symbol-edit-instance","Symbol instance",selected,&entries,false)&&let Some(id)=selected{next=Some(Next::Edit(*id));}
        }
        Operation::Author(request)=>{
            let mut prior = definitions.iter().any(|(_, definition)| definition.symbol == request.symbol).then_some(request.symbol);
            let identities = definitions.iter().map(|(_, definition)| (definition.symbol, format!("{} · {}",definition.name,&definition.symbol.to_string()[..6]))).collect::<BTreeMap<_,_>>();
            let entries = identities.into_iter().collect::<Vec<_>>();
            if entity_choice(ui,"symbol-author-identity","Definition identity",&mut prior,&entries,true) {
                if let Some(symbol)=prior && let Some((_,definition))=definitions.iter().find(|(_,definition)| definition.symbol==symbol) {
                    request.symbol=definition.symbol;request.revision=format!("{}.next",definition.revision);
                } else { request.symbol=EntityId::new();request.revision="1".into(); }
                changed=true;
            }
            ui.small("None creates a new symbol. Choose an existing identity to author a new revision from the selected source.");
            let owned=symbols::owned_entities_in_session(session).unwrap_or_default();
            let roots=document.entities().filter(|e|!owned.contains(&e.id) && (matches!(e.data,EntityData::Part{..}|EntityData::Assembly{..}) || super::architecture::author::construction_role(document,e.id).is_some())).map(|e|(e.id,entity_label(document,e.id))).collect::<Vec<_>>();
            let mut root=document.get(request.root).map(|e|e.id);
            if entity_choice(ui,"symbol-create-root","Authored source",&mut root,&roots,false)&&let Some(root)=root{if super::architecture::author::construction_role(document,root).is_some(){next=Some(Next::AuthorConstruction(root));}else{request.root=root;changed=true;}}
            changed|=fields::text(ui,"Symbol name",&mut request.name,"Library display name; identity and saved instance pins remain stable");
            changed|=fields::text(ui,"Revision",&mut request.revision,"Explicit revision label. Different contents cannot claim the same symbol and revision");
            ui.horizontal(|ui|{
                changed|=ui.selectable_value(&mut request.kind,SymbolKind::Static,"Static").on_hover_text("Fixed design values for every placement").changed();
                changed|=ui.selectable_value(&mut request.kind,SymbolKind::Dynamic,"Dynamic").on_hover_text("Each placement supplies only the parameters you expose below").changed();
            });
            if request.kind==SymbolKind::Dynamic {
                ui.strong("Expose instance inputs");
                let evaluated=Some(session.parameters());
                let mut exposed=request.exposed.iter().copied().collect::<BTreeSet<_>>();
                for entity in document.entities().filter(|e|matches!(e.data,EntityData::Parameter{..})) {
                    let mut checked=exposed.contains(&entity.id);
                    if ui.checkbox(&mut checked,&entity.name).on_hover_text(format!("{}\nID: {}",entity.description,entity.id)).changed(){if checked{exposed.insert(entity.id);}else{exposed.remove(&entity.id);}changed=true;}
                    if let Some(value)=evaluated.as_ref().and_then(|values|values.values.get(&entity.id).and_then(|v|values.value(entity.id,v.unit).ok().map(|n|fields::display_value(n,v.unit.symbol())))){ui.small(value);}
                    if let EntityData::Parameter{expression,..}=&entity.data{ui.small(format!("Default: {expression}"));}
                }
                request.exposed=exposed.into_iter().collect();
            }else if !request.exposed.is_empty(){request.exposed.clear();changed=true;}
            let mut plan=request.plan.as_ref().map(|plan|plan.sketch);
            let sketches=document.entities().filter(|e|matches!(e.data,EntityData::Sketch{..})).map(|e|(e.id,entity_label(document,e.id))).collect::<Vec<_>>();
            if entity_choice(ui,"symbol-create-plan","Optional simplified 2D sketch",&mut plan,&sketches,true){request.plan=plan.map(|sketch|SymbolPlan{sketch});changed=true;}
            ui.small("The chosen source and plan are embedded together. Parameter exposure does not change the source project's inputs.");
        }
        Operation::ArchitecturalAuthor{request,editor}=>{
            ui.strong(match request.interface.role { symbols::architecture::ArchitectureRole::Wall=>"Reusable layered wall", symbols::architecture::ArchitectureRole::Fence=>"Reusable fence" });
            ui.label(format!("Source: {}",document.get(request.interface.construction).map_or("Missing construction",|e|e.name.as_str())));
            changed|=fields::text(ui,"Symbol name",&mut request.symbol.name,"Name shown in the existing Library");
            changed|=fields::text(ui,"Revision",&mut request.symbol.revision,"Publish changes as an explicit revision");
            let identities=definitions.iter().filter(|(_,d)|super::architecture::role(d)==Some(request.interface.role) && d.root==request.blueprint && d.template.id()==document.id()).map(|(_,d)|(d.symbol,d.name.clone())).collect::<BTreeMap<_,_>>().into_iter().collect::<Vec<_>>();
            let mut identity=identities.iter().find(|(id,_)|*id==request.symbol.symbol).map(|(id,_)|*id);
            if entity_choice(ui,"architectural-author-identity","Definition identity",&mut identity,&identities,true){
                if let Some(id)=identity { request.symbol.symbol=id;if let Some((_,old))=definitions.iter().find(|(_,d)|d.symbol==id){request.symbol.revision=format!("{}.next",old.revision);} }
                else{request.symbol.symbol=EntityId::new();request.symbol.revision="1".into();}
                changed=true;
            }
            changed|=editor.show(ui,&mut request.interface);
            if let Ok(dependencies)=request.interface.dependencies(document){
                ui.strong("Numeric parameters");
                for entity in document.entities().filter(|e|dependencies.contains(&e.id)&&matches!(e.data,EntityData::Parameter{..})){
                    let mut exposed=request.symbol.exposed.contains(&entity.id);
                    if ui.checkbox(&mut exposed,&entity.name).changed(){if exposed{request.symbol.exposed.push(entity.id);}else{request.symbol.exposed.retain(|id|*id!=entity.id);}changed=true;}
                }
            }
            ui.small("Preview and Apply create the reusable definition. Save the project, set its name/description/preview in file metadata, then Add to Library.");
        }
        Operation::Import(definition)=>{
            summary(ui,definition);
            ui.label("Preview and Apply to embed this immutable revision. Then use Library to place it.");
        }
        Operation::Place{definition,library,parent,name,values,pose,..}=>{
            if library.is_none() {
                let entries=definitions.iter().filter(|(_,def)|super::architecture::role(def).is_none()).map(|(id,def)|(*id,format!("{} · revision {}",def.name,def.revision))).collect::<Vec<_>>();
                if entity_choice(ui,"symbol-place-definition","Embedded definition",definition,&entries,false)&&let Some(id)=definition{next=Some(Next::Place(*id));}
            } else { ui.small("Library pin · embedded together with this occurrence on Apply"); }
            let source = library.as_ref().or_else(|| definitions.iter().find(|(id,_)|Some(*id)==*definition).map(|(_,source)| source));
            if let Some(source)=source{
                summary(ui,source);
                let parent_label = parent.map_or_else(|| format!("{} (root)", document.name()), |id| {
                    if id != document.id() && document.get(id).is_none() { return "Missing parent · reopen placement".into(); }
                    multicad_core::hierarchy::Index::new(document)
                        .map(|index| crate::model_tree::container_path(document, &index, Some(id)))
                        .unwrap_or_else(|_| "Unavailable parent".into())
                });
                ui.add(egui::Label::new(format!("Parent: {parent_label}")).wrap());
                changed|=fields::text(ui,"Instance name",name,"Name for this occurrence; the source definition and other instances are unchanged");
                changed|=values.show(ui,source);
                values.show_hardware_attribution(ui, source);
                changed|=pose.show(ui);
            }else{ui.small("Choose an embedded definition or use Place on a Library card.");}
        }
        Operation::ArchitecturalEdit{values,..}=>{
            ui.label("Pinned architectural construction");
            changed|=values.show(ui);
            ui.small("Path, level, openings and placement are available in the Wall or Fence inspector.");
        }
        Operation::Edit{request,definition,values}=>{
            summary(ui,definition);ui.small(format!("Instance: {}",instance_label(document,request.record)));
            if let Ok(instance)=symbols::read_instance(document,request.record)&&let Ok(Some((_,source)))=symbols::project_source_of(document,instance.definition){ui.small(format!("Referenced project: {} · pinned revision {}",source.path,definition.revision));}
            changed|=values.show(ui,definition);
            values.show_hardware_attribution(ui, definition);
            ui.small("Only this instance's exposed inputs change. Move changes its shared 2D/3D placement.");
        }
        Operation::Update{definition,replacement,request,values,architecture_values,report}=>{
            let entries=definitions.iter().map(|(id,def)|(*id,format!("{} · revision {}",def.name,def.revision))).collect::<Vec<_>>();
            if entity_choice(ui,"symbol-update-definition","Embedded definition to replace",definition,&entries,false){*replacement=None;*request=None;values.clear();*report=None;changed=true;}
            if let Some((old_id,old))=definitions.iter().find(|(id,_)|Some(*id)==*definition){
                ui.small(format!("Pinned revision: {}",old.revision));
                let selected=replacement.as_ref().map(|d|format!("{} · revision {}",d.name,d.revision)).unwrap_or_else(||"Choose incoming revision".into());
                controls::inspector_combo(ui,"symbol-update-replacement").selected_text(selected).show_ui(ui,|ui|{
                    for (candidate,pin) in definitions.iter().map(|(id,d)|(d,catalogue.embedded_pins[id])).chain(library.iter().zip(catalogue.library_pins).map(|(d,pin)|(d,*pin))).filter(|(d,pin)|d.symbol==old.symbol&&*pin!=catalogue.embedded_pins[old_id]){
                        let label=format!("{} · revision {}",candidate.name,candidate.revision);
                        if ui.selectable_label(report.as_ref().is_some_and(|report|report.new_pin==pin),label).on_hover_text(pin.to_string()).clicked(){next=Some(Next::Replacement(Box::new(candidate.clone())));ui.close();}
                    }
                }).response.on_hover_text("Only revisions of the same stable symbol identity can replace this definition");
                if ui.button("Open library file…").clicked(){action=Some(SymbolAction::ImportLibrary);}
            }
            if let (Some(replacement),Some(report))=(replacement.as_ref(),report.as_ref()){
                ui.strong("Review definition changes");
                ui.small(format!("Old pin: {}",report.old_pin));ui.small(format!("New pin: {}",report.new_pin));
                ui.label(format!("{} changed entities · {} affected instance(s)",report.changed_entities.len(),report.affected_instances.len()));
                let old=definition.and_then(|id|definitions.iter().find(|(key,_)|*key==id).map(|(_,d)|d));
                for (heading,ids) in [("Added numeric parameters",&report.added_parameters),("Removed numeric parameters",&report.removed_parameters),("Changed numeric parameters/defaults",&report.changed_parameters)] {
                    ui.strong(heading);
                    if ids.is_empty(){ui.small("None");}
                    for id in ids {
                        let name=replacement.template.get(*id).or_else(||old.and_then(|d|d.template.get(*id))).map_or("Missing parameter",|e|e.name.as_str());
                        ui.label(name);
                        if let Some(old)=old.and_then(|d|d.template.get(*id)) && let EntityData::Parameter{expression,unit,..}=&old.data{ui.small(format!("Before: {expression} ({unit})"));}
                        if let Some(new)=replacement.template.get(*id) && let EntityData::Parameter{expression,unit,..}=&new.data{ui.small(format!("After: {expression} ({unit})"));}
                    }
                }
                for conflict in &report.conflicts{ui.colored_label(egui::Color32::LIGHT_YELLOW,format!("Override needs review: {} · {}",instance_label(document,conflict.instance),conflict.reason));}
                for instance in &report.affected_instances{ui.label(format!("Affected: {}",instance_label(document,*instance)));}
                for (record,fields) in values {
                    ui.push_id(("symbol-update-instance",*record),|ui| {
                        ui.group(|ui|{ui.strong(instance_label(document,*record));changed|=fields.show(ui,replacement);});
                    });
                }
                for change in &report.architecture_changes {
                    ui.push_id(("symbol-update-architecture",change.record),|ui| {
                    ui.group(|ui| {
                        ui.strong(instance_label(document,change.output.anchors()[1]));
                        for layer in &change.removed_layers { ui.label(format!("Removed layer: {}",layer.name)); }
                        for recipe in &change.changed_recipes { ui.label(format!("Replaced infill: {}",recipe.name)); }
                        if !change.removed_members.is_empty() { ui.label(format!("{} physical members will be removed",change.removed_members.len())); }
                        if let Some(fields)=architecture_values.get_mut(&change.record) { fields.show_changes(ui,change); changed|=fields.show(ui); }
                    });
                    });
                }
                if let Some(error)=&report.architecture_error { ui.colored_label(egui::Color32::LIGHT_YELLOW,error); }

                ui.small("Compatible overrides are retained. Uncheck removed or incompatible values explicitly. Preview validates the resolved inputs and exact geometry; Apply updates all listed instances in one undoable action.");
            }
        }
    }});
    if changed {
        form.changed();
    }
    if let Some(next) = next {
        error = transition(form, document, next).err();
    }
    (action, error)
}

fn transition(form: &mut Form, document: &Document, next: Next) -> Result<(), String> {
    match next {
        Next::Place(id) => form.place(document, id),
        Next::PlaceLibrary(definition) => form.place_library(document, &definition),
        Next::AuthorConstruction(id) => form.author_construction(document, id),
        Next::Import(definition) => {
            form.import(*definition);
            Ok(())
        }
        Next::Update(id) => {
            form.tool = SymbolTool::Update;
            form.operation = Operation::Update {
                definition: Some(id),
                replacement: None,
                request: None,
                values: BTreeMap::new(),
                architecture_values: BTreeMap::new(),
                report: None,
            };
            form.changed();
            Ok(())
        }
        Next::Replacement(definition) => form.set_replacement(document, *definition),
        Next::Edit(id) => Form::existing(document, id).map(|mut replacement| {
            replacement.version = form.version.saturating_add(1);
            *form = replacement;
        }),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LibraryKey {
    Loaded(ContentHash),
    Embedded(EntityId),
    Installed(EntityId),
}

pub(super) enum PrimaryAction {
    Loaded(usize),
    Embedded(EntityId),
    Request(SymbolAction),
}

#[derive(Default)]
pub(super) struct LibraryBrowser {
    selected: Option<LibraryKey>,
    focused_search: bool,
    filter: String,
    scroll_to_top: bool,
}

impl LibraryBrowser {
    pub(super) fn reset_catalogue(&mut self) {
        self.selected = None;
        self.scroll_to_top = true;
    }
}

pub(super) struct PrimaryControls<'a> {
    selected: &'a mut Option<LibraryKey>,
    choice: Option<(LibraryKey, String, PrimaryAction)>,
}

impl<'a> PrimaryControls<'a> {
    pub(super) fn new(selected: &'a mut Option<LibraryKey>) -> Self {
        Self {
            selected,
            choice: None,
        }
    }

    pub(super) fn title(
        &mut self,
        ui: &mut egui::Ui,
        key: LibraryKey,
        name: &str,
        enabled: bool,
        action: impl FnOnce() -> PrimaryAction,
    ) {
        if !enabled {
            ui.strong(name);
            return;
        }
        let response = ui.selectable_label(
            *self.selected == Some(key),
            egui::RichText::new(name).strong(),
        );
        if response.gained_focus() {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        if response.clicked() || response.has_focus() {
            *self.selected = Some(key);
        }
        if self.choice.is_none() || *self.selected == Some(key) {
            self.choice = Some((key, name.to_owned(), action()));
        }
    }
}

pub(super) fn show_library(
    form: &mut Form,
    browser: &mut LibraryBrowser,
    ui: &mut egui::Ui,
    session: &Session,
    catalogue: Catalogue<'_>,
    enabled: bool,
    failure: Option<&str>,
) -> (Option<SymbolAction>, Option<String>, bool) {
    controls::inspector_layout(ui);
    let close = ui
        .horizontal(|ui| {
            ui.heading("Library");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.button("Close")
                    .on_hover_text("Close Library (Escape)")
                    .clicked()
            })
            .inner
        })
        .inner;
    let mut action = None;
    let mut next = None;
    let document = session.document();
    ui.add_enabled_ui(enabled, |ui| {
        let label = ui.label("Filter symbols");
        let search = ui
            .add(
                egui::TextEdit::singleline(&mut form.filter)
                    .id_salt("library-search")
                    .hint_text("Search by name, size, or M3")
                    .desired_width(controls::inspector_width(ui)),
            )
            .labelled_by(label.id);
        if !browser.focused_search {
            search.request_focus();
            browser.focused_search = true;
        }
        if search.changed() {
            form.changed();
        }
        if ui
            .button("Open library file…")
            .on_hover_text("Load definitions without changing this project")
            .clicked()
        {
            action = Some(SymbolAction::ImportLibrary);
        }
    });

    if browser.filter != form.filter {
        browser.filter.clone_from(&form.filter);
        browser.reset_catalogue();
    }
    let filter = SearchQuery::new(&form.filter);
    let catalogue_height = (ui.available_height() - 61.).max(35.);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("library-catalogue")
        .max_height(catalogue_height)
        .auto_shrink([false, false]);
    if std::mem::take(&mut browser.scroll_to_top) {
        scroll = scroll.vertical_scroll_offset(0.);
    }
    let mut primary = PrimaryControls::new(&mut browser.selected);
    scroll.show(ui, |ui| {
            if let Some(failure) = failure {
                ui.colored_label(egui::Color32::LIGHT_RED, failure);
            }
            ui.add_enabled_ui(enabled, |ui| {
                catalogue.installed.show_filters(ui);
                ui.small("M3 matches a 3 mm nominal diameter. Other terms search names and source dimensions.");
                let facets = catalogue.installed.facets();
                if !catalogue.library.is_empty() {
                    ui.separator();
                    ui.strong("Loaded library · choose a definition to place");
                    let mut shown = 0;
                    for (index, ((definition, pin), search)) in catalogue.library.iter()
                        .zip(catalogue.library_pins).zip(catalogue.library_search).enumerate()
                    {
                        if !search.matches(&filter, facets) {
                            continue;
                        }
                        shown += 1;
                        ui.push_id(pin.0, |ui| ui.group(|ui| {
                            let embedded = catalogue.embedded.iter().any(|(id, existing)| {
                                existing.symbol == definition.symbol && catalogue.embedded_pins.get(id) == Some(pin)
                            });
                            let unavailable = if super::architecture::role(definition).is_some() {
                                Some("Architectural construction needs a path and level. Import explicitly, then Draw wall or Draw fence.".to_string())
                            } else {
                                symbols::library_placement_definition(document, definition).err().map(|error| error.to_string())
                            };
                            primary.title(ui, LibraryKey::Loaded(*pin), &definition.name,
                                unavailable.is_none(), || PrimaryAction::Loaded(index));
                            summary_details(ui, definition);
                            if embedded {
                                ui.small("This exact revision is embedded");
                            }
                            if let Some(reason) = &unavailable {
                                ui.colored_label(egui::Color32::LIGHT_RED, reason);
                            }
                            if focus_button(ui.add_enabled(unavailable.is_none(), egui::Button::new("Place"))
                                .on_hover_text("Preview and Apply embed this exact pin and place one occurrence in one transaction"))
                            {
                                next = Some(Next::PlaceLibrary(Box::new(definition.clone())));
                            }
                            if !embedded && focus_button(ui.button("Import definition")
                                .on_hover_text("Embed this immutable definition without placing an occurrence"))
                            {
                                next = Some(Next::Import(Box::new(definition.clone())));
                            }
                            super::installed::show_cards(ui, search, &filter, facets);
                        }));
                    }
                    if shown == 0 {
                        ui.label("No matching library symbols loaded");
                    }
                }
                if !catalogue.embedded.is_empty() {
                    ui.separator();
                    ui.strong("Embedded in this project");
                    let mut shown = 0;
                    for (id, definition) in catalogue.embedded {
                        let search = &catalogue.embedded_search[id];
                        if !search.matches(&filter, facets) {
                            continue;
                        }
                        shown += 1;
                        ui.push_id(*id, |ui| ui.group(|ui| {
                            let construction = super::architecture::role(definition);
                            let request = construction.map(|role| SymbolAction::DrawConstruction {
                                definition: *id,
                                role,
                            });
                            primary.title(ui, LibraryKey::Embedded(*id), &definition.name, true, || {
                                request.clone().map_or(PrimaryAction::Embedded(*id), PrimaryAction::Request)
                            });
                            summary_details(ui, definition);
                            if let Some(SymbolAction::DrawConstruction { role, .. }) = request.as_ref() {
                                let label = match role {
                                    symbols::architecture::ArchitectureRole::Wall => "Draw wall",
                                    symbols::architecture::ArchitectureRole::Fence => "Draw fence",
                                };
                                if focus_button(ui.button(label)) {
                                    action = request;
                                }
                            } else if focus_button(ui.button("Place")
                                .on_hover_text("Place this pinned definition; dynamic symbols ask for exposed values"))
                            {
                                next = Some(Next::Place(*id));
                            }
                            ui.horizontal_wrapped(|ui| {
                                if focus_button(ui.small_button("Save definition…")) {
                                    action = Some(SymbolAction::SaveDefinition(*id));
                                }
                                if focus_button(ui.small_button("Compare update")
                                    .on_hover_text("Choose a revision explicitly; this never upgrades instances automatically"))
                                {
                                    next = Some(Next::Update(*id));
                                }
                            });
                            super::installed::show_cards(ui, search, &filter, facets);
                        }));
                    }
                    if shown == 0 {
                        ui.label("No matching embedded symbols");
                    }
                }
                ui.separator();
                if let Some(installed_action) = catalogue.installed.show(ui, &filter, &mut primary) {
                    action = Some(installed_action);
                }
            });
        });
    let choice = primary.choice;
    browser.selected = choice.as_ref().map(|(key, _, _)| *key);
    ui.separator();
    ui.horizontal(|ui| {
        let label = match choice.as_ref().map(|(_, _, action)| action) {
            Some(PrimaryAction::Request(SymbolAction::OpenInstalled(_))) => {
                "Choose matching definition"
            }
            Some(PrimaryAction::Request(SymbolAction::DrawConstruction { .. })) => "Draw selected",
            _ => "Place selected",
        };
        let place = ui.add_enabled(
            enabled && choice.is_some(),
            egui::Button::new(label)
                .fill(ui.visuals().selection.bg_fill)
                .min_size(egui::vec2(115., 30.)),
        );
        if let Some((_, name, command)) = choice {
            ui.add(egui::Label::new(format!("Selected: {name}")).truncate());
            if place.clicked() {
                match command {
                    PrimaryAction::Loaded(index) => {
                        next = Some(Next::PlaceLibrary(Box::new(
                            catalogue.library[index].clone(),
                        )));
                    }
                    PrimaryAction::Embedded(id) => next = Some(Next::Place(id)),
                    PrimaryAction::Request(request) => action = Some(request),
                }
            }
        }
    });
    let error = next.and_then(|next| transition(form, document, next).err());
    (action, error, close)
}

pub(super) fn focus_button(response: egui::Response) -> bool {
    if response.gained_focus() {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response.clicked()
}
fn summary(ui: &mut egui::Ui, definition: &SymbolDefinition) {
    ui.strong(&definition.name);
    summary_details(ui, definition);
}

fn summary_details(ui: &mut egui::Ui, definition: &SymbolDefinition) {
    ui.small(format!(
        "Revision {} · {}",
        definition.revision,
        match definition.kind {
            SymbolKind::Static => "Static",
            SymbolKind::Dynamic => "Dynamic",
        }
    ));
    ui.small(format!(
        "{} exposed inputs · {}",
        super::architecture::input_count(definition),
        match super::architecture::role(definition) {
            Some(symbols::architecture::ArchitectureRole::Wall) => "Path-driven wall",
            Some(symbols::architecture::ArchitectureRole::Fence) => "Path-driven fence",
            None if definition.plan.is_some() => "Linked 2D/3D",
            None => "3D",
        }
    ));
}

fn instance_label(document: &Document, id: EntityId) -> String {
    let instance = symbols::read_instance(document, id).map_or(id, |instance| instance.instance);
    entity_label(document, instance)
}
