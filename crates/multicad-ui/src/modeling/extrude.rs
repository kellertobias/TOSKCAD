use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction as Action},
    termination::{EndCondition, EndPicker, EndSide, TerminationViewport},
};
use crate::{controls, egui};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{
        Direction, Extent, ExtrudeParameters, ExtrudeRequest, FaceInput, Operation, Termination,
    },
    project_file::ContentHash,
    sketch_regions::RegionIntent,
};

/// Availability comes from the installed native provider. Defaults expose no
/// execution path; registering a ribbon button alone cannot enable geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtrudeCapabilities {
    pub new_body: bool,
    pub direction: bool,
    pub start_offset: bool,
    pub two_sided: bool,
    pub symmetric: bool,
    pub taper: bool,
    pub to_plane: bool,
    pub to_face: bool,
}
impl ExtrudeCapabilities {
    /// The first real vertical slice: independent bodies, distance extents,
    /// direction and offset. Boolean targets and associative limits remain
    /// unavailable until their preview and persistent rebuild paths exist.
    pub fn distance_bodies() -> Self {
        Self {
            new_body: true,
            direction: true,
            start_offset: true,
            two_sided: true,
            symmetric: true,
            taper: true,
            to_plane: true,
            to_face: true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ExtrudeEvent {
    Changed(FormStamp),
    Preview {
        stamp: FormStamp,
        request: Box<ExtrudeRequest>,
    },
    Apply(FormStamp),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtentChoice {
    OneSided,
    TwoSided,
    Symmetric,
}
impl ExtentChoice {
    fn label(self) -> &'static str {
        match self {
            Self::OneSided => "One side",
            Self::TwoSided => "Two sides",
            Self::Symmetric => "Symmetric · total distance",
        }
    }
}

struct Draft {
    request: ExtrudeRequest,
    stamp: FormStamp,
    elevation: DimensionField,
    azimuth: DimensionField,
    reverse: bool,
    offset: DimensionField,
    taper: DimensionField,
    extent: ExtentChoice,
    forward: EndCondition,
    backward: EndCondition,
    total: DimensionField,
    picker: EndPicker,
}
impl Draft {
    fn new(request: ExtrudeRequest) -> Result<Self, String> {
        let parameters = &request.parameters;
        let fallback = || Termination::Distance {
            distance: millimeters(10.),
        };
        let (extent, forward, backward, total) = match &parameters.extent {
            Extent::OneSided { end } => (
                ExtentChoice::OneSided,
                end.clone(),
                fallback(),
                millimeters(10.),
            ),
            Extent::TwoSided { forward, backward } => (
                ExtentChoice::TwoSided,
                forward.clone(),
                backward.clone(),
                millimeters(10.),
            ),
            Extent::Symmetric { total } => (
                ExtentChoice::Symmetric,
                fallback(),
                fallback(),
                total.clone(),
            ),
        };
        Ok(Self {
            stamp: FormStamp::new(),
            elevation: DimensionField::from_dimension(&parameters.direction.elevation, "deg")?,
            azimuth: DimensionField::from_dimension(&parameters.direction.azimuth, "deg")?,
            reverse: parameters.direction.reverse,
            offset: DimensionField::from_dimension(&parameters.start_offset, "mm")?,
            taper: DimensionField::from_dimension(&parameters.taper, "deg")?,
            extent,
            forward: EndCondition::new(&forward, &request.faces)?,
            backward: EndCondition::new(&backward, &request.faces)?,
            picker: EndPicker::default(),
            total: DimensionField::from_dimension(&total, "mm")?,
            request,
        })
    }
    fn request(
        &self,
        document: &Document,
        capabilities: ExtrudeCapabilities,
    ) -> Result<ExtrudeRequest, String> {
        if !capabilities.new_body {
            return Err("The native extrusion provider is unavailable".into());
        }
        if self.request.operation != Operation::NewBody
            || !self.request.targets.is_empty()
            || self.request.keep_tools
        {
            return Err(
                "Extrusion profile controls require an independent sweep request; operation controls are configured separately".into(),
            );
        }
        if self.request.outputs.is_empty() {
            return Err("Select at least one exact closed profile".into());
        }
        if self.picker.active.is_some() {
            return Err("Finish or cancel end-target selection before Preview".into());
        }
        let mut selected_faces = vec![];
        let mut end = |field: &EndCondition| -> Result<Termination, String> {
            let (termination, face) =
                field.resolve(document, capabilities.to_plane, capabilities.to_face)?;
            if let Some(face) = face {
                selected_faces.push(face);
            }
            Ok(termination)
        };
        let parameters = ExtrudeParameters {
            direction: Direction {
                elevation: self.elevation.resolve(document)?,
                azimuth: self.azimuth.resolve(document)?,
                reverse: self.reverse,
            },
            start_offset: self.offset.resolve(document)?,
            taper: self.taper.resolve(document)?,
            extent: match self.extent {
                ExtentChoice::OneSided => Extent::OneSided {
                    end: end(&self.forward)?,
                },
                ExtentChoice::TwoSided if capabilities.two_sided => Extent::TwoSided {
                    forward: end(&self.forward)?,
                    backward: end(&self.backward)?,
                },
                ExtentChoice::Symmetric if capabilities.symmetric => Extent::Symmetric {
                    total: self.total.resolve(document)?,
                },
                _ => return Err("This extent is unavailable in the native provider".into()),
            },
        };
        if !capabilities.direction
            && (parameters.direction.elevation != degrees(90.)
                || parameters.direction.azimuth != degrees(0.)
                || parameters.direction.reverse)
        {
            return Err("Custom extrusion direction is unavailable in the native provider".into());
        }
        if !capabilities.start_offset && parameters.start_offset != millimeters(0.) {
            return Err("Start offset is unavailable in the native provider".into());
        }
        if !capabilities.taper && parameters.taper != degrees(0.) {
            return Err("Taper is unavailable in the native provider".into());
        }
        if !matches!(parameters.extent, Extent::Symmetric { .. })
            && selected_associative(&parameters.extent)
            && matches!(&parameters.taper,Dimension::Literal{value,..} if *value!=0.)
        {
            return Err("Nonzero taper with To plane / To face is not supported; use distance end conditions".into());
        }
        let mut request = self.request.clone();
        request.faces = ordered_faces(&self.request.faces, selected_faces)?;
        request.parameters = parameters;
        Ok(request)
    }
    fn remember(&mut self, request: &ExtrudeRequest) {
        self.elevation
            .remember(&request.parameters.direction.elevation);
        self.azimuth.remember(&request.parameters.direction.azimuth);
        self.offset.remember(&request.parameters.start_offset);
        self.taper.remember(&request.parameters.taper);
        match &request.parameters.extent {
            Extent::OneSided { end } => self.forward.remember(end),
            Extent::TwoSided { forward, backward } => {
                self.forward.remember(forward);
                self.backward.remember(backward);
            }
            Extent::Symmetric { total } => self.total.remember(total),
        }
        self.request = request.clone();
    }
}

#[derive(Default)]
pub struct ExtrudePanel {
    draft: Option<Draft>,
    error: Option<String>,
    picking_available: bool,
}
impl ExtrudePanel {
    /// Called once when entering the tool, never from painting. Each selected
    /// exact region receives independent persistent feature/body identities.
    pub fn begin(
        &mut self,
        sketch: EntityId,
        regions: Vec<RegionIntent>,
    ) -> Result<FormStamp, String> {
        self.begin_request(ExtrudeRequest::new(
            sketch,
            regions,
            ExtrudeParameters::default(),
        ))
    }
    /// Reopen a source-owned request while preserving its output identities.
    /// Unsupported associative requests fail explicitly instead of becoming a
    /// distance extrusion and silently losing their references.
    pub fn begin_request(&mut self, request: ExtrudeRequest) -> Result<FormStamp, String> {
        if request.outputs.is_empty() {
            return Err("Select at least one exact closed profile".into());
        }
        if request.operation != Operation::NewBody
            || !request.targets.is_empty()
            || request.keep_tools
        {
            return Err("This form currently edits independent new-body extrusions only; the original request was preserved".into());
        }
        let output = &request.outputs[0];
        let mut faces = request.faces.clone();
        let profile = match &output.face {
            Some(face) => {
                faces.push(face.clone());
                multicad_core::model::ModelProfile::Face(multicad_core::model::FaceProfile {
                    face: face.intent.id,
                })
            }
            None => multicad_core::model::ProfileSource {
                sketch: request.sketch,
                region: output.region.clone(),
            }
            .into(),
        };
        multicad_core::model::ExtrudeDefinition {
            profile,
            output: multicad_core::model::BodyOutput { body: output.body },
            parameters: request.parameters.clone(),
            operation: request.operation,
            targets: request.targets.clone(),
            keep_tools: request.keep_tools,
        }
        .inputs(&faces)
        .map_err(|error| error.to_string())?;
        let draft = Draft::new(request)?;
        let stamp = draft.stamp;
        self.draft = Some(draft);
        self.error = None;
        Ok(stamp)
    }
    pub fn active(&self) -> bool {
        self.draft.is_some()
    }
    /// The signed one-sided distance the viewport arrow edits, in mm. Other
    /// extents (two-sided, symmetric, To plane / To face) keep their Inspector
    /// controls and show no arrow.
    pub(crate) fn gizmo_distance(&self, document: &Document) -> Option<Result<f64, String>> {
        let draft = self.draft.as_ref()?;
        if draft.extent != ExtentChoice::OneSided
            || draft.forward.kind != super::termination::EndKind::Distance
        {
            return None;
        }
        Some(millimeter_value(document, &draft.forward.distance))
    }
    #[cfg(test)]
    pub(crate) fn set_test_taper(&mut self, degrees: f64) {
        if let Some(draft) = &mut self.draft {
            draft.taper.set_literal(degrees, "deg");
            draft.stamp.changed();
        }
    }
    /// Write a gizmo or typed value into the same Inspector field so preview,
    /// Inspector and commit always share one source of truth.
    pub(crate) fn set_gizmo_distance(&mut self, millimeters: f64) {
        if let Some(draft) = &mut self.draft {
            draft.forward.distance.set_literal(millimeters, "mm");
            draft.stamp.changed();
        }
        self.error = None;
    }
    /// Positive sweep direction in sketch U,V,N (ignoring the distance sign)
    /// and the start offset along N, both resolved from the current fields.
    pub(crate) fn gizmo_axis(&self, document: &Document) -> Result<([f64; 3], f64), String> {
        let draft = self.draft.as_ref().ok_or("Start Extrude first")?;
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        let number = |field: &DimensionField, unit| {
            field
                .resolve(document)?
                .value_in(unit, Some(&parameters))
                .map_err(|e| e.to_string())
        };
        let elevation = number(&draft.elevation, multicad_core::parameters::Unit::Radian)?;
        let azimuth = number(&draft.azimuth, multicad_core::parameters::Unit::Radian)?;
        let offset = number(&draft.offset, multicad_core::parameters::Unit::Millimeter)?;
        let sign = if draft.reverse { -1. } else { 1. };
        Ok((
            [
                elevation.cos() * azimuth.cos() * sign,
                elevation.cos() * azimuth.sin() * sign,
                elevation.sin() * sign,
            ],
            offset,
        ))
    }
    pub(crate) fn changed(&mut self) {
        if let Some(draft) = &mut self.draft {
            draft.stamp.changed();
        }
        self.error = None;
    }
    pub(crate) fn set_picking_available(&mut self, available: bool) {
        self.picking_available = available;
        if !available && let Some(draft) = &mut self.draft {
            draft.picker.cancel();
        }
    }
    pub fn stamp(&self) -> Option<FormStamp> {
        self.draft.as_ref().map(|draft| draft.stamp)
    }
    pub fn cancel(&mut self) {
        self.draft = None;
        self.error = None;
    }
    pub fn termination_picking(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| draft.picker.active.is_some())
    }
    pub fn termination_highlight(&self) -> Option<crate::viewport::Selection> {
        self.draft
            .as_ref()
            .and_then(|draft| draft.picker.highlight())
    }
    pub fn termination_viewport(
        &mut self,
        document: &Document,
        revision: u64,
        source_hash: ContentHash,
        view: TerminationViewport<'_>,
    ) -> bool {
        if !self.picking_available {
            return false;
        }
        let Some(draft) = self.draft.as_mut() else {
            return false;
        };
        if let Some((side, choice)) = draft.picker.viewport(
            document,
            revision,
            source_hash,
            view,
            &draft
                .request
                .outputs
                .iter()
                .map(|o| o.feature)
                .collect::<Vec<_>>(),
        ) {
            match side {
                EndSide::Forward => draft.forward.accept(choice),
                EndSide::Backward => draft.backward.accept(choice),
            }
            draft.stamp.changed();
            self.error = None;
            return true;
        }
        false
    }
    pub fn request(
        &self,
        document: &Document,
        capabilities: ExtrudeCapabilities,
    ) -> Result<ExtrudeRequest, String> {
        self.draft
            .as_ref()
            .ok_or("Start Extrude first")?
            .request(document, capabilities)
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        capabilities: ExtrudeCapabilities,
        preview: PreviewState<'_>,
    ) -> Option<ExtrudeEvent> {
        let draft = self.draft.as_mut()?;
        controls::inspector_layout(ui);
        ui.strong("Extrude");
        ui.label(format!("{} selected profiles", draft.request.outputs.len()));
        ui.small("Preview checks native solids. Apply changes the document only after that preview succeeds.");
        ui.separator();
        let mut changed = false;
        ui.add_enabled_ui(capabilities.direction, |ui| {
            changed |= draft.elevation.show(
                ui,
                "Angle from sketch plane",
                "90° is perpendicular to the sketch plane. This changes direction, not wall taper.",
            );
            changed |= draft.azimuth.show(
                ui,
                "Direction around plane",
                "Azimuth rotates the tilted extrusion direction around the sketch normal",
            );
            changed |= ui
                .checkbox(&mut draft.reverse, "Reverse direction")
                .on_hover_text("Reverse the extrusion vector without changing draft/taper")
                .changed();
        });
        if !capabilities.direction {
            ui.small("Custom direction is unavailable in the native provider.");
        }
        ui.add_enabled_ui(capabilities.start_offset, |ui| {
            changed |= draft.offset.show(ui, "Start offset", "Move the start along the sketch normal. Distance is then measured along the sweep direction; for a perpendicular sweep, 10 mm offset plus 100 mm distance spans 10–110 mm.");
        });
        ui.separator();
        ui.label("Extent");
        let previous_extent = draft.extent;
        controls::inspector_combo(ui, "extrude-extent")
            .selected_text(draft.extent.label())
            .show_ui(ui, |ui| {
                for (extent, enabled) in [
                    (ExtentChoice::OneSided, true),
                    (ExtentChoice::TwoSided, capabilities.two_sided),
                    (ExtentChoice::Symmetric, capabilities.symmetric),
                ] {
                    ui.add_enabled_ui(enabled, |ui| {
                        changed |= ui
                            .selectable_value(&mut draft.extent, extent, extent.label())
                            .changed();
                    })
                    .response
                    .on_disabled_hover_text("This extent is unavailable in the native provider");
                }
            });
        if previous_extent != draft.extent {
            draft.picker.cancel();
        }
        match draft.extent {
            ExtentChoice::OneSided => {
                changed |= show_end(
                    ui,
                    document,
                    ("End condition", EndSide::Forward),
                    &mut draft.forward,
                    &mut draft.picker,
                    capabilities,
                    self.picking_available,
                )
            }
            ExtentChoice::TwoSided => {
                changed |= show_end(
                    ui,
                    document,
                    ("Forward end", EndSide::Forward),
                    &mut draft.forward,
                    &mut draft.picker,
                    capabilities,
                    self.picking_available,
                );
                ui.separator();
                changed |= show_end(
                    ui,
                    document,
                    ("Backward end", EndSide::Backward),
                    &mut draft.backward,
                    &mut draft.picker,
                    capabilities,
                    self.picking_available,
                );
            }
            ExtentChoice::Symmetric => {
                draft.picker.cancel();
                changed|=draft.total.show(ui,"Total distance","Total span centered at the offset start; each side receives half this distance");
                ui.small("Half of the total distance on each side of the offset start.");
            }
        }
        if let Some(message) = draft.picker.message() {
            ui.colored_label(egui::Color32::LIGHT_YELLOW, message);
        }
        ui.add_enabled_ui(capabilities.taper, |ui| {
            changed |= draft.taper.show(ui, "Taper angle", "Positive taper expands material and shrinks holes by distance × tan(angle), independently along each side. Exact shared-vertex convex polygons and circles only; arcs, splines, topology changes and plane/face limits with nonzero taper are unsupported.");
        });
        if !capabilities.taper {
            ui.small("Taper is unavailable in the native provider; the section remains constant.");
        } else {
            ui.small("Positive widens material; negative narrows. Preview rejects unsupported profiles or collapsing holes.");
        }
        ui.separator();
        if !capabilities.new_body {
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                "The native extrusion provider is unavailable.",
            );
        }
        form::preview_status(ui, draft.stamp, preview);
        let validation = draft.request(document, capabilities);
        let valid = validation.is_ok();
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let requested = form::actions(ui, valid, changed, draft.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        self.finish_frame(changed, requested, document, capabilities, preview)
    }
    fn finish_frame(
        &mut self,
        changed: bool,
        requested: Option<Action>,
        document: &Document,
        capabilities: ExtrudeCapabilities,
        preview: PreviewState<'_>,
    ) -> Option<ExtrudeEvent> {
        if requested == Some(Action::Cancel) {
            self.cancel();
            return Some(ExtrudeEvent::Cancel);
        }
        let draft = self.draft.as_mut()?;
        if changed {
            draft.stamp.changed();
            self.error = None;
            if requested != Some(Action::Preview) {
                return Some(ExtrudeEvent::Changed(draft.stamp));
            }
        }
        match requested {
            Some(Action::Preview) => match draft.request(document, capabilities) {
                Ok(request) => {
                    draft.remember(&request);
                    self.error = None;
                    Some(ExtrudeEvent::Preview {
                        stamp: draft.stamp,
                        request: Box::new(request),
                    })
                }
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(Action::Apply)
                if preview.ready(draft.stamp) && draft.request(document, capabilities).is_ok() =>
            {
                Some(ExtrudeEvent::Apply(draft.stamp))
            }
            _ => None,
        }
    }
}
fn selected_associative(extent: &Extent) -> bool {
    match extent {
        Extent::OneSided { end } => !matches!(end, Termination::Distance { .. }),
        Extent::TwoSided { forward, backward } => {
            !matches!(forward, Termination::Distance { .. })
                || !matches!(backward, Termination::Distance { .. })
        }
        Extent::Symmetric { .. } => false,
    }
}
fn ordered_faces(
    previous: &[FaceInput],
    selected: Vec<FaceInput>,
) -> Result<Vec<FaceInput>, String> {
    let mut unique = vec![];
    for face in selected {
        if let Some(old) = unique
            .iter()
            .find(|old: &&FaceInput| old.intent.id == face.intent.id)
        {
            if old != &face {
                return Err(
                    "The same end reference has conflicting selectors; choose the target again"
                        .into(),
                );
            }
        } else {
            unique.push(face);
        }
    }
    let mut result = vec![];
    for old in previous {
        if let Some(face) = unique.iter().find(|face| face.intent.id == old.intent.id) {
            result.push(face.clone());
        }
    }
    for face in unique {
        if !result.iter().any(|old| old.intent.id == face.intent.id) {
            result.push(face);
        }
    }
    Ok(result)
}
fn show_end(
    ui: &mut egui::Ui,
    document: &Document,
    (label, side): (&str, EndSide),
    field: &mut EndCondition,
    picker: &mut EndPicker,
    capabilities: ExtrudeCapabilities,
    can_pick: bool,
) -> bool {
    let active = picker.active.is_some_and(|(current, _)| current == side);
    let action = field.show(ui, document, label, capabilities, active, can_pick);
    if action.changed && active || action.cancel {
        picker.cancel();
    }
    if let Some(kind) = action.pick {
        picker.begin(side, kind);
    }
    action.changed || action.pick.is_some() || action.cancel
}
pub(crate) fn millimeter_value(document: &Document, field: &DimensionField) -> Result<f64, String> {
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
    field
        .resolve(document)?
        .value_in(
            multicad_core::parameters::Unit::Millimeter,
            Some(&parameters),
        )
        .map_err(|e| e.to_string())
}
fn millimeters(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}
fn degrees(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "deg".into(),
    }
}

#[cfg(test)]
mod tests;
