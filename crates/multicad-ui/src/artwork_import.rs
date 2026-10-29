//! DXF/SVG import Inspector. The chosen file is read once on a bounded
//! worker; the preview and Apply both derive from those exact bytes. Cancel or
//! a failed Apply drops the plan and leaves the document unchanged.
use crate::{
    egui,
    inspector_actions::{self, Button, Owner},
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::source::Source,
    document::EntityId,
    references::ReferenceReport,
    sketch::Datum,
    sketch_import::{
        self, Artwork, Format, ImportOptions, ImportPlan, ImportPreview, SourceUnit, UnitChoice,
    },
};
use multicad_solver::import::{ImportSource, ImportTarget, prepare_import};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Open(EntityId),
    New(Datum),
}

#[derive(Debug, PartialEq)]
pub enum ImportOutcome {
    Applied {
        sketch: EntityId,
        curves: Vec<EntityId>,
    },
    Cancelled,
}

#[derive(Debug)]
struct Loaded {
    file_name: String,
    format: Format,
    bytes: Vec<u8>,
}

struct Ready {
    file: Loaded,
    artwork: Result<Artwork, String>,
    options: ImportOptions,
    target: Target,
    /// The sketch that was open when the file was chosen, if any.
    open: Option<EntityId>,
    /// Recomputed whenever options change; never authorizes Apply by itself.
    plan: Result<ImportPlan, String>,
    preview: Option<ImportPreview>,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Reading(Receiver<Result<Loaded, String>>),
    Ready(Box<Ready>),
}

#[derive(Default)]
pub struct ImportController {
    state: State,
    error: Option<String>,
    default_target: Option<Target>,
}

/// Reads at most the import limit plus one byte so oversize files fail
/// without being loaded completely.
fn read_bounded(path: &Path) -> Result<Loaded, String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("The selected file has no readable name")?
        .to_string();
    let format = Format::from_file_name(&file_name).ok_or("Choose a .dxf or .svg file")?;
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open {file_name}: {e}"))?;
    let mut bytes = Vec::new();
    file.take(sketch_import::MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read {file_name}: {e}"))?;
    if bytes.len() > sketch_import::MAX_FILE_BYTES {
        return Err(format!(
            "{file_name} exceeds the {} MiB import limit",
            sketch_import::MAX_FILE_BYTES / (1024 * 1024)
        ));
    }
    Ok(Loaded {
        file_name,
        format,
        bytes,
    })
}

impl ImportController {
    pub fn active(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
    pub fn pending(&self) -> bool {
        matches!(self.state, State::Reading(_))
    }
    pub fn cancel(&mut self) {
        self.state = State::Idle;
        self.error = None;
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Starts reading a picked file. `target` is the open sketch, if any.
    pub fn begin(&mut self, path: PathBuf, target: Option<EntityId>) {
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("multicad-artwork-read".into())
            .spawn(move || {
                let _ = tx.send(read_bounded(&path));
            });
        self.error = None;
        self.default_target = Some(target.map_or(Target::New(Datum::Xy), Target::Open));
        match spawned {
            Ok(_) => self.state = State::Reading(rx),
            Err(e) => {
                self.state = State::Idle;
                self.error = Some(e.to_string());
            }
        }
    }
    /// Test and external-control seam that bypasses the file worker.
    pub fn begin_bytes(
        &mut self,
        file_name: &str,
        bytes: Vec<u8>,
        target: Option<EntityId>,
    ) -> Result<(), String> {
        let format = Format::from_file_name(file_name).ok_or("Choose a .dxf or .svg file")?;
        self.default_target = Some(target.map_or(Target::New(Datum::Xy), Target::Open));
        self.accept(Loaded {
            file_name: file_name.into(),
            format,
            bytes,
        });
        Ok(())
    }
    pub fn poll(&mut self) {
        let State::Reading(rx) = &self.state else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(loaded)) => self.accept(loaded),
            Ok(Err(error)) => {
                self.state = State::Idle;
                self.error = Some(error);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.state = State::Idle;
                self.error = Some("The file reader stopped without a result".into());
            }
        }
    }
    fn accept(&mut self, file: Loaded) {
        let options = ImportOptions::default();
        let artwork =
            sketch_import::parse(file.format, &file.bytes, options.units).map_err(|e| e.0);
        let mut ready = Ready {
            file,
            artwork,
            options,
            target: self.default_target.unwrap_or(Target::New(Datum::Xy)),
            open: match self.default_target {
                Some(Target::Open(id)) => Some(id),
                _ => None,
            },
            plan: Err(String::new()),
            preview: None,
        };
        ready.replan(None);
        self.state = State::Ready(Box::new(ready));
    }
    pub fn options(&self) -> Option<&ImportOptions> {
        match &self.state {
            State::Ready(r) => Some(&r.options),
            _ => None,
        }
    }
    pub fn preview(&self) -> Option<&ImportPreview> {
        match &self.state {
            State::Ready(r) => r.preview.as_ref(),
            _ => None,
        }
    }
    pub fn target(&self) -> Option<Target> {
        match &self.state {
            State::Ready(r) => Some(r.target),
            _ => None,
        }
    }
    /// Changes options through the same path as the Inspector controls.
    pub fn edit(
        &mut self,
        session: &Session,
        report: Option<&ReferenceReport>,
        change: impl FnOnce(&mut ImportOptions, &mut Target),
    ) {
        if let State::Ready(ready) = &mut self.state {
            let units = ready.options.units;
            change(&mut ready.options, &mut ready.target);
            if ready.options.units != units {
                ready.artwork =
                    sketch_import::parse(ready.file.format, &ready.file.bytes, ready.options.units)
                        .map_err(|e| e.0);
                ready.options.layers = None;
            }
            ready.replan(Some((session, report)));
        }
    }
    pub fn can_apply(&self) -> bool {
        matches!(&self.state, State::Ready(r) if r.plan.is_ok() && r.preview.as_ref().is_some_and(|p| p.applicable.is_ok()))
    }

    pub fn apply(
        &mut self,
        session: &mut Session,
        report: Option<&ReferenceReport>,
        channel: Channel,
    ) -> Result<ImportOutcome, String> {
        let State::Ready(ready) = &self.state else {
            return Err("Choose a DXF or SVG file first".into());
        };
        let plan = ready.plan.as_ref().map_err(|e| e.clone())?;
        let target = match ready.target {
            Target::Open(id) => ImportTarget::Existing(id),
            Target::New(datum) => ImportTarget::New {
                datum,
                name: sketch_name(&ready.file.file_name),
            },
        };
        let report = match ready.target {
            Target::Open(_) => report,
            Target::New(_) => None,
        };
        let edit = prepare_import(
            session,
            session.revision(),
            Actor::local(channel),
            &target,
            report,
            plan,
            &ImportSource {
                file_name: &ready.file.file_name,
                bytes: &ready.file.bytes,
            },
        )
        .map_err(|e| e.to_string())?;
        let preview = edit.edit.preview.ok_or_else(|| {
            format!(
                "Imported sketch did not solve: {:?}",
                edit.edit.evaluation.diagnostics.state
            )
        })?;
        session.commit(preview).map_err(|e| e.to_string())?;
        self.state = State::Idle;
        self.error = None;
        Ok(ImportOutcome::Applied {
            sketch: edit.sketch,
            curves: edit.curves,
        })
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        report: Option<&ReferenceReport>,
    ) -> Option<ImportOutcome> {
        ui.heading("Import DXF / SVG");
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.pending() {
            ui.spinner();
            ui.small("Reading the selected file…");
        }
        if let State::Ready(ready) = &mut self.state
            && ready.controls(ui, session)
        {
            ready.replan(Some((session, report)));
        }
        let can_apply = self.can_apply();
        let clicks = inspector_actions::row(
            ui,
            Owner::Import,
            None,
            Button::new(
                "Apply",
                can_apply,
                "Create the imported curves in one undoable transaction",
            ),
            Button::new(
                "Cancel",
                true,
                "Close the import without changing the document",
            ),
        );
        if clicks.cancel {
            self.cancel();
            return Some(ImportOutcome::Cancelled);
        }
        if clicks.apply {
            match self.apply(session, report, Channel::Ui) {
                Ok(outcome) => return Some(outcome),
                Err(error) => self.error = Some(error),
            }
        }
        None
    }
}

fn sketch_name(file_name: &str) -> String {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    let name: String = stem.chars().filter(|c| !c.is_control()).take(120).collect();
    if name.trim().is_empty() {
        "Imported sketch".into()
    } else {
        name
    }
}

fn unit_label(choice: UnitChoice, format: Format) -> String {
    match choice {
        UnitChoice::FromFile => match format {
            Format::Dxf => "From file ($INSUNITS)".into(),
            Format::Svg => "From file (size/viewBox; px = 1/96 in)".into(),
        },
        UnitChoice::Explicit(unit) => unit.label().into(),
    }
}

impl Ready {
    fn replan(&mut self, context: Option<(&Session, Option<&ReferenceReport>)>) {
        self.plan = match &self.artwork {
            Ok(artwork) => sketch_import::plan::plan(artwork, &self.options).map_err(|e| e.0),
            Err(e) => Err(e.clone()),
        };
        let target_source = match (self.target, context) {
            (Target::Open(id), Some((session, report))) => {
                Source::read_for_evaluation(session.document(), id, session.revision(), report)
                    .map(|(s, _, _)| s)
                    .map_err(|e| e.to_string())
            }
            (Target::Open(_), None) => Err("Open sketch is not available".into()),
            (Target::New(datum), _) => Ok(Source {
                u: datum.frame().u,
                sketch: Default::default(),
            }),
        };
        self.preview = match (&self.plan, target_source) {
            (Ok(plan), Ok(source)) => Some(plan.preview(&source)),
            (Ok(plan), Err(error)) => {
                let mut preview = plan.preview(&Source {
                    u: [1., 0., 0.],
                    sketch: Default::default(),
                });
                preview.applicable = Err(error);
                Some(preview)
            }
            (Err(_), _) => None,
        };
    }

    /// Returns true when an option changed and the plan must be rebuilt.
    fn controls(&mut self, ui: &mut egui::Ui, session: &Session) -> bool {
        let mut changed = false;
        ui.label(format!(
            "{} · {} · {:.1} KiB",
            self.file.file_name,
            self.file.format.label(),
            self.file.bytes.len() as f64 / 1024.
        ));
        ui.separator();
        ui.strong("Target sketch");
        if let Some(id) = self.open {
            let name = session
                .document()
                .get(id)
                .map_or("sketch", |e| e.name.as_str());
            changed |= ui
                .radio_value(
                    &mut self.target,
                    Target::Open(id),
                    format!("Open sketch: {name}"),
                )
                .changed();
        }
        for datum in Datum::ALL {
            changed |= ui
                .radio_value(
                    &mut self.target,
                    Target::New(datum),
                    format!("New sketch on {}", datum.label()),
                )
                .changed();
        }
        ui.separator();
        ui.strong("Scale and placement");
        let units = self.options.units;
        egui::ComboBox::from_label("Source units")
            .selected_text(unit_label(units, self.file.format))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.options.units,
                    UnitChoice::FromFile,
                    unit_label(UnitChoice::FromFile, self.file.format),
                );
                for unit in SourceUnit::ALL {
                    ui.selectable_value(
                        &mut self.options.units,
                        UnitChoice::Explicit(unit),
                        unit.label(),
                    );
                }
            });
        if self.options.units != units {
            self.artwork =
                sketch_import::parse(self.file.format, &self.file.bytes, self.options.units)
                    .map_err(|e| e.0);
            self.options.layers = None;
            changed = true;
        }
        let number = |ui: &mut egui::Ui, label: &str, value: &mut f64, speed: f64, suffix: &str| {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::DragValue::new(value).speed(speed).suffix(suffix))
                    .changed()
            })
            .inner
        };
        changed |= number(ui, "Scale factor", &mut self.options.scale, 0.01, "×");
        changed |= number(
            ui,
            "Insertion X",
            &mut self.options.insertion_mm[0],
            0.5,
            " mm",
        );
        changed |= number(
            ui,
            "Insertion Y",
            &mut self.options.insertion_mm[1],
            0.5,
            " mm",
        );
        changed |= number(ui, "Rotation", &mut self.options.rotation_deg, 1., "°");
        changed |= number(
            ui,
            "Endpoint weld",
            &mut self.options.weld_tolerance_mm,
            0.0001,
            " mm",
        );
        changed |= ui
            .checkbox(
                &mut self.options.construction,
                "Import as construction geometry",
            )
            .changed();
        if let Ok(artwork) = &self.artwork {
            for note in &artwork.notes {
                ui.small(note);
            }
            ui.separator();
            ui.strong(match self.file.format {
                Format::Dxf => "Layers",
                Format::Svg => "Groups",
            });
            let summaries = self
                .preview
                .as_ref()
                .map(|p| p.layers.clone())
                .unwrap_or_default();
            for layer in &artwork.layers {
                let mut on = self
                    .options
                    .layers
                    .as_ref()
                    .is_none_or(|s| s.contains(layer));
                let curves = summaries
                    .iter()
                    .find(|s| &s.name == layer)
                    .map_or(0, |s| s.curves);
                if ui
                    .checkbox(&mut on, format!("{layer} · {curves} curves"))
                    .changed()
                {
                    let mut set: BTreeSet<String> = self
                        .options
                        .layers
                        .clone()
                        .unwrap_or_else(|| artwork.layers.iter().cloned().collect());
                    if on {
                        set.insert(layer.clone());
                    } else {
                        set.remove(layer);
                    }
                    self.options.layers = Some(set);
                    changed = true;
                }
            }
        }
        ui.separator();
        ui.strong("Preview");
        match (&self.artwork, &self.plan, &self.preview) {
            (Err(error), _, _) | (_, Err(error), _) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            (_, _, Some(p)) => {
                if let Some([min, max]) = p.bounds_mm {
                    ui.label(format!(
                        "Bounds X {:.3} … {:.3} mm, Y {:.3} … {:.3} mm ({:.3} × {:.3} mm)",
                        min[0],
                        max[0],
                        min[1],
                        max[1],
                        max[0] - min[0],
                        max[1] - min[1]
                    ));
                }
                ui.label(format!(
                    "{} lines · {} arcs · {} circles · {} splines",
                    p.curves.lines, p.curves.arcs, p.curves.circles, p.curves.splines
                ));
                ui.label(format!(
                    "{} closed loops · {} open chains · {} new points",
                    p.closed_loops, p.open_chains, p.new_points
                ));
                if p.skipped_degenerate > 0 {
                    ui.small(format!(
                        "{} zero-length curves skipped",
                        p.skipped_degenerate
                    ));
                }
                if !p.unsupported.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, "Not imported:");
                    for d in &p.unsupported {
                        ui.small(format!("{} × {}", d.count, d.message));
                    }
                }
                if let Err(reason) = &p.applicable {
                    ui.colored_label(egui::Color32::LIGHT_RED, reason);
                }
            }
            _ => {}
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::document::Document;

    const DXF: &[u8] = include_bytes!("../../multicad-core/tests/fixtures/sketch_import/plate.dxf");
    const SVG: &[u8] = include_bytes!("../../multicad-core/tests/fixtures/sketch_import/badge.svg");

    fn session() -> Session {
        Session::new(Document::new("Artwork").unwrap()).unwrap()
    }
    fn frame(
        ctx: &egui::Context,
        c: &mut ImportController,
        s: &mut Session,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<ImportOutcome>) {
        let mut outcome = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320., 1400.),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                inspector_actions::scope(ui, Some(Owner::Import), egui::Color32::BLUE, |ui| {
                    outcome = c.show(ui, s, None);
                });
            },
        );
        output.textures_delta.clear();
        (output, outcome)
    }
    fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(s) => {
                    out.push((s.galley.text().to_string(), s.pos + s.galley.size() * 0.5))
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        output.shapes.iter().for_each(|s| walk(&s.shape, &mut out));
        out
    }
    fn click(
        ctx: &egui::Context,
        c: &mut ImportController,
        s: &mut Session,
        pos: egui::Pos2,
    ) -> Option<ImportOutcome> {
        let mut outcome = None;
        for pressed in [true, false] {
            let (output, value) = frame(
                ctx,
                c,
                s,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
            output.drop_without_applying_deltas();
            outcome = value.or(outcome);
        }
        outcome
    }

    #[test]
    fn inspector_previews_layers_bounds_and_unsupported_content_then_applies_one_transaction() {
        let mut s = session();
        let baseline = s.document().clone();
        let mut c = ImportController::default();
        c.begin_bytes("badge.svg", SVG.to_vec(), None).unwrap();
        let ctx = egui::Context::default();
        let (output, _) = frame(&ctx, &mut c, &mut s, vec![]);
        let painted = texts(&output);
        let has = |needle: &str| painted.iter().any(|(t, _)| t.contains(needle));
        assert!(has("Outline · 8 curves") && has("holes · 4 curves") && has("island · 1 curves"));
        assert!(
            has("Bounds X 5.000 … 75.000 mm, Y 5.000 … 35.000 mm"),
            "{painted:?}"
        );
        assert!(
            has("4 lines · 4 arcs · 2 circles · 3 splines"),
            "{painted:?}"
        );
        assert!(has("5 closed loops · 0 open chains"), "{painted:?}");
        assert!(has("Scripts are ignored") && has("Text is not converted"));
        assert!(has("viewBox maps to 80.000000 × 40.000000 mm"));
        assert_eq!(s.document(), &baseline, "preview never edits the document");
        let apply = painted.iter().find(|(t, _)| t == "Apply").unwrap().1;
        let outcome = click(&ctx, &mut c, &mut s, apply);
        let Some(ImportOutcome::Applied { sketch, curves }) = outcome else {
            panic!("{outcome:?} {:?}", c.error)
        };
        assert_eq!(curves.len(), 13);
        assert_eq!(s.revision(), 1);
        assert_eq!(s.document().get(sketch).unwrap().name, "badge");
        assert_eq!(
            multicad_core::sketch_import::provenance::for_sketch(s.document(), sketch).len(),
            1
        );
        assert!(!c.active());
        s.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &baseline);
    }

    #[test]
    fn unitless_dxf_requires_an_explicit_unit_layers_filter_and_cancel_changes_nothing() {
        let mut s = session();
        let unitless =
            String::from_utf8_lossy(DXF).replace("$INSUNITS\r\n70\r\n4", "$INSUNITS\r\n70\r\n0");
        let mut c = ImportController::default();
        c.begin_bytes("plate.dxf", unitless.into_bytes(), None)
            .unwrap();
        assert!(!c.can_apply());
        c.edit(&s, None, |o, _| {
            o.units = UnitChoice::Explicit(SourceUnit::Inch)
        });
        assert!(c.can_apply());
        c.edit(&s, None, |o, t| {
            o.layers = Some(BTreeSet::from(["CUT".into()]));
            *t = Target::New(Datum::Yz);
        });
        let p = c.preview().unwrap();
        assert_eq!((p.curves.total(), p.closed_loops), (10, 3));
        let [min, max] = p.bounds_mm.unwrap();
        assert!((max[0] - min[0] - 2540.).abs() < 1e-9 && (max[1] - min[1] - 1524.).abs() < 1e-9);
        let ctx = egui::Context::default();
        let (output, _) = frame(&ctx, &mut c, &mut s, vec![]);
        let cancel = texts(&output)
            .into_iter()
            .find(|(t, _)| t == "Cancel")
            .unwrap()
            .1;
        assert_eq!(
            click(&ctx, &mut c, &mut s, cancel),
            Some(ImportOutcome::Cancelled)
        );
        assert_eq!(s.revision(), 0);
        assert!(!c.active());
    }

    #[test]
    fn existing_open_sketch_target_appends_and_stale_failure_keeps_the_panel() {
        let (sketch, edits) = Source::create(Datum::Xy, "Open").unwrap();
        let mut s =
            Session::new(Document::new("Artwork").unwrap().edited(&edits).unwrap()).unwrap();
        let mut c = ImportController::default();
        c.begin_bytes("plate.dxf", DXF.to_vec(), Some(sketch))
            .unwrap();
        assert_eq!(c.target(), Some(Target::Open(sketch)));
        c.edit(&s, None, |o, _| {
            o.layers = Some(BTreeSet::from(["MIRROR".into()]))
        });
        let outcome = c.apply(&mut s, None, Channel::Ui).unwrap();
        assert_eq!(
            outcome,
            ImportOutcome::Applied {
                sketch,
                curves: outcome_curves(&outcome)
            }
        );
        // A deleted target cannot be imported into; the document stays unchanged.
        let mut c = ImportController::default();
        c.begin_bytes("plate.dxf", DXF.to_vec(), Some(EntityId::new()))
            .unwrap();
        let before = s.document().clone();
        assert!(c.apply(&mut s, None, Channel::Ui).is_err());
        assert_eq!(s.document(), &before);
        assert!(c.active());
        assert!(read_bounded(Path::new("/nonexistent/art.dxf")).is_err());
        assert!(
            read_bounded(Path::new("notes.txt"))
                .unwrap_err()
                .contains(".dxf")
        );
    }

    fn outcome_curves(outcome: &ImportOutcome) -> Vec<EntityId> {
        match outcome {
            ImportOutcome::Applied { curves, .. } => {
                assert_eq!(curves.len(), 2);
                curves.clone()
            }
            ImportOutcome::Cancelled => panic!(),
        }
    }
}
