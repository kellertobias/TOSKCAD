use super::{
    SymbolAction, SymbolTool,
    fields::{self, entity_choice, entity_label},
    forms::{Form, Operation},
};
use crate::{controls, egui};
use multicad_core::{
    commands::Session,
    document::{Document, EntityData, EntityId},
    project_file::ContentHash,
    symbols::{self, SymbolDefinition, SymbolKind, SymbolPlan},
};
use std::collections::{BTreeMap, BTreeSet};
enum Next {
    Place(EntityId),
    Import(Box<SymbolDefinition>),
    Update(EntityId),
    Replacement(Box<SymbolDefinition>),
    Edit(EntityId),
    AuthorConstruction(EntityId),
}
pub(super) struct Catalogue<'a> {
    pub embedded: &'a [(EntityId, SymbolDefinition)],
    pub embedded_pins: &'a BTreeMap<EntityId, ContentHash>,
    pub embedded_search: &'a BTreeMap<EntityId, String>,
    pub library: &'a [SymbolDefinition],
    pub library_pins: &'a [ContentHash],
    pub library_search: &'a [String],
    pub installed: &'a mut super::installed::Installed,
}
pub(super) fn show(
    form: &mut Form,
    ui: &mut egui::Ui,
    session: &Session,
    catalogue: Catalogue<'_>,
    enabled: bool,
) -> (Option<SymbolAction>, Option<String>) {
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
        Operation::Library=>{
            changed|=fields::text(ui,"Filter symbols",&mut form.filter,"Search listed files, embedded and loaded definitions");
            action=catalogue.installed.show(ui,&form.filter);
            ui.separator();
            if ui.button("Open library file…").on_hover_text("Load definitions without changing this project. Embedded pins continue working when the file is unavailable").clicked(){action=Some(SymbolAction::ImportLibrary);}
            ui.strong("Embedded in this project");
            let filter=form.filter.to_lowercase();
            let mut shown=0;
            for (id,definition) in definitions {
                if !super::installed::contains_terms(&catalogue.embedded_search[id], &filter){continue;}
                shown+=1;
                ui.group(|ui|{
                    summary(ui,definition);
                    if let Some(role)=super::architecture::role(definition) {
                        let label=match role { symbols::architecture::ArchitectureRole::Wall=>"Draw wall", symbols::architecture::ArchitectureRole::Fence=>"Draw fence" };
                        if ui.button(label).clicked(){action=Some(SymbolAction::DrawConstruction{definition:*id,role});}
                    } else if ui.button("Place").on_hover_text("Place this pinned definition; dynamic symbols ask only for exposed values").clicked(){next=Some(Next::Place(*id));}
                    ui.horizontal_wrapped(|ui|{
                        if ui.small_button("Save definition…").clicked(){action=Some(SymbolAction::SaveDefinition(*id));}
                        if ui.small_button("Compare update").on_hover_text("Choose a revision explicitly; this never upgrades instances automatically").clicked(){next=Some(Next::Update(*id));}
                    });
                });
            }
            if shown==0{ui.label("No matching embedded symbols");}
            ui.separator();ui.strong("Loaded library · import before placing");
            let mut shown=0;
            for ((definition,pin),search) in library.iter().zip(catalogue.library_pins).zip(catalogue.library_search) {
                if !super::installed::contains_terms(search, &filter){continue;}
                shown+=1;
                ui.group(|ui|{
                    summary(ui,definition);
                    let embedded=definitions.iter().any(|(id,def)|def.symbol==definition.symbol&&catalogue.embedded_pins.get(id)==Some(pin));
                    if embedded {ui.small("This exact revision is embedded");}else if ui.button("Import definition").on_hover_text("Preview and explicitly embed this immutable definition before placing it").clicked(){next=Some(Next::Import(Box::new(definition.clone())));}
                });
            }
            if shown==0{ui.label("No matching library symbols loaded");}
        }
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
                    if let Some(value)=evaluated.as_ref().and_then(|values|values.values.get(&entity.id).and_then(|v|values.value(entity.id,v.unit).ok().map(|n|format!("{n} {}",v.unit.symbol())))){ui.small(value);}
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
        Operation::Place{definition,name,values,pose,..}=>{
            let entries=definitions.iter().filter(|(_,def)|super::architecture::role(def).is_none()).map(|(id,def)|(*id,format!("{} · revision {}",def.name,def.revision))).collect::<Vec<_>>();
            if entity_choice(ui,"symbol-place-definition","Embedded definition",definition,&entries,false)&&let Some(id)=definition{next=Some(Next::Place(*id));}
            if let Some((_,source))=definitions.iter().find(|(id,_)|Some(*id)==*definition){
                summary(ui,source);
                changed|=fields::text(ui,"Instance name",name,"Name for this occurrence; the source definition and other instances are unchanged");
                changed|=values.show(ui,source);
                changed|=pose.show(ui);
            }else{ui.small("Choose an embedded definition. Import a library file through Library first.");}
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
                for (heading,ids) in [("Added inputs",&report.added_parameters),("Removed inputs",&report.removed_parameters),("Changed inputs/defaults",&report.changed_parameters)] {
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
                    ui.group(|ui|{ui.strong(instance_label(document,*record));changed|=fields.show(ui,replacement);});
                }
                for change in &report.architecture_changes {
                    ui.group(|ui| {
                        ui.strong(instance_label(document,change.output.anchors()[1]));
                        for layer in &change.removed_layers { ui.label(format!("Removed layer: {}",layer.name)); }
                        for recipe in &change.changed_recipes { ui.label(format!("Replaced infill: {}",recipe.name)); }
                        if !change.removed_members.is_empty() { ui.label(format!("{} physical members will be removed",change.removed_members.len())); }
                        if let Some(fields)=architecture_values.get_mut(&change.record) { changed|=fields.show(ui); }
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
        let result = match next {
            Next::Place(id) => form.place(document, id),
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
        };
        if let Err(failure) = result {
            error = Some(failure);
        }
    }
    (action, error)
}
fn summary(ui: &mut egui::Ui, definition: &SymbolDefinition) {
    ui.strong(&definition.name);
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
        definition.exposed.len(),
        if definition.plan.is_some() {
            "Linked 2D/3D"
        } else {
            "3D"
        }
    ));
}

fn instance_label(document: &Document, id: EntityId) -> String {
    let instance = symbols::read_instance(document, id).map_or(id, |instance| instance.instance);
    entity_label(document, instance)
}
