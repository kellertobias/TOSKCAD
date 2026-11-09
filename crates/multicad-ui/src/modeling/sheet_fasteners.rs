//! Append one metric fastener pattern while keeping existing sheet hardware.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{egui, viewport::Scene};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityData, EntityId},
    model::{self, CutoutFace, HardwarePreset, HardwareSide, MountingPattern, Thread},
};

#[derive(Clone, Debug)]
pub struct FastenerCommand {
    feature: EntityId,
    pub(super) part: EntityId,
    pattern: MountingPattern,
}

impl FastenerCommand {
    pub(super) fn prepare(
        &self,
        session: &Session,
        actor: Actor,
    ) -> Result<model::ModelPrepared, String> {
        let mut sheet = model::read_sheet(session.document(), self.feature)
            .map_err(|error| error.to_string())?;
        if sheet.output.body != self.part {
            return Err("The selected sheet Part changed".into());
        }
        sheet.fasteners.push(self.pattern.clone());
        model::prepare_sheet_fasteners(
            session,
            session.revision(),
            actor,
            self.feature,
            &sheet.fasteners,
        )
        .map(|prepared| prepared.model)
        .map_err(|error| error.to_string())
    }
}

/// Hardware Parts are generated in definition coordinates. Only one physical
/// occurrence at that pose can use these Parts without separate placements.
pub(super) fn check_placement(
    document: &Document,
    part: EntityId,
    scene: &Scene,
) -> Result<(), String> {
    if !document.entities().any(
        |entity| matches!(entity.data, EntityData::Instance { component, .. } if component == part),
    ) {
        return Ok(());
    }
    let info = scene
        .assembly_evaluation()
        .ok_or("Placed sheets require a current native placement check")?;
    let mut occurrences = info.leaves.iter().filter(|leaf| leaf.component == part);
    if occurrences
        .next()
        .is_some_and(|leaf| leaf.pose == Default::default())
        && occurrences.next().is_none()
    {
        Ok(())
    } else {
        Err(
            "Adding fasteners requires an unplaced sheet or one occurrence at its definition pose"
                .into(),
        )
    }
}

pub(super) struct FastenerPanel {
    stamp: FormStamp,
    feature: EntityId,
    part: EntityId,
    name: String,
    flanges: Vec<u32>,
    existing: Vec<MountingPattern>,
    suppressed: bool,
    face: CutoutFace,
    side: HardwareSide,
    thread: Thread,
    length: DimensionField,
    washers: bool,
    nut: bool,
    points: Vec<[DimensionField; 2]>,
}

fn field(mm: f64) -> DimensionField {
    DimensionField::from_dimension(&model::length(mm), "mm").expect("valid length field")
}

fn face_label(face: CutoutFace) -> String {
    match face {
        CutoutFace::Base => "Base sheet".into(),
        CutoutFace::Flange { index } => format!("Flange {}", index + 1),
    }
}

fn pattern_label(pattern: &MountingPattern) -> String {
    let kind = match &pattern.preset {
        HardwarePreset::Screw {
            thread,
            washers,
            nut,
            ..
        } => {
            let mut label = format!("{} socket-head screws", thread.label());
            if *washers {
                label.push_str(" + washers");
            }
            if *nut {
                label.push_str(" + nuts");
            }
            label
        }
        HardwarePreset::Standoff { thread, .. } => format!("{} standoffs", thread.label()),
        HardwarePreset::Insert { designation, .. } => designation.clone(),
    };
    format!(
        "{kind} · {} · {} points",
        face_label(pattern.face),
        pattern.points.len()
    )
}

impl FastenerPanel {
    pub(super) fn new(document: &Document, target: EntityId) -> Result<Self, String> {
        let selected = document.get(target).ok_or("Select a sheet-metal Part")?;
        let target = match selected.data {
            EntityData::Instance { component, .. } => component,
            _ => target,
        };
        let feature = model::sheet_feature(document, target).unwrap_or(target);
        let sheet = model::read_sheet(document, feature)
            .map_err(|_| "Select a sheet-metal Part, feature, or placed sheet".to_owned())?;
        let part = sheet.output.body;
        let name = document
            .get(part)
            .ok_or("The sheet Part is missing")?
            .name
            .clone();
        let suppressed = document.get(feature).is_some_and(|entity| {
            matches!(
                entity.data,
                EntityData::Feature {
                    suppressed: true,
                    ..
                }
            )
        });
        Ok(Self {
            stamp: FormStamp::new(),
            feature,
            part,
            name,
            flanges: sheet.flanges.iter().map(|flange| flange.edge).collect(),
            existing: sheet.fasteners,
            suppressed,
            face: CutoutFace::Base,
            side: HardwareSide::Material,
            thread: Thread::M3,
            length: field(12.),
            washers: false,
            nut: false,
            points: vec![[field(10.), field(10.)]],
        })
    }

    pub(super) fn stamp(&self) -> FormStamp {
        self.stamp
    }

    pub(super) fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<FastenerCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.suppressed {
            return Err("Unsuppress the sheet-metal feature before adding fasteners".into());
        }
        if self.existing.len() >= model::MAX_PATTERNS {
            return Err(format!(
                "This sheet already has the limit of {} patterns",
                model::MAX_PATTERNS
            ));
        }
        let points = self
            .points
            .iter()
            .map(|[u, v]| Ok([u.resolve(document)?, v.resolve(document)?]))
            .collect::<Result<_, String>>()?;
        Ok(FastenerCommand {
            feature: self.feature,
            part: self.part,
            pattern: MountingPattern {
                preset: HardwarePreset::Screw {
                    thread: self.thread,
                    length: self.length.resolve(document)?,
                    washers: self.washers,
                    nut: self.nut,
                },
                face: self.face,
                side: self.side,
                points,
            },
        })
    }

    pub(super) fn remember(&mut self, command: &FastenerCommand) {
        if let HardwarePreset::Screw { length, .. } = &command.pattern.preset {
            self.length.remember(length);
        }
        for ([u, v], [saved_u, saved_v]) in self.points.iter_mut().zip(&command.pattern.points) {
            u.remember(saved_u);
            v.remember(saved_v);
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        ui.label(&self.name);
        ui.collapsing(
            format!("Existing patterns ({})", self.existing.len()),
            |ui| {
                if self.existing.is_empty() {
                    ui.label("No attached patterns");
                }
                for pattern in &self.existing {
                    ui.label(pattern_label(pattern));
                }
            },
        );
        ui.separator();
        ui.label("Add metric socket-head screws");
        ui.small("Mounting points use the selected face's local coordinates. Preview checks that the new hardware fits the sheet.");
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Mounting face");
            egui::ComboBox::from_id_salt("sheet-fastener-face")
                .selected_text(face_label(self.face))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.face, CutoutFace::Base, "Base sheet")
                        .changed();
                    for (index, edge) in self.flanges.iter().enumerate() {
                        let face = CutoutFace::Flange {
                            index: index as u32,
                        };
                        changed |= ui
                            .selectable_value(
                                &mut self.face,
                                face,
                                format!("Flange {} · base edge {}", index + 1, edge + 1),
                            )
                            .changed();
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Screw head side");
            egui::ComboBox::from_id_salt("sheet-fastener-side")
                .selected_text(match self.side {
                    HardwareSide::Material => "Material side",
                    HardwareSide::Base => "Base side",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.side, HardwareSide::Material, "Material side")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.side, HardwareSide::Base, "Base side")
                        .changed();
                });
        });
        ui.horizontal(|ui| {
            ui.label("Metric thread");
            egui::ComboBox::from_id_salt("sheet-fastener-thread")
                .selected_text(self.thread.label())
                .show_ui(ui, |ui| {
                    for thread in [Thread::M3, Thread::M4, Thread::M5, Thread::M6] {
                        changed |= ui
                            .selectable_value(&mut self.thread, thread, thread.label())
                            .changed();
                    }
                });
        });
        changed |=
            self.length
                .show_named(ui, "screw-length", "Screw length", "Length below the head");
        changed |= ui.checkbox(&mut self.washers, "Washers").changed();
        changed |= ui.checkbox(&mut self.nut, "Hex nuts").changed();

        let mut remove = None;
        let can_remove = self.points.len() > 1;
        let [u_help, v_help] = match self.face {
            CutoutFace::Base => [
                "Along the base sheet's U axis",
                "Along the base sheet's V axis",
            ],
            CutoutFace::Flange { .. } => [
                "Along the edge from the flange start",
                "Along the leg from the end of its last bend",
            ],
        };
        for (index, [u, v]) in self.points.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Mounting point {}", index + 1));
                if can_remove && ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
            changed |= u.show_named(ui, ("screw-point-u", index), "U", u_help);
            changed |= v.show_named(ui, ("screw-point-v", index), "V", v_help);
        }
        if let Some(index) = remove {
            self.points.remove(index);
            changed = true;
        }
        if ui
            .add_enabled(
                self.points.len() < model::MAX_PATTERN_POINTS,
                egui::Button::new("Add mounting point"),
            )
            .clicked()
        {
            self.points
                .push([field(10. * (self.points.len() + 1) as f64), field(10.)]);
            changed = true;
        }

        if changed {
            self.stamp.changed();
        }
        let request = self.request(document, enabled);
        if let Err(error) = &request {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        form::preview_status(ui, self.stamp, preview);
        match form::actions(ui, request.is_ok(), changed, self.stamp, preview) {
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            Some(FormAction::Apply) => Some(ModelEvent::Apply),
            Some(FormAction::Preview) => request.ok().map(|request| {
                self.remember(&request);
                ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::SheetFasteners(Box::new(request)),
                }
            }),
            None if changed => Some(ModelEvent::Changed),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{ModelToolController, native_fixture::NativeBackend};
    use multicad_core::{
        commands::Channel,
        project_file::{self, ProjectResources},
    };
    use std::time::{Duration, Instant};

    #[test]
    fn native_preview_appends_a_metric_pattern_to_the_shipped_placed_sheet() {
        let project = project_file::read(
            &mut include_bytes!("../../../../examples/front-panel.tcd").as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let mut session = Session::new(project.document).unwrap();
        let resources: &ProjectResources = &project.resources;
        let target = session
            .document()
            .entities()
            .find(|entity| entity.name == "Sheet-metal front panel")
            .unwrap()
            .id;
        let panel = FastenerPanel::new(session.document(), target).unwrap();
        let feature = panel.feature;
        let existing = panel.existing;
        let original_hardware = model::sheet_hardware(session.document(), feature).unwrap();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(NativeBackend::new()));
        controller
            .begin_sheet_fasteners(&session, resources, target, Channel::Ui)
            .unwrap();
        let ctx = egui::Context::default();
        for time in [0., 0.5] {
            ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    controller.tick_auto_preview(ui, &session, resources, false);
                },
            )
            .drop_without_applying_deltas();
        }
        assert!(!controller.can_apply(&session, resources));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !controller.can_apply(&session, resources) {
            controller.poll(&session, resources);
            assert!(controller.error().is_none(), "{:?}", controller.error());
            assert!(
                Instant::now() < deadline,
                "Native fastener preview timed out"
            );
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(session.revision(), 0);
        assert_eq!(
            model::read_sheet(session.document(), feature)
                .unwrap()
                .fasteners,
            existing
        );
        controller.apply(&mut session, resources).unwrap();
        let result = model::read_sheet(session.document(), feature).unwrap();
        assert_eq!(&result.fasteners[..existing.len()], existing);
        let added = result.fasteners.last().unwrap();
        assert_eq!(result.fasteners.len(), existing.len() + 1);
        assert!(matches!(
            added.preset,
            HardwarePreset::Screw {
                thread: Thread::M3,
                ..
            }
        ));
        assert_eq!(added.points, vec![[model::length(10.), model::length(10.)]]);
        let hardware = model::sheet_hardware(session.document(), feature).unwrap();
        assert_eq!(hardware.len(), original_hardware.len() + 1);
        assert!(original_hardware.iter().all(|item| hardware.contains(item)));
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(
            model::sheet_hardware(session.document(), feature).unwrap(),
            original_hardware
        );
    }
}
