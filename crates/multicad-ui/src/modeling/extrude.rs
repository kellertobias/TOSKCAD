use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction as Action},
};
use crate::{controls, egui};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{Direction, Extent, ExtrudeParameters, ExtrudeRequest, Operation, Termination},
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
            taper: false,
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
    forward: DimensionField,
    backward: DimensionField,
    total: DimensionField,
}
impl Draft {
    fn new(request: ExtrudeRequest) -> Result<Self, String> {
        let parameters = &request.parameters;
        let (extent, forward, backward, total) = match &parameters.extent {
            Extent::OneSided { end } => (
                ExtentChoice::OneSided,
                distance(end)?,
                millimeters(10.),
                millimeters(10.),
            ),
            Extent::TwoSided { forward, backward } => (
                ExtentChoice::TwoSided,
                distance(forward)?,
                distance(backward)?,
                millimeters(10.),
            ),
            Extent::Symmetric { total } => (
                ExtentChoice::Symmetric,
                millimeters(10.),
                millimeters(10.),
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
            forward: DimensionField::from_dimension(&forward, "mm")?,
            backward: DimensionField::from_dimension(&backward, "mm")?,
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
            || !self.request.faces.is_empty()
        {
            return Err(
                "Boolean targets and associative limits are not available in this form yet".into(),
            );
        }
        if self.request.outputs.is_empty() {
            return Err("Select at least one exact closed profile".into());
        }
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
                    end: Termination::Distance {
                        distance: self.forward.resolve(document)?,
                    },
                },
                ExtentChoice::TwoSided if capabilities.two_sided => Extent::TwoSided {
                    forward: Termination::Distance {
                        distance: self.forward.resolve(document)?,
                    },
                    backward: Termination::Distance {
                        distance: self.backward.resolve(document)?,
                    },
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
        let mut request = self.request.clone();
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
            Extent::OneSided {
                end: Termination::Distance { distance },
            } => self.forward.remember(distance),
            Extent::TwoSided {
                forward: Termination::Distance { distance: forward },
                backward: Termination::Distance { distance: backward },
            } => {
                self.forward.remember(forward);
                self.backward.remember(backward);
            }
            Extent::Symmetric { total } => self.total.remember(total),
            _ => {}
        }
    }
}

#[derive(Default)]
pub struct ExtrudePanel {
    draft: Option<Draft>,
    error: Option<String>,
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
            || !request.faces.is_empty()
        {
            return Err("This form currently edits independent new-body extrusions only; the original request was preserved".into());
        }
        let draft = Draft::new(request)?;
        let stamp = draft.stamp;
        self.draft = Some(draft);
        self.error = None;
        Ok(stamp)
    }
    pub fn active(&self) -> bool {
        self.draft.is_some()
    }
    pub fn stamp(&self) -> Option<FormStamp> {
        self.draft.as_ref().map(|draft| draft.stamp)
    }
    pub fn cancel(&mut self) {
        self.draft = None;
        self.error = None;
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
        ui.label(format!(
            "{} selected profiles · one independent body per profile",
            draft.request.outputs.len()
        ));
        ui.small("Preview checks native solids. Apply changes the document only after that preview succeeds.");
        ui.separator();
        ui.label("Operation");
        controls::inspector_combo(ui, "extrude-operation")
            .selected_text("New body")
            .show_ui(ui, |ui| {
                ui.label("New body");
                for label in ["Join", "Cut", "Intersect", "Split"] {
                    ui.add_enabled(false, egui::Button::new(label))
                        .on_disabled_hover_text(
                            "Explicit Boolean targets need the native preview and rebuild provider",
                        );
                }
            });
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
        ui.label("End condition");
        controls::inspector_combo(ui, "extrude-limit")
            .selected_text("Distance")
            .show_ui(ui, |ui| {
                ui.label("Distance");
                for label in ["To plane", "To face"] {
                    ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text("Associative limit picking needs the native reference and preview provider");
                }
            });
        match draft.extent {
            ExtentChoice::OneSided => {
                changed |= draft.forward.show(
                    ui,
                    "Distance",
                    "Distance from the offset start, not an absolute endpoint",
                )
            }
            ExtentChoice::TwoSided => {
                changed |= draft.forward.show(ui, "Forward distance", "Independent positive distance along the chosen direction from the offset start");
                changed |= draft.backward.show(ui, "Backward distance", "Independent positive distance opposite the chosen direction from the offset start");
            }
            ExtentChoice::Symmetric => {
                changed |= draft.total.show(ui, "Total distance", "Total span centered at the offset start; each side receives half this distance");
                ui.small("Half of the total distance on each side of the offset start.");
            }
        }
        ui.add_enabled_ui(capabilities.taper, |ui| {
            changed |= draft.taper.show(ui, "Taper angle", "Draft changes the cross-section size. Zero taper keeps a constant section and is separate from extrusion direction.");
        });
        if !capabilities.taper {
            ui.small("Taper is unavailable in the native provider; the section remains constant.");
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
fn distance(termination: &Termination) -> Result<Dimension, String> {
    match termination {
        Termination::Distance { distance } => Ok(distance.clone()),
        _ => Err("Associative limits require the native picking provider; the stored reference was preserved".into()),
    }
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
