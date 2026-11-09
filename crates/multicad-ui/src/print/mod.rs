//! Export → Print: choose finished Parts, include or leave out their
//! removable FDM aids, review the process records and findings, preview the
//! package contents and write a 3MF print package (optionally one STL per
//! Part) through the native save dialog. The native provider runs off the UI
//! thread; this workspace owns no CAD handles. It never slices, controls a
//! printer or asserts that a package prints.
use crate::{egui, icons::Icon};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::*,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Installed by the native composition root.
pub trait PrintBackend {
    fn preview(
        &mut self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: Actor,
    ) -> Result<EntityId, ExportError>;
    fn write(
        &mut self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError>;
    fn cancel(&mut self, id: EntityId) -> bool;
    fn poll(&mut self) -> Option<PrintReply>;
    fn pending(&self) -> bool;
}
/// The document and revision a cached listing answers.
type SourceKey = (EntityId, u64);
pub type PrintBackendFactory = Arc<dyn Fn() -> Box<dyn PrintBackend> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintTool {
    Preview,
    Write,
    Cancel,
}
impl PrintTool {
    pub const ALL: [Self; 3] = [Self::Preview, Self::Write, Self::Cancel];
    pub fn id(self) -> &'static str {
        match self {
            Self::Preview => "print-preview",
            Self::Write => "print-write",
            Self::Cancel => "print-cancel",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preview package",
            Self::Write => "Write 3MF package",
            Self::Cancel => "Cancel print job",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Preview => Icon::Library,
            Self::Write => Icon::Export,
            Self::Cancel => Icon::Undo,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Preview => &["printpreview", "previewprint"],
            Self::Write => &["print3mf", "write3mf"],
            Self::Cancel => &["cancelprint"],
        }
    }
}
struct Pending {
    document: EntityId,
    revision: u64,
    actor: Actor,
    request: PrintRequest,
    expected: Option<PrintKey>,
    path: Option<PathBuf>,
}
pub struct PrintWorkspace {
    backend: Option<Box<dyn PrintBackend>>,
    /// The explicit Parts and placed copies to package, chosen in the
    /// picker or captured on entering Export.
    captured: Vec<EntityId>,
    /// The model selection captured on entering Export, restorable after
    /// picking.
    entered: Vec<EntityId>,
    /// Picker entries for the document revision they were listed from.
    candidates: Option<(SourceKey, Result<Vec<PrintCandidate>, String>)>,
    by_tag: bool,
    tag: String,
    aids: AidPolicy,
    chord_mm: f64,
    stl_per_part: bool,
    preview: Option<PrintPreview>,
    pending: BTreeMap<EntityId, Pending>,
    /// The source and request the latest preview was asked for.
    requested: Option<(EntityId, u64, PrintRequest)>,
    scheduled: Option<f64>,
    history: VecDeque<String>,
    error: Option<String>,
    notice: Option<(String, bool)>,
    write_requested: bool,
}
impl Default for PrintWorkspace {
    fn default() -> Self {
        Self {
            backend: None,
            captured: vec![],
            entered: vec![],
            candidates: None,
            by_tag: false,
            tag: DEFAULT_PRINT_TAG.into(),
            aids: AidPolicy::IncludeAll,
            chord_mm: MeshQuality::default().chord_deviation_mm,
            stl_per_part: false,
            preview: None,
            pending: BTreeMap::new(),
            requested: None,
            scheduled: None,
            history: VecDeque::new(),
            error: None,
            notice: None,
            write_requested: false,
        }
    }
}
impl PrintWorkspace {
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
    pub fn set_backend(&mut self, backend: Box<dyn PrintBackend>) -> Result<(), String> {
        if self.pending() {
            return Err("Wait for accepted print jobs before replacing the print service".into());
        }
        self.backend = Some(backend);
        self.preview = None;
        self.requested = None;
        Ok(())
    }
    pub fn pending(&self) -> bool {
        !self.pending.is_empty() || self.backend.as_ref().is_some_and(|b| b.pending())
    }
    pub fn writing(&self) -> bool {
        self.pending.values().any(|p| p.path.is_some())
    }
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take()
    }
    pub fn take_write_request(&mut self) -> bool {
        std::mem::take(&mut self.write_requested)
    }
    /// The selection captured on entering Export; it chooses Parts until
    /// the next capture, independently of later viewport state.
    pub fn capture(&mut self, selected: Vec<EntityId>) {
        self.entered = selected.clone();
        self.captured = selected;
    }
    /// Choose the explicit Parts and placed copies to package, in place of
    /// the captured selection. Only this workspace changes; the document
    /// and the model selection stay as they are.
    pub fn choose(&mut self, parts: Vec<EntityId>) {
        self.by_tag = false;
        self.captured = parts;
    }
    pub fn chosen(&self) -> &[EntityId] {
        &self.captured
    }
    /// Return to the selection captured on entering Export.
    pub fn restore_captured(&mut self) {
        self.captured = self.entered.clone();
    }
    /// The picker's entries for the current revision.
    pub fn candidates(&mut self, s: &Session) -> Result<&[PrintCandidate], String> {
        let key = (s.document().id(), s.revision());
        if self.candidates.as_ref().is_none_or(|(k, _)| *k != key) {
            self.candidates = Some((
                key,
                print_candidates(s.document()).map_err(|e| e.to_string()),
            ));
        }
        match &self.candidates.as_ref().expect("listed").1 {
            Ok(list) => Ok(list),
            Err(error) => Err(error.clone()),
        }
    }
    pub fn set_source_tag(&mut self, tag: Option<&str>) {
        self.by_tag = tag.is_some();
        if let Some(tag) = tag {
            self.tag = tag.into();
        }
    }
    pub fn set_aids(&mut self, aids: AidPolicy) {
        self.aids = aids;
    }
    pub fn set_stl_per_part(&mut self, on: bool) {
        self.stl_per_part = on;
    }
    pub fn preview(&self) -> Option<&PrintPreview> {
        self.preview.as_ref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn request(&self) -> PrintRequest {
        PrintRequest {
            source: if self.by_tag {
                PrintSource::Tag {
                    tag: self.tag.trim().into(),
                }
            } else {
                PrintSource::Parts {
                    parts: self.captured.clone(),
                }
            },
            aids: self.aids,
            quality: MeshQuality {
                chord_deviation_mm: self.chord_mm,
                ..MeshQuality::default()
            },
            stl_per_part: self.stl_per_part,
        }
    }
    /// The preview answers the current document revision and settings.
    pub fn current(&self, s: &Session) -> bool {
        self.preview.as_ref().is_some_and(|p| {
            p.key.source.document == s.document().id()
                && p.key.source.revision == s.revision()
                && p.request == self.request()
        })
    }
    pub fn enabled(&self, tool: PrintTool, s: &Session) -> bool {
        match tool {
            PrintTool::Preview => self.backend.is_some() && !self.pending(),
            PrintTool::Write => {
                self.backend.is_some()
                    && !self.pending()
                    && self.current(s)
                    && self.preview.as_ref().is_some_and(PrintPreview::can_write)
            }
            PrintTool::Cancel => self.pending(),
        }
    }
    pub fn request_preview(&mut self, s: &Session, channel: Channel) -> Result<String, String> {
        if self.pending() {
            return Err("Wait for the accepted print job first".into());
        }
        let request = self.request();
        request.validate().map_err(|e| e.to_string())?;
        let actor = Actor::local(channel);
        let id = self
            .backend
            .as_mut()
            .ok_or("Native print service is not installed")?
            .preview(s.document(), s.revision(), &request, actor.clone())
            .map_err(|e| e.to_string())?;
        self.requested = Some((s.document().id(), s.revision(), request.clone()));
        self.scheduled = None;
        self.error = None;
        self.pending.insert(
            id,
            Pending {
                document: s.document().id(),
                revision: s.revision(),
                actor,
                request,
                expected: None,
                path: None,
            },
        );
        Ok("Resolving the print package from current exact solids…".into())
    }
    /// Checks before the native save dialog opens; returns the suggested
    /// file name.
    pub fn begin_write(&self, s: &Session) -> Result<String, String> {
        if self.pending() {
            return Err("Wait for the accepted print job first".into());
        }
        let preview = self
            .preview
            .as_ref()
            .filter(|_| self.current(s))
            .ok_or("Preview the current print package before writing it")?;
        if let Some(reason) = &preview.blocked {
            return Err(reason.clone());
        }
        preview
            .key
            .source
            .check(s.document(), s.revision())
            .map_err(|e| e.to_string())?;
        let name: String = s
            .document()
            .name()
            .chars()
            .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
            .take(120)
            .collect();
        Ok(format!(
            "{}.3mf",
            if name.trim().is_empty() {
                "Print package"
            } else {
                name.trim()
            }
        ))
    }
    /// Queue the write of the current preview to the file chosen in the
    /// native save dialog. The provider rebuilds and rechecks the source.
    pub fn write(&mut self, s: &Session, path: &Path, channel: Channel) -> Result<String, String> {
        self.begin_write(s)?;
        let mut path = path.to_owned();
        if path.extension().is_none() {
            path.set_extension("3mf");
        }
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("3mf"))
        {
            return Err("A print package file must end in .3mf".into());
        }
        let preview = self.preview.as_ref().expect("checked current preview");
        let request = preview.request.clone();
        let expected = preview.key.clone();
        let actor = Actor::local(channel);
        let id = self
            .backend
            .as_mut()
            .ok_or("Native print service is not installed")?
            .write(
                s.document(),
                s.revision(),
                &request,
                &expected,
                &path,
                actor.clone(),
            )
            .map_err(|e| e.to_string())?;
        self.pending.insert(
            id,
            Pending {
                document: s.document().id(),
                revision: s.revision(),
                actor,
                request,
                expected: Some(expected),
                path: Some(path.clone()),
            },
        );
        Ok(format!(
            "Writing print package for revision {} · {}",
            s.revision(),
            path.display()
        ))
    }
    pub fn cancel(&mut self) -> String {
        if let Some(b) = self.backend.as_mut() {
            for id in self.pending.keys() {
                b.cancel(*id);
            }
        }
        self.scheduled = None;
        "Cancellation requested; files are replaced only after every readback passes".into()
    }
    fn remember(&mut self, text: String) {
        self.history.push_front(text);
        self.history.truncate(16);
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
                self.error = Some("Print service returned an unknown job identity".into());
                continue;
            };
            if reply.document != p.document
                || reply.revision != p.revision
                || reply.actor != p.actor
            {
                self.error =
                    Some("Print result differs from its accepted request; nothing is shown".into());
                continue;
            }
            match (reply.result, &p.path) {
                (PrintResult::Preview(Ok(preview)), None) => {
                    if preview.requested_by != p.actor
                        || preview.request != p.request
                        || preview.key.source.document != p.document
                        || preview.key.source.revision != p.revision
                    {
                        self.error = Some("Print preview identity differs from its request".into());
                        continue;
                    }
                    // A reply for an older revision or setting stays unused.
                    if s.document().id() == p.document
                        && s.revision() == p.revision
                        && p.request == self.request()
                    {
                        self.notice = Some(match &preview.blocked {
                            Some(reason) => (format!("Print preview blocked · {reason}"), true),
                            None if !preview.can_write() => {
                                ("No printable parts in the preview".into(), true)
                            }
                            None => ("Print preview ready".into(), false),
                        });
                        self.preview = Some(*preview);
                        self.error = None;
                    }
                }
                (PrintResult::Preview(Err(e)), None) => {
                    if s.document().id() == p.document
                        && s.revision() == p.revision
                        && p.request == self.request()
                    {
                        self.preview = None;
                        if e.kind == ErrorKind::Cancelled {
                            self.notice = Some(("Print preview cancelled".into(), false));
                        } else {
                            let reason = e.to_string();
                            self.notice = Some((format!("Print preview failed · {reason}"), true));
                            self.error = Some(reason);
                        }
                    }
                }
                (PrintResult::Write(Ok(receipt)), Some(path)) => {
                    if p.expected.as_ref() != Some(&receipt.key) || receipt.actor != p.actor {
                        self.error = Some(
                            "Print receipt differs from the accepted job; inspect the written files"
                                .into(),
                        );
                        continue;
                    }
                    let text = format!(
                        "Wrote {} file(s) · revision {} · {}",
                        receipt.files.len(),
                        p.revision,
                        path.display()
                    );
                    self.notice = Some((text.clone(), false));
                    self.error = None;
                    let mut entry = text;
                    for file in &receipt.files {
                        entry.push_str(&format!(
                            "\n  {} · {} triangles · {:.3} mm³",
                            file.path, file.triangles, file.mesh_volume_mm3
                        ));
                    }
                    self.remember(entry);
                }
                (PrintResult::Write(Err(e)), Some(path)) => {
                    let text = format!("Print package not written · {} · {e}", path.display());
                    self.notice = Some((text.clone(), e.kind != ErrorKind::Cancelled));
                    self.error = Some(e.to_string());
                    self.remember(text);
                }
                _ => self.error = Some("Print reply has the wrong operation kind".into()),
            }
        }
    }
    /// Keep the preview answering the current revision and settings: a
    /// changed source or setting re-resolves after a short pause.
    fn refresh(&mut self, s: &Session, time: f64, allow_io: bool) {
        let wanted = (s.document().id(), s.revision(), self.request());
        if self.requested.as_ref() != Some(&wanted) {
            if self.scheduled.is_none() {
                self.scheduled = Some(time + 0.25);
            }
            if let Some(b) = self.backend.as_mut() {
                for (id, p) in &self.pending {
                    if p.path.is_none() {
                        b.cancel(*id);
                    }
                }
            }
        }
        if allow_io
            && self.backend.is_some()
            && !self.pending()
            && self.scheduled.is_some_and(|at| time >= at)
            && wanted.2.validate().is_ok()
            && let Err(error) = self.request_preview(s, Channel::Ui)
        {
            self.error = Some(error);
            self.scheduled = None;
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, s: &Session, allow_io: bool) {
        self.poll(s);
        let time = ui.ctx().input(|i| i.time);
        self.refresh(s, time, allow_io);
        if self.pending() || self.scheduled.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let mut action = None;
        let source_max = (ui.available_width() * 0.4).max(0.);
        egui::Panel::left("print-source")
            .default_size(260.)
            .size_range(180_f32.min(source_max)..=source_max)
            .resizable(true)
            .frame(egui::Frame::new().fill(crate::theme::PANEL).inner_margin(12))
            .show(ui, |ui| {
                crate::controls::inspector_layout(ui);
                ui.heading("Printable Parts");
                egui::ScrollArea::vertical()
                    .id_salt("print-source-settings")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.radio_value(&mut self.by_tag, false, "Chosen Parts")
                            .on_hover_text(
                                "Choose world Parts, placed copies and placed Assemblies here; each copy is packaged at its placement. Entering Export starts from the model selection",
                            );
                        if !self.by_tag {
                            self.source_picker(ui, s);
                        }
                        ui.radio_value(&mut self.by_tag, true, "Tag")
                            .on_hover_text(
                                "Every world Part and every placed copy of a Part carrying this exact, case-sensitive tag",
                            );
                        if self.by_tag {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.tag)
                                    .desired_width(ui.available_width())
                                    .hint_text(DEFAULT_PRINT_TAG),
                            );
                        }
                        ui.separator();
                        let mut include = self.aids == AidPolicy::IncludeAll;
                        if ui
                            .checkbox(&mut include, "Include removable FDM aids")
                            .on_hover_text(
                                "Tabs, bridges and supports serving the chosen Parts, classified by their aid feature",
                            )
                            .changed()
                        {
                            self.aids = if include {
                                AidPolicy::IncludeAll
                            } else {
                                AidPolicy::ExcludeRemovable
                            };
                        }
                        ui.checkbox(&mut self.stl_per_part, "Also write one STL per Part")
                            .on_hover_text("Binary STL files beside the 3MF, named after the 3MF and the Part");
                        ui.add(
                            egui::Slider::new(&mut self.chord_mm, 0.01..=1.0)
                                .logarithmic(true)
                                .text("chord mm"),
                        )
                        .on_hover_text("Tessellation chord deviation from the exact solid");
                        ui.small("Units: millimeter, declared in the 3MF. +Z is the build direction; model coordinates are kept.");
                    });
            });
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(22, 16))
            .show(ui, |ui| {
                crate::controls::inspector_layout(ui);
                ui.heading("Print package");
                ui.label("Packages exact geometry only. MultiCAD does not slice, arrange plates, control printers or confirm printability.");
                ui.horizontal_wrapped(|ui| {
                    for tool in PrintTool::ALL {
                        if ui
                            .add_enabled(
                                self.enabled(tool, s) && (allow_io || tool == PrintTool::Cancel),
                                egui::Button::new(tool.label()),
                            )
                            .clicked()
                        {
                            action = Some(tool);
                        }
                    }
                });
                egui::ScrollArea::vertical().id_salt("print-preview").show(ui, |ui| {
                    if self.backend.is_none() {
                        ui.colored_label(egui::Color32::LIGHT_YELLOW, "Native print service is not installed.");
                    }
                    if self.pending() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(if self.writing() { "Writing print package…" } else { "Resolving print package…" });
                        });
                    }
                    if let Some(error) = &self.error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                    if let Some(preview) = &self.preview {
                        self.show_preview(ui, s, preview);
                    } else {
                        ui.label("The preview lists every packaged Part, aid, process record and finding before anything is written.");
                    }
                    if !self.history.is_empty() {
                        ui.separator();
                        ui.collapsing("Written packages", |ui| {
                            for entry in &self.history {
                                ui.monospace(entry);
                            }
                        });
                    }
                });
            });
        if let Some(tool) = action {
            match tool {
                PrintTool::Preview => match self.request_preview(s, Channel::Ui) {
                    Ok(message) => self.notice = Some((message, false)),
                    Err(error) => self.error = Some(error),
                },
                PrintTool::Write => self.write_requested = true,
                PrintTool::Cancel => self.notice = Some((self.cancel(), false)),
            }
        }
    }
    /// The explicit source list: every Part, placed copy and placed
    /// Assembly the package can hold, with the chosen ones ticked. Changing
    /// it makes the current preview display-only until it re-resolves.
    fn source_picker(&mut self, ui: &mut egui::Ui, s: &Session) {
        let candidates = match self.candidates(s) {
            Ok(list) => list.to_vec(),
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
                return;
            }
        };
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button("All")
                .on_hover_text("Choose every Part, placed copy and placed Assembly")
                .clicked()
            {
                self.captured = candidates.iter().map(|c| c.entity).collect();
            }
            if ui
                .small_button("None")
                .on_hover_text("Clear the chosen list")
                .clicked()
            {
                self.captured.clear();
            }
            if ui
                .add_enabled(
                    self.captured != self.entered,
                    egui::Button::new("Model selection").small(),
                )
                .on_hover_text("Return to the selection captured when Export was entered")
                .clicked()
            {
                self.restore_captured();
            }
        });
        ui.small(format!(
            "{} of {} chosen",
            self.captured
                .iter()
                .filter(|id| candidates.iter().any(|c| c.entity == **id))
                .count(),
            candidates.len()
        ));
        if candidates.is_empty() {
            ui.small("This project has no Part, placed copy or placed Assembly yet.");
        }
        for c in &candidates {
            let mut on = self.captured.contains(&c.entity);
            let kind = match (c.kind, &c.source) {
                (PrintCandidateKind::Part, _) => "Part".to_owned(),
                (PrintCandidateKind::PlacedCopy, Some(source)) => {
                    format!("placed copy of {source}")
                }
                (PrintCandidateKind::PlacedCopy, None) => "placed copy".to_owned(),
                (PrintCandidateKind::Assembly, _) => "placed Assembly".to_owned(),
            };
            if ui
                .checkbox(&mut on, format!("{} · {kind}", c.name))
                .on_hover_text(format!("Identity {}", c.entity))
                .changed()
            {
                if on {
                    self.captured.push(c.entity);
                } else {
                    self.captured.retain(|id| *id != c.entity);
                }
            }
        }
        // Chosen identities the picker does not offer (component definitions,
        // aids or deleted entities) stay explicit; Preview lists their reason.
        for id in self
            .captured
            .iter()
            .filter(|id| !candidates.iter().any(|c| c.entity == **id))
            .take(12)
        {
            ui.small(format!(
                "Also chosen: {}",
                s.document()
                    .get(*id)
                    .map_or_else(|| format!("missing {id}"), |e| e.name.clone())
            ));
        }
    }
    fn show_preview(&self, ui: &mut egui::Ui, s: &Session, preview: &PrintPreview) {
        if !self.current(s) {
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                "Previous preview · display only. The source or settings changed.",
            );
        }
        if let Some(reason) = &preview.blocked {
            ui.colored_label(egui::Color32::LIGHT_YELLOW, reason);
        }
        let total: f64 = preview.objects.iter().map(|o| o.volume_mm3).sum();
        let placed = preview
            .objects
            .iter()
            .filter(|o| !o.occurrence_path.is_empty())
            .count();
        ui.label(format!(
            "{} object(s), {placed} of them placed occurrence(s) · {:.1} mm³ exact volume · revision {}",
            preview.objects.len(),
            total,
            preview.key.source.revision
        ));
        let column_width =
            (crate::controls::inspector_width(ui) - 4. * ui.spacing().item_spacing.x) / 5.;
        egui::Grid::new("print-objects")
            .striped(true)
            .num_columns(5)
            .min_col_width(0.)
            .max_col_width(column_width.max(1.))
            .show(ui, |ui| {
                for h in ["Object", "Role", "Z range mm", "Volume mm³", "Triangles"] {
                    ui.strong(h);
                }
                ui.end_row();
                for o in &preview.objects {
                    ui.label(&o.name)
                        .on_hover_text(if o.occurrence_path.is_empty() {
                            "World Part"
                        } else {
                            "Placed occurrence, packaged at its world placement"
                        });
                    ui.label(match &o.role {
                        PrintRole::Finished => "Finished".to_string(),
                        PrintRole::Aid { target, .. } => format!(
                            "Aid for {}",
                            s.document()
                                .get(*target)
                                .map_or("Part", |e| e.name.as_str())
                        ),
                    });
                    ui.label(format!(
                        "{:.3} – {:.3}",
                        o.bounds_mm[0][2], o.bounds_mm[1][2]
                    ));
                    ui.label(format!("{:.3}", o.volume_mm3));
                    ui.label(o.triangles.to_string());
                    ui.end_row();
                }
            });
        if !preview.files.is_empty() {
            ui.separator();
            ui.strong("Files");
            for file in &preview.files {
                ui.label(format!(
                    "{} · {} mesh object(s) · {} build item(s) · {} triangles",
                    file.filename("<name>"),
                    file.objects,
                    file.items,
                    file.triangles
                ));
            }
            ui.small("<name> is the file chosen in the save dialog; existing files with these names are replaced after readback.");
        }
        for skip in &preview.selection.skipped {
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                format!("Not packaged: {} · {}", skip.name, skip.reason),
            );
        }
        ui.separator();
        ui.strong("FDM process records");
        if preview.processes.is_empty() {
            ui.small("No aid serving these Parts names a process record.");
        }
        for p in &preview.processes {
            ui.label(format!(
                "{} · layer {:.3} mm · line {:.3} mm · ≥ {} layers · ≥ {} lines · support gap {:.3} mm · fit clearance {:.3} mm",
                p.name,
                p.layer_height_mm,
                p.line_width_mm,
                p.min_layers,
                p.min_lines,
                p.support_gap_mm,
                p.fit_clearance_mm
            ));
        }
        ui.separator();
        ui.strong("Findings");
        if preview.findings.is_empty() {
            ui.small("No findings. This is not a printability check.");
        }
        for f in &preview.findings {
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                format!("{} · {}", f.subject, f.message),
            );
        }
    }
}

#[cfg(test)]
mod tests;
