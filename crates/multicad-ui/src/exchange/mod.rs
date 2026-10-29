//! Saved output sets and nonblocking, attributable export jobs.
mod backend;
mod form;
use crate::{egui, icons::Icon};
pub use backend::*;
use form::{Catalog, Draft};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityId,
    exchange::*,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportTool {
    NewSet,
    SaveSet,
    Discard,
    Preview,
    Run,
    Cancel,
}
impl ExportTool {
    pub const ALL: [Self; 6] = [
        Self::NewSet,
        Self::SaveSet,
        Self::Discard,
        Self::Preview,
        Self::Run,
        Self::Cancel,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::NewSet => "export-set",
            Self::SaveSet => "export-set-save",
            Self::Discard => "export-set-discard",
            Self::Preview => "export-preview",
            Self::Run => "export-run",
            Self::Cancel => "export-cancel",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NewSet => "New export set",
            Self::SaveSet => "Save set",
            Self::Discard => "Discard edits",
            Self::Preview => "Preview outputs",
            Self::Run => "Run export set",
            Self::Cancel => "Cancel jobs",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::NewSet => Icon::New,
            Self::SaveSet => Icon::Save,
            Self::Discard | Self::Cancel => Icon::Undo,
            Self::Preview => Icon::Library,
            Self::Run => Icon::Export,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSet => &["exportset"],
            Self::SaveSet => &["saveset"],
            Self::Discard => &["discardset"],
            Self::Preview => &["previewexport"],
            Self::Run => &["runexport"],
            Self::Cancel => &["cancelexport"],
        }
    }
}
struct Pending {
    document: EntityId,
    revision: u64,
    set: EntityId,
    actor: Actor,
    run: bool,
    expected: Option<PreviewKey>,
    catalog_scope: Option<Scope>,
}
struct JobSummary {
    id: EntityId,
    text: String,
    directory: Option<String>,
}
#[derive(Default)]
pub struct ExportWorkspace {
    backend: Option<Box<dyn ExportBackend>>,
    selected: Option<EntityId>,
    draft: Option<Draft>,
    original: Option<Draft>,
    preview: Option<ExportPreview>,
    pending: BTreeMap<EntityId, Pending>,
    history: VecDeque<JobSummary>,
    directory: String,
    error: Option<String>,
    notice: Option<(String, bool)>,
    observed: Option<(EntityId, u64, Option<EntityId>)>,
    scheduled: Option<f64>,
    captured: Vec<EntityId>,
    catalog: Catalog,
    catalog_key: Option<(EntityId, u64)>,
    planar_catalog: Option<PlanarCatalog>,
}
impl ExportWorkspace {
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
    pub fn set_backend(&mut self, backend: Box<dyn ExportBackend>) -> Result<(), String> {
        if self.pending() {
            return Err("Wait for accepted export jobs before replacing the output service".into());
        }
        self.backend = Some(backend);
        self.preview = None;
        self.observed = None;
        Ok(())
    }
    pub fn dirty(&self) -> bool {
        self.draft != self.original
    }
    pub fn writing(&self) -> bool {
        self.pending.values().any(|p| p.run)
    }
    pub fn pending(&self) -> bool {
        !self.pending.is_empty() || self.backend.as_ref().is_some_and(|b| b.pending())
    }
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take()
    }
    pub fn capture(&mut self, selected: Vec<EntityId>) {
        self.captured = selected;
    }
    fn load(&mut self, s: &Session, id: EntityId) -> Result<(), String> {
        let draft = Draft::load(s, id)?;
        self.selected = Some(id);
        self.draft = Some(draft.clone());
        self.original = Some(draft);
        self.error = None;
        self.preview = None;
        self.observed = None;
        Ok(())
    }
    pub fn new_set(&mut self, s: &Session) -> Result<(), String> {
        if self.dirty() {
            return Err("Save or discard the current export-set edits first".into());
        }
        self.selected = None;
        self.draft = Some(Draft::new(s));
        self.original = None;
        self.preview = None;
        self.error = None;
        self.scheduled = None;
        Ok(())
    }
    pub fn discard(&mut self, s: &Session) {
        if let Some(id) = self.selected {
            if let Err(e) = self.load(s, id) {
                self.error = Some(e);
                self.draft = None;
                self.original = None;
            }
        } else {
            self.draft = None;
            self.original = None;
        }
        self.scheduled = None;
    }
    pub fn save(&mut self, s: &mut Session, channel: Channel) -> Result<String, String> {
        let d = self
            .draft
            .as_mut()
            .ok_or("Create or select an export set")?;
        if d.document != s.document().id() {
            return Err("Export-set draft belongs to another document".into());
        }
        d.sync_query();
        let id = d.id;
        let before = s.revision();
        let prepared = prepare_set_with_geometry(
            s,
            d.revision,
            Actor::local(channel),
            id,
            &d.name,
            &d.source,
            &d.geometry,
        )
        .map_err(|e| e.to_string())?;
        s.commit(prepared).map_err(|e| e.to_string())?;
        self.load(s, id)?;
        Ok(if s.revision() == before {
            "No canonical changes; draft normalized"
        } else {
            "Export set saved · one Undo step"
        }
        .into())
    }
    fn request_preview(&mut self, s: &Session, channel: Channel) -> Result<String, String> {
        if self.dirty() {
            return Err("Save or discard export-set edits before resolving outputs".into());
        }
        let set = self.selected.ok_or("Select a saved export set")?;
        let actor = Actor::local(channel);
        let b = self
            .backend
            .as_mut()
            .ok_or("Native output service is not installed")?;
        let id = b
            .preview(s.document(), s.revision(), set, actor.clone())
            .map_err(|e| e.to_string())?;
        self.pending.insert(
            id,
            Pending {
                document: s.document().id(),
                revision: s.revision(),
                set,
                actor,
                run: false,
                expected: None,
                catalog_scope: None,
            },
        );
        self.scheduled = None;
        Ok("Resolving current output scope…".into())
    }
    pub fn request_planar_catalog(
        &mut self,
        s: &Session,
        channel: Channel,
    ) -> Result<String, String> {
        if self.pending() {
            return Err("Wait for the accepted output query before loading faces".into());
        }
        let draft = self
            .draft
            .as_ref()
            .ok_or("Create or choose an export set")?;
        let scope = draft.source.scope.clone();
        let set = draft.id;
        let actor = Actor::local(channel);
        let id = self
            .backend
            .as_mut()
            .ok_or("Native output service is not installed")?
            .planar_catalog(s.document(), s.revision(), &scope, actor.clone())
            .map_err(|e| e.to_string())?;
        self.pending.insert(
            id,
            Pending {
                document: s.document().id(),
                revision: s.revision(),
                set,
                actor,
                run: false,
                expected: None,
                catalog_scope: Some(scope),
            },
        );
        Ok("Loading current native planar faces…".into())
    }
    fn current(&self, s: &Session) -> bool {
        !self.dirty()
            && self.preview.as_ref().is_some_and(|p| {
                p.key.source.document == s.document().id()
                    && p.key.source.revision == s.revision()
                    && self.selected == Some(p.key.set)
            })
    }
    pub fn enabled(&self, tool: ExportTool, s: &Session) -> bool {
        match tool {
            ExportTool::NewSet => !self.dirty() && !self.pending(),
            ExportTool::SaveSet => self.dirty(),
            ExportTool::Discard => self.dirty(),
            ExportTool::Preview => {
                self.selected.is_some()
                    && !self.dirty()
                    && self.backend.is_some()
                    && !self.pending()
            }
            ExportTool::Run => {
                self.current(s)
                    && self.preview.as_ref().is_some_and(ExportPreview::can_run)
                    && !self.directory.trim().is_empty()
                    && !self.pending()
            }
            ExportTool::Cancel => self.pending(),
        }
    }
    pub fn execute(
        &mut self,
        s: &mut Session,
        tool: ExportTool,
        path: Option<&str>,
        channel: Channel,
    ) -> Result<String, String> {
        match tool {
            ExportTool::NewSet => {
                self.new_set(s)?;
                Ok("Define a reusable scope and output recipes".into())
            }
            ExportTool::SaveSet => self.save(s, channel),
            ExportTool::Discard => {
                self.discard(s);
                Ok("Export-set draft discarded".into())
            }
            ExportTool::Preview => self.request_preview(s, channel),
            ExportTool::Run => {
                if let Some(path) = path {
                    self.directory = path.into();
                }
                if !self.current(s) {
                    return Err("Preview the current saved set before running it".into());
                }
                let preview = self.preview.as_ref().expect("current preview");
                if !preview.can_run() {
                    return Err(
                        "Empty or unsupported recipes block Run; inspect the preview".into(),
                    );
                }
                preview
                    .key
                    .source
                    .check(s.document(), s.revision())
                    .map_err(|e| e.to_string())?;
                let actor = Actor::local(channel);
                let b = self
                    .backend
                    .as_mut()
                    .ok_or("Native output service is not installed")?;
                let id = b
                    .run(
                        s.document(),
                        s.revision(),
                        &preview.key,
                        Path::new(self.directory.trim()),
                        actor.clone(),
                    )
                    .map_err(|e| e.to_string())?;
                self.pending.insert(
                    id,
                    Pending {
                        document: s.document().id(),
                        revision: s.revision(),
                        set: preview.key.set,
                        actor,
                        run: true,
                        expected: Some(preview.key.clone()),
                        catalog_scope: None,
                    },
                );
                Ok(format!(
                    "Export queued for revision {} · source remains unchanged",
                    s.revision()
                ))
            }
            ExportTool::Cancel => {
                if let Some(b) = self.backend.as_mut() {
                    for id in self.pending.keys() {
                        b.cancel(*id);
                    }
                }
                self.scheduled = None;
                Ok("Cancellation requested; completed publications remain recorded".into())
            }
        }
    }
    fn remember(&mut self, id: EntityId, text: String, directory: Option<String>) {
        self.history.push_front(JobSummary {
            id,
            text,
            directory,
        });
        while self.history.len() > 32 {
            self.history.pop_back();
        }
    }
    pub fn poll(&mut self, s: &Session) {
        let mut replies = vec![];
        if let Some(b) = self.backend.as_mut() {
            while let Some(reply) = b.poll() {
                replies.push(reply);
            }
        }
        for reply in replies {
            let Some(p) = self.pending.remove(&reply.id) else {
                self.error = Some("Output service returned an unknown job identity".into());
                continue;
            };
            if reply.document != p.document
                || reply.revision != p.revision
                || reply.actor != p.actor
            {
                self.error=Some("Output result scope differs from the accepted request; inspect the output service".into());
                self.remember(
                    reply.id,
                    "Output result could not be attributed to its accepted request".into(),
                    None,
                );
                continue;
            }
            match reply.result {
                ExportResult::PlanarCatalog(result) if p.catalog_scope.is_some() => match result {
                    Ok(catalog)
                        if Some(&catalog.scope) == p.catalog_scope.as_ref()
                            && catalog.stamp.document == p.document
                            && catalog.stamp.revision == p.revision =>
                    {
                        if catalog.stamp.check(s.document(), s.revision()).is_ok()
                            && self
                                .draft
                                .as_ref()
                                .is_some_and(|d| d.id == p.set && d.source.scope == catalog.scope)
                        {
                            self.planar_catalog = Some(*catalog);
                            self.error = None;
                        }
                    }
                    Ok(_) => {
                        self.error = Some(
                            "Planar face catalog differs from its accepted source request".into(),
                        )
                    }
                    Err(e) => {
                        if e.kind != ErrorKind::Cancelled {
                            self.error = Some(e.to_string());
                        }
                    }
                },
                ExportResult::Preview(result) if !p.run && p.catalog_scope.is_none() => {
                    match result {
                        Ok(preview) => {
                            if preview.requested_by != p.actor
                                || preview.key.set != p.set
                                || preview.selection.set != p.set
                                || preview.selection.stamp != preview.key.source
                                || preview.key.source.document != p.document
                                || preview.key.source.revision != p.revision
                            {
                                self.error = Some(
                                    "Preview identity or source stamp differs from its request"
                                        .into(),
                                );
                                continue;
                            }
                            if s.document().id() == p.document
                                && s.revision() == p.revision
                                && self.selected == Some(p.set)
                                && !self.dirty()
                            {
                                match preview.key.source.check(s.document(), s.revision()) {
                                    Ok(()) => {
                                        self.observed =
                                            Some((s.document().id(), s.revision(), self.selected));
                                        self.scheduled = None;
                                        self.preview = Some(*preview);
                                        self.error = None;
                                    }
                                    Err(e) => self.error = Some(e.to_string()),
                                }
                            }
                        }
                        Err(e) => {
                            if e.kind != ErrorKind::Cancelled {
                                self.error = Some(e.to_string());
                                self.notice = Some((e.to_string(), true));
                            }
                        }
                    }
                }
                ExportResult::Run(result) if p.run => match result {
                    Ok(receipt) => {
                        if p.expected.as_ref() != Some(&receipt.key)
                            || receipt.key.set != p.set
                            || receipt.key.source.document != p.document
                            || receipt.key.source.revision != p.revision
                            || receipt.actor != p.actor
                        {
                            self.error=Some("Published output receipt differs from the accepted job; inspect its manifest".into());
                            continue;
                        }
                        let text = format!(
                            "{} {} file(s) · {} ({:?}) · revision {}",
                            if receipt.reused { "Reused" } else { "Exported" },
                            receipt.files.len(),
                            p.actor.id,
                            p.actor.channel,
                            p.revision
                        );
                        self.notice = Some((format!("{text} · {}", receipt.directory), false));
                        self.remember(reply.id, text, Some(receipt.directory));
                    }
                    Err(e) => {
                        let text = format!(
                            "Export failed · {} ({:?}) · revision {} · {e}",
                            p.actor.id, p.actor.channel, p.revision
                        );
                        self.notice = Some((text.clone(), true));
                        self.error = Some(e.to_string());
                        self.remember(reply.id, text, None);
                    }
                },
                _ => {
                    self.error = Some("Output reply has the wrong operation kind".into());
                }
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, s: &mut Session, allow_io: bool) {
        self.poll(s);
        let catalog_key = (s.document().id(), s.revision());
        if self.catalog_key != Some(catalog_key) {
            self.catalog = Catalog::new(s);
            self.catalog_key = Some(catalog_key);
        }
        let time = ui.ctx().input(|i| i.time);
        let key = (s.document().id(), s.revision(), self.selected);
        if self.observed != Some(key) {
            self.observed = Some(key);
            if !self.dirty()
                && let Some(id) = self.selected
            {
                let previous = self.preview.take();
                if let Err(error) = self.load(s, id) {
                    self.error = Some(error);
                    self.selected = None;
                    self.draft = None;
                    self.original = None;
                }
                if self.selected == Some(id) {
                    self.preview = previous;
                }
                self.observed = Some((s.document().id(), s.revision(), self.selected));
                if self.selected.is_some() {
                    self.scheduled = Some(time + 0.15);
                }
            }
            if let Some(b) = self.backend.as_mut() {
                for (id, p) in &self.pending {
                    if !p.run {
                        b.cancel(*id);
                    }
                }
            }
        }
        if allow_io
            && !self.pending()
            && !self.dirty()
            && self.selected.is_some()
            && self.backend.is_some()
            && self.scheduled.is_some_and(|at| time >= at)
            && let Err(error) = self.request_preview(s, Channel::Ui)
        {
            self.error = Some(error);
            self.scheduled = None;
        }
        if self.pending() || self.scheduled.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let mut action = None;
        let mut load_planar = false;
        egui::Panel::left("saved-export-sets")
            .default_size(205.)
            .resizable(true)
            .show(ui, |ui| {
                crate::controls::inspector_layout(ui);
                ui.heading("Export sets");
                if ui
                    .add_enabled(
                        self.enabled(ExportTool::NewSet, s),
                        egui::Button::new("New export set"),
                    )
                    .on_hover_text("Create a saved source scope and reusable recipes")
                    .clicked()
                {
                    action = Some(ExportTool::NewSet);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for id in sets(s.document()) {
                        let name = &s.document().get(id).expect("known set").name;
                        if ui
                            .add_enabled(
                                !self.dirty(),
                                egui::Button::selectable(self.selected == Some(id), name),
                            )
                            .clicked()
                            && let Err(e) = self.load(s, id)
                        {
                            self.error = Some(e);
                        }
                    }
                });
            });
        egui::Panel::right("export-set-inspector").default_size(325.).resizable(true).show(ui,|ui|{
   crate::controls::inspector_layout(ui);ui.heading("Export set");egui::ScrollArea::vertical().show(ui,|ui|{if let Some(draft)=&mut self.draft{load_planar=draft.show(ui,s,&self.captured,&self.catalog,self.planar_catalog.as_ref());}else{ui.label("Choose a saved set or create one.");}
    ui.horizontal_wrapped(|ui|{for tool in[ExportTool::SaveSet,ExportTool::Discard]{if ui.add_enabled(self.enabled(tool,s),egui::Button::new(tool.label())).on_hover_text("Edit the canonical export-set definition through one Undo step").clicked(){action=Some(tool);}}});
   });
  });
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(22,16)).show(ui,|ui|{
   ui.heading("Output preview");ui.label("Saved scopes resolve current source identities, independently of model selection and visibility.");
   ui.horizontal_wrapped(|ui|{for tool in[ExportTool::Preview,ExportTool::Run,ExportTool::Cancel]{if ui.add_enabled(self.enabled(tool,s)&&(allow_io||tool==ExportTool::Cancel),egui::Button::new(tool.label())).on_hover_text(match tool{ExportTool::Preview=>"Resolve the current saved scope against native geometry",ExportTool::Run=>"Export the exact previewed snapshot into a verified output bundle",_=>"Cancel accepted jobs at the next safe boundary"}).clicked(){action=Some(tool);}}});
   ui.label("Absolute output directory");ui.add(egui::TextEdit::singleline(&mut self.directory).desired_width(ui.available_width()).hint_text("Choose an existing absolute directory")).on_hover_text("The run creates a source- and provider-identified bundle. Unknown or modified bundles are never overwritten");
   if self.dirty(){ui.colored_label(egui::Color32::LIGHT_YELLOW,"Save or discard set edits before previewing or running.");}else if self.pending(){ui.label("Output job in progress…");}
   if let Some(error)=&self.error{ui.colored_label(egui::Color32::LIGHT_RED,error);}
   if self.backend.is_none(){ui.colored_label(egui::Color32::LIGHT_YELLOW,"Native output service is not installed.");}
   egui::ScrollArea::vertical().id_salt("export-results").show(ui,|ui|{
    if let Some(preview)=&self.preview{
     let current=self.current(s);if !current{ui.colored_label(egui::Color32::LIGHT_YELLOW,"Previous preview · display only. Resolve the current source before Run.");}
     ui.label(format!("{} matched source occurrence(s) · {} selected page(s) · revision {}",preview.selection.parts.len(),preview.selection.pages.len(),preview.key.source.revision));
     for report in &preview.recipes{let name=preview.selection.recipes.iter().find(|r|r.id==report.recipe).map_or("Recipe",|r|r.name.as_str());match &report.status{RecipeStatus::Ready=>{ui.label(format!("{name} · Ready · {} file(s)",report.files));},RecipeStatus::Empty=>{ui.colored_label(egui::Color32::LIGHT_YELLOW,format!("{name} · Empty"));},RecipeStatus::Unsupported(reason)=>{ui.colored_label(egui::Color32::LIGHT_YELLOW,format!("{name} · Unsupported · {reason}"));}}}
     ui.collapsing("Matched parts and occurrences",|ui|{for part in &preview.selection.parts {let path=if part.occurrence_names.is_empty(){part.name.clone()}else{format!("{} / {}",part.occurrence_names.join(" / "),part.name)};ui.label(path).on_hover_text(format!("Part {} · occurrence path {:?}",part.reference.part,part.reference.occurrence_path));}});
     ui.separator();for artifact in &preview.artifacts{let label=match &artifact.output.subject{OutputSubject::Part(reference)=>preview.selection.parts.iter().find(|p|p.reference==*reference).map(|p|if p.occurrence_names.is_empty(){p.name.clone()}else{format!("{} · {}",p.name,p.occurrence_names.join(" / "))}).unwrap_or_else(||"Part".into()),OutputSubject::Parts(parts)=>format!("Assembly · {} part(s)",parts.len()),_=>"Drawing".into()};ui.strong(label);if let Some(drawing)=&artifact.drawing{ui.label(format!("{} page(s) · {} vector item(s) · {} text line(s)",drawing.pages.len(),drawing.vectors,drawing.texts));}else if let Some(mesh)=&artifact.mesh{ui.label(format!("{} object(s) · {} placement(s) · {} triangles · chord {} mm",mesh.objects,mesh.build_items,mesh.triangles,mesh.chord_deviation_mm));}else if let Some(planar)=&artifact.planar{ui.label(format!("1:1 · {:.3} mm² · {} outer / {} inner loop(s)",planar.area_mm2,planar.outer_loops,planar.inner_loops));}else{ui.label(format!("{:.3} mm³ · {} solid(s)",artifact.volume_mm3,artifact.solid_count));}ui.add(egui::Label::new(&artifact.output.filename).truncate()).on_hover_text(&artifact.output.filename);}
     for excluded in &preview.excluded{let name=s.document().get(excluded.reference.part).map_or("Part",|e|e.name.as_str());ui.colored_label(egui::Color32::LIGHT_YELLOW,format!("Not exported: {name} · {}",match excluded.reason{ExclusionReason::ConsumedProducer=>"consumed by a later operation",ExclusionReason::NoCurrentSolid=>"no current unsuppressed solid"}));}
     if !preview.selection.pages.is_empty(){ui.separator();for(index,page)in preview.selection.pages.iter().enumerate(){ui.label(format!("Page {} · {} · {} · {} × {} mm",index+1,page.number,page.title,page.paper_mm[0],page.paper_mm[1]));}}
     ui.collapsing("Source identities",|ui|{ui.monospace(format!("Source {}",preview.key.source.source_hash));for part in &preview.selection.parts{ui.monospace(format!("Part {} · occurrences {:?}",part.reference.part,part.reference.occurrence_path));}});
    }else{ui.label("A current native preview lists every output, exclusion and unsupported recipe before Run.");}
    if !self.history.is_empty(){ui.separator();ui.collapsing("Output history",|ui|{for job in &self.history{ui.label(&job.text);if let Some(directory)=&job.directory{ui.monospace(directory);if ui.small_button("Copy output directory").clicked(){ui.ctx().copy_text(directory.clone());}}ui.small(format!("Job {}",job.id));}});}
   });
  });
        if load_planar && allow_io {
            match self.request_planar_catalog(s, Channel::Ui) {
                Ok(message) => {
                    self.notice = Some((message, false));
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(tool) = action {
            match self.execute(s, tool, None, Channel::Ui) {
                Ok(message) => {
                    self.notice = Some((message, false));
                    self.error = None;
                }
                Err(error) => {
                    self.notice = Some((error.clone(), true));
                    self.error = Some(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
