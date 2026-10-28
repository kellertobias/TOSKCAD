//! Source-neutral associative end controls and current canonical viewer picks.
//! Stored face intent remains on the shared FaceInput link; no display frame,
//! occurrence transform or arbitrary inherited anchor is baked into source.
use super::dimension::DimensionField;
use crate::{
    camera::Camera,
    controls, egui,
    viewport::{Scene, SceneStamp, SectionPlane, Selection, Transform},
};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{FaceInput, Termination, length},
    project_file::ContentHash,
    references::{GeometryIntent, ReferenceReport, TopologyKey, TopologyKind},
    sketch::{self, Frame},
};

pub struct TerminationViewport<'a> {
    pub scene: Option<&'a Scene>,
    pub report: Option<&'a ReferenceReport>,
    pub camera: Camera,
    pub rect: egui::Rect,
    pub pointer: Option<egui::Pos2>,
    pub clicked: bool,
    pub painter: &'a egui::Painter,
    pub section: Option<SectionPlane>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EndSide {
    Forward,
    Backward,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EndKind {
    Distance,
    Plane,
    Face,
}
impl EndKind {
    fn label(self) -> &'static str {
        match self {
            Self::Distance => "Distance",
            Self::Plane => "To plane",
            Self::Face => "To face",
        }
    }
}
pub(super) struct EndAction {
    pub changed: bool,
    pub pick: Option<EndKind>,
    pub cancel: bool,
}
pub(super) struct EndCondition {
    pub kind: EndKind,
    pub distance: DimensionField,
    offset: DimensionField,
    plane: Option<EntityId>,
    face: Option<FaceInput>,
}
impl EndCondition {
    pub fn new(end: &Termination, faces: &[FaceInput]) -> Result<Self, String> {
        let (kind, distance, offset, plane, face) = match end {
            Termination::Distance { distance } => {
                (EndKind::Distance, distance.clone(), length(0.), None, None)
            }
            Termination::ToPlane { plane, offset } => (
                EndKind::Plane,
                length(10.),
                offset.clone(),
                Some(*plane),
                None,
            ),
            Termination::ToFace { reference, offset } => {
                let matches: Vec<_> = faces.iter().filter(|f| f.intent.id == *reference).collect();
                if matches.len() != 1 {
                    return Err("The saved end face needs its unique canonical input link; repair the reference before reopening".into());
                }
                (
                    EndKind::Face,
                    length(10.),
                    offset.clone(),
                    None,
                    Some(matches[0].clone()),
                )
            }
        };
        Ok(Self {
            kind,
            distance: DimensionField::from_dimension(&distance, "mm")?,
            offset: DimensionField::from_dimension(&offset, "mm")?,
            plane,
            face,
        })
    }
    pub fn resolve(
        &self,
        document: &Document,
        planes: bool,
        faces: bool,
    ) -> Result<(Termination, Option<FaceInput>), String> {
        match self.kind {
            EndKind::Distance => Ok((
                Termination::Distance {
                    distance: self.distance.resolve(document)?,
                },
                None,
            )),
            EndKind::Plane => {
                if !planes {
                    return Err("Associative plane limits are unavailable in this provider".into());
                }
                let plane = self
                    .plane
                    .ok_or("Select an existing construction plane in the viewer or list")?;
                if !matches!(
                    document.get(plane).map(|e| &e.data),
                    Some(EntityData::Plane { .. })
                ) {
                    return Err(
                        "The selected construction plane is missing; select or repair it".into(),
                    );
                }
                Ok((
                    Termination::ToPlane {
                        plane,
                        offset: self.offset.resolve(document)?,
                    },
                    None,
                ))
            }
            EndKind::Face => {
                if !faces {
                    return Err("Associative face limits are unavailable in this provider".into());
                }
                let face = self
                    .face
                    .as_ref()
                    .ok_or("Select a named planar face in the current model")?;
                face.intent.validate()?;
                if face.intent.kind != TopologyKind::Face
                    || !matches!(
                        document.get(face.source).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                {
                    return Err("The selected face producer is missing or suppressed; repair its canonical link".into());
                }
                Ok((
                    Termination::ToFace {
                        reference: face.intent.id,
                        offset: self.offset.resolve(document)?,
                    },
                    Some(face.clone()),
                ))
            }
        }
    }
    pub fn remember(&mut self, end: &Termination) {
        match end {
            Termination::Distance { distance } => self.distance.remember(distance),
            Termination::ToPlane { offset, .. } | Termination::ToFace { offset, .. } => {
                self.offset.remember(offset)
            }
        }
    }
    pub fn accept(&mut self, choice: PickChoice) {
        match choice {
            PickChoice::Plane { id, .. } => {
                self.kind = EndKind::Plane;
                self.plane = Some(id);
            }
            PickChoice::Face { input, .. } => {
                self.kind = EndKind::Face;
                self.face = Some(input);
            }
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        label: &str,
        planes: bool,
        faces: bool,
        picking: bool,
    ) -> EndAction {
        let mut action = EndAction {
            changed: false,
            pick: None,
            cancel: false,
        };
        ui.label(label);
        controls::inspector_combo(ui, ("extrude-end-kind", label))
            .selected_text(self.kind.label())
            .show_ui(ui, |ui| {
                for (kind, enabled) in [
                    (EndKind::Distance, true),
                    (EndKind::Plane, planes),
                    (EndKind::Face, faces),
                ] {
                    ui.add_enabled_ui(enabled, |ui| {
                        action.changed |= ui
                            .selectable_value(&mut self.kind, kind, kind.label())
                            .changed();
                    })
                    .response
                    .on_disabled_hover_text(
                        "This end condition is unavailable in the native provider",
                    );
                }
            });
        match self.kind {
            EndKind::Distance => {
                action.changed |= self.distance.show(
                    ui,
                    "Distance",
                    "Positive distance from the offset start along this side's sweep direction",
                )
            }
            EndKind::Plane => {
                controls::inspector_combo(ui,("extrude-end-plane",label)).selected_text(self.plane.and_then(|id|document.get(id)).map_or("Select construction plane…",|e|e.name.as_str())).show_ui(ui,|ui|{
                    for entity in document.entities().filter(|e|matches!(e.data,EntityData::Plane{..})){
                        if ui.selectable_label(self.plane==Some(entity.id),&entity.name).on_hover_text("Use this authored plane; exact Preview validates its current references and reachability").clicked(){self.plane=Some(entity.id);action.changed=true;}
                    }
                }).response.on_hover_text("Choose an existing canonical plane even when its glyph is outside the viewport");
                action.changed|=self.offset.show(ui,"Plane offset","Signed offset along the evaluated plane's positive normal; formulas retain stable parameter bindings");
            }
            EndKind::Face => {
                ui.label(
                    self.face
                        .as_ref()
                        .and_then(|f| document.get(f.source))
                        .map_or("No face selected".into(), |e| format!("Face on {}", e.name)),
                );
                action.changed|=self.offset.show(ui,"Face offset","Signed offset follows the current native face's outward normal. Preview validates the exact saved face reference");
            }
        }
        if self.kind != EndKind::Distance {
            if picking{if ui.button("Cancel target selection").on_hover_text("Keep the prior end target without changing document source").clicked(){action.cancel=true;}}
            else if ui.button(if self.kind==EndKind::Plane{"Pick plane in viewer"}else{"Pick face in viewer"}).on_hover_text("Select on the fresh canonical model; candidate and stale geometry cannot become end intent").clicked(){action.pick=Some(self.kind);}
        }
        action
    }
}
#[derive(Clone)]
pub(super) enum PickChoice {
    Plane {
        id: EntityId,
        frame: Frame,
    },
    Face {
        input: FaceInput,
        instance: EntityId,
        key: TopologyKey,
        stamp: SceneStamp,
    },
}
#[derive(Default)]
pub(super) struct EndPicker {
    pub active: Option<(EndSide, EndKind)>,
    hovered: Option<PickChoice>,
    message: Option<String>,
}
impl EndPicker {
    pub fn begin(&mut self, side: EndSide, kind: EndKind) {
        self.active = Some((side, kind));
        self.hovered = None;
        self.message = None;
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub fn highlight(&self) -> Option<Selection> {
        match &self.hovered {
            Some(PickChoice::Face { instance, key, .. }) => Some(Selection::Face {
                instance: *instance,
                ordinal: key.ordinal,
            }),
            _ => None,
        }
    }
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        source_hash: ContentHash,
        view: TerminationViewport<'_>,
        forbidden: &[EntityId],
    ) -> Option<(EndSide, PickChoice)> {
        let (side, kind) = self.active?;
        self.message = None;
        let current = || -> Result<(), String> {
            let scene = view
                .scene
                .ok_or("Wait for the current canonical model before picking an end target")?;
            if scene.stamp().document != document.id()
                || scene.stamp().revision != revision
                || scene
                    .model_evaluation()
                    .is_none_or(|info| info.source_hash != source_hash)
            {
                return Err("The visible end target is stale or belongs to an unapplied candidate; wait for the canonical model".into());
            }
            Ok(())
        };
        if let Err(error) = current() {
            self.hovered = None;
            self.message = Some(error);
            return None;
        }
        let pointer = view.pointer.filter(|p| view.rect.contains(*p));
        let result = match kind {
            EndKind::Face => pointer
                .map(|pointer| {
                    pick_face(
                        document,
                        revision,
                        &view,
                        pointer,
                        forbidden,
                        self.hovered.as_ref(),
                    )
                })
                .transpose()
                .map(Option::flatten),
            EndKind::Plane => {
                let mut chosen = None;
                let mut depth = f32::INFINITY;
                let mut ambiguous = false;
                for entity in document
                    .entities()
                    .filter(|e| matches!(e.data, EntityData::Plane { .. }))
                {
                    let Ok(frame) =
                        sketch::inspect_plane(document, entity.id, revision, view.report)
                    else {
                        continue;
                    };
                    paint_plane(
                        view.painter,
                        view.camera,
                        view.rect,
                        frame,
                        &entity.name,
                        false,
                    );
                    if let Some(pointer) = pointer
                        && let Some(p) = view.camera.on_plane(pointer, view.rect, frame)
                        && p.iter().all(|v| v.abs() <= 40.)
                    {
                        let z = view
                            .camera
                            .project(frame.point(p).map(|v| v as f32), view.rect)[2];
                        if z >= 0.1 {
                            if (z - depth).abs() < 1e-5 {
                                ambiguous = true;
                            } else if z < depth {
                                depth = z;
                                ambiguous = false;
                                chosen = Some(PickChoice::Plane {
                                    id: entity.id,
                                    frame,
                                });
                            }
                        }
                    }
                }
                if ambiguous {
                    Err("Multiple construction planes overlap here; choose one by name in the end-condition list".into())
                } else {
                    Ok(chosen)
                }
            }
            EndKind::Distance => Ok(None),
        };
        match result {
            Ok(choice) => self.hovered = choice,
            Err(error) => {
                self.hovered = None;
                self.message = Some(error);
            }
        }
        if let Some(PickChoice::Plane { frame, id }) = &self.hovered {
            paint_plane(
                view.painter,
                view.camera,
                view.rect,
                *frame,
                document.get(*id).map_or("Plane", |e| e.name.as_str()),
                true,
            );
        }
        if view.clicked {
            if let Some(choice) = self.hovered.clone() {
                self.cancel();
                return Some((side, choice));
            }
            if self.message.is_none() {
                self.message = Some(
                    if kind == EndKind::Plane {
                        "Select a visible construction-plane glyph or use its explicit list"
                    } else {
                        "Select a named planar face on the current model"
                    }
                    .into(),
                );
            }
        }
        None
    }
}
fn pick_face(
    document: &Document,
    revision: u64,
    view: &TerminationViewport<'_>,
    pointer: egui::Pos2,
    forbidden: &[EntityId],
    cached: Option<&PickChoice>,
) -> Result<Option<PickChoice>, String> {
    let scene = view.scene.ok_or("Wait for the current canonical model")?;
    let Some(ray) = view.camera.ray(pointer, view.rect) else {
        return Ok(None);
    };
    let Some(hit) = scene.pick_face_section(ray, view.section) else {
        return Ok(None);
    };
    if hit.stamp != scene.stamp()
        || !matches!(
            document.get(hit.geometry).map(|e| &e.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        )
    {
        return Err("Choose a current upstream modeling face; an unapplied/self face cannot terminate its own extrusion".into());
    }
    if forbidden.contains(&hit.geometry) {
        return Err(
            "An extrusion cannot terminate on its own result; choose an upstream face".into(),
        );
    }
    let occurrence = scene
        .instances()
        .iter()
        .find(|i| i.id == hit.instance && i.geometry == hit.geometry)
        .ok_or("Face occurrence is no longer available")?;
    if occurrence.transform != Transform::default() {
        return Err("Associative end picking currently requires an unplaced source occurrence; cross-occurrence end limits are unsupported".into());
    }
    if let Some(PickChoice::Face {
        input,
        instance,
        key,
        stamp,
    }) = cached
        && *stamp == scene.stamp()
        && *instance == hit.instance
        && *key == hit.key
        && input.source == hit.geometry
    {
        return Ok(cached.cloned());
    }
    let snapshot = scene
        .geometry_snapshot(hit.geometry)
        .ok_or("The face source has no current topology")?;
    if snapshot.owner != hit.geometry || snapshot.revision != revision {
        return Err("Face topology is stale; select it again after the model rebuilds".into());
    }
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key == hit.key)
        .ok_or("Selected face is missing")?;
    if element.frames.is_empty() {
        return Err("Choose a named planar face; curved or unframed surfaces cannot be picked as an end target".into());
    }
    for frame in &element.frames {
        frame.frame.validate()?;
    }
    let intent = GeometryIntent::capture(&snapshot, hit.key)?;
    // No implicit anchor: the native provider obtains this uniquely named
    // face's true support plane/outward normal, including multi-frame faces.
    Ok(Some(PickChoice::Face {
        input: FaceInput {
            source: hit.geometry,
            intent,
        },
        instance: hit.instance,
        key: hit.key,
        stamp: scene.stamp(),
    }))
}
fn paint_plane(
    painter: &egui::Painter,
    camera: Camera,
    rect: egui::Rect,
    frame: Frame,
    label: &str,
    active: bool,
) {
    let color = if active {
        egui::Color32::from_rgb(90, 195, 225)
    } else {
        egui::Color32::from_rgb(75, 110, 145)
    };
    let points = [[-40., -40.], [40., -40.], [40., 40.], [-40., 40.]].map(|p| frame.point(p));
    for i in 0..4 {
        if let Some(segment) = camera.segment(points[i], points[(i + 1) % 4], rect) {
            painter.line_segment(
                segment,
                egui::Stroke::new(if active { 1.8 } else { 0.8 }, color),
            );
        }
    }
    let p = camera.project(frame.origin.map(|x| x as f32), rect);
    if p[2] >= 0.1 {
        painter.text(
            egui::Pos2::new(p[0], p[1]),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(11.),
            color,
        );
    }
}

#[cfg(test)]
mod tests;
