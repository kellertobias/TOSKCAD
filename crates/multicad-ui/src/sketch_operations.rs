//! Source-neutral operation forms. The host owns numerical previews and commits.
use crate::{
    egui::{self, Color32},
    icons::Icon,
};
use multicad_core::{
    constraint_sketch::{CurveKind, source::Source},
    document::EntityId,
    sketch_pattern::{MAX_PATTERN_INSTANCES, Pattern, PatternOptions, PatternRequest},
    sketch_transform::{Transform, TransformOptions, TransformRequest},
    sketch_trim::{
        Contact, Endpoint, ExtendLine, ExtendSpline, TrimCurve, TrimPolicy, TrimRequest,
    },
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationTool {
    Move,
    Rotate,
    Mirror,
    LinearPattern,
    CircularPattern,
    Trim,
    Extend,
}
impl OperationTool {
    pub const ALL: [Self; 7] = [
        Self::Move,
        Self::Rotate,
        Self::Mirror,
        Self::LinearPattern,
        Self::CircularPattern,
        Self::Trim,
        Self::Extend,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Move => "sketch-move",
            Self::Rotate => "sketch-rotate",
            Self::Mirror => "sketch-mirror",
            Self::LinearPattern => "sketch-linear-pattern",
            Self::CircularPattern => "sketch-circular-pattern",
            Self::Trim => "sketch-trim",
            Self::Extend => "sketch-extend",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Mirror => "Mirror",
            Self::LinearPattern => "Linear pattern",
            Self::CircularPattern => "Circular pattern",
            Self::Trim => "Trim",
            Self::Extend => "Extend",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Move => Icon::Transform,
            Self::Rotate => Icon::Rotate,
            Self::Mirror => Icon::Mirror,
            Self::LinearPattern => Icon::LinearPattern,
            Self::CircularPattern => Icon::CircularPattern,
            Self::Trim => Icon::Trim,
            Self::Extend => Icon::Extend,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Move => &["smove", "move-sketch"],
            Self::Rotate => &["rotate", "srotate"],
            Self::Mirror => &["mirror", "smirror"],
            Self::LinearPattern => &["linear-pattern", "array"],
            Self::CircularPattern => &["circular-pattern", "polar-array"],
            Self::Trim => &["trim"],
            Self::Extend => &["extend"],
        }
    }
    pub fn usage(self) -> String {
        format!(
            "Usage: {} (opens {} controls; enter values in the Inspector, then Preview)",
            self.id(),
            self.label()
        )
    }
    fn curve_operation(self) -> bool {
        matches!(self, Self::Trim | Self::Extend)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum OperationRequest {
    Transform(TransformRequest),
    Pattern(PatternRequest),
    Trim(TrimRequest, TrimPolicy),
}
/// A viewport click chooses an authored curve or a point in its sketch plane.
/// These picks select query inputs; they never establish exact intersections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurvePickLane {
    Target,
    Span,
    Boundaries,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MirrorAxis {
    Horizontal,
    Vertical,
    Angle,
}
/// Fields are local to the active form. No canonical entities or constraints are
/// changed here; even a successful request still needs a solver-owned preview.
pub struct Panel {
    tool: Option<OperationTool>,
    delta_mm: [f64; 2],
    center_mm: [f64; 2],
    angle_deg: f64,
    mirror_axis: MirrorAxis,
    count: usize,
    expand_connected: bool,
    target: Option<EntityId>,
    pick_lane: CurvePickLane,
    boundaries: BTreeSet<EntityId>,
    pick_mm: [f64; 2],
    endpoint: Endpoint,
    max_length_mm: f64,
    extension_contacts: Option<(ExtendSpline, Vec<Contact>)>,
    extension_contact: Option<usize>,
    selection_seeded: bool,
    error: Option<String>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            tool: None,
            delta_mm: [10., 0.],
            center_mm: [0.; 2],
            angle_deg: 45.,
            mirror_axis: MirrorAxis::Vertical,
            count: 3,
            expand_connected: false,
            target: None,
            pick_lane: CurvePickLane::Target,
            boundaries: BTreeSet::new(),
            pick_mm: [0.; 2],
            endpoint: Endpoint::End,
            max_length_mm: 100.,
            extension_contacts: None,
            extension_contact: None,
            selection_seeded: false,
            error: None,
        }
    }
}
impl Panel {
    pub fn begin(&mut self, tool: OperationTool) {
        *self = Self {
            tool: Some(tool),
            ..Default::default()
        };
    }
    pub fn tool(&self) -> Option<OperationTool> {
        self.tool
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn curve_pick_lane(&self) -> Option<CurvePickLane> {
        self.tool
            .filter(|tool| tool.curve_operation())
            .map(|_| self.pick_lane)
    }
    pub fn curve_target(&self) -> Option<EntityId> {
        self.target
    }
    pub fn curve_boundaries(&self) -> &BTreeSet<EntityId> {
        &self.boundaries
    }
    pub fn curve_selection(&self) -> BTreeSet<EntityId> {
        self.boundaries.iter().copied().chain(self.target).collect()
    }
    pub fn clear_extension_contacts(&mut self) {
        self.extension_contacts = None;
        self.extension_contact = None;
    }
    /// The host must independently validate the query's source/session stamp.
    /// A matching form is necessary, but never authorizes a model mutation.
    pub fn set_extension_contacts(
        &mut self,
        request: &ExtendSpline,
        contacts: Vec<Contact>,
    ) -> Result<(), String> {
        let mut base = request.clone();
        base.contact_index = None;
        if self.tool != Some(OperationTool::Extend) || self.extension_spec().as_ref() != Some(&base)
        {
            return Err("Extension choices belong to different operation controls".into());
        }
        if contacts.len() > 256
            || contacts.iter().any(|contact| {
                !contact.parameter.is_finite()
                    || !(0. ..=1.).contains(&contact.parameter)
                    || contact.point_mm.iter().any(|v| !v.is_finite())
                    || contact.boundaries.is_empty()
                    || !contact.boundaries.is_subset(&base.boundaries)
            })
            || contacts
                .windows(2)
                .any(|pair| pair[1].parameter - pair[0].parameter <= 1e-12)
        {
            return Err("Invalid or unordered exact extension contacts".into());
        }
        self.extension_contacts = Some((base, contacts));
        self.extension_contact = None;
        self.error = None;
        Ok(())
    }
    fn extension_spec(&self) -> Option<ExtendSpline> {
        Some(ExtendSpline {
            target: self.target?,
            endpoint: self.endpoint,
            boundaries: self.boundaries.clone(),
            max_length_mm: self.max_length_mm,
            contact_index: None,
        })
    }
    pub fn set_curve_pick(
        &mut self,
        source: &Source,
        curve: EntityId,
        point_mm: [f64; 2],
        toggle: bool,
    ) -> Result<(), String> {
        let tool = self
            .tool
            .filter(|tool| tool.curve_operation())
            .ok_or("Choose Trim or Extend before picking curves")?;
        if point_mm.iter().any(|value| !value.is_finite()) {
            return Err("Pick coordinates must be finite".into());
        }
        let kind = &source
            .sketch
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .ok_or("The picked curve no longer exists")?
            .kind;
        match self.pick_lane {
            CurvePickLane::Target => {
                if !target_supported(tool, kind) {
                    return Err("Choose a supported open target curve for this operation".into());
                }
                self.target = Some(curve);
                self.boundaries.remove(&curve);
                self.pick_mm = point_mm;
                self.pick_lane = CurvePickLane::Boundaries;
            }
            CurvePickLane::Span => {
                if tool != OperationTool::Trim || self.target != Some(curve) {
                    return Err("Pick the span on the selected Trim target".into());
                }
                self.pick_mm = point_mm;
                self.pick_lane = CurvePickLane::Boundaries;
            }
            CurvePickLane::Boundaries => {
                if self.target.is_none() {
                    return Err("Choose the target curve before its boundaries".into());
                }
                if self.target == Some(curve) {
                    return Err("A curve cannot be its own trim or extension boundary".into());
                }
                if !toggle {
                    self.boundaries.clear();
                }
                if !self.boundaries.insert(curve) && toggle {
                    self.boundaries.remove(&curve);
                }
            }
        }
        self.clear_extension_contacts();
        self.selection_seeded = true;
        self.error = None;
        Ok(())
    }
    fn seed_target_selection(&mut self, source: &Source, selected: &[EntityId]) {
        if self.selection_seeded {
            return;
        }
        self.selection_seeded = true;
        let tool = self.tool.unwrap_or(OperationTool::Trim);
        let targets: Vec<_> = source
            .sketch
            .curves
            .iter()
            .filter(|curve| selected.contains(&curve.id) && target_supported(tool, &curve.kind))
            .map(|curve| curve.id)
            .collect();
        if targets.len() == 1 {
            self.target = Some(targets[0]);
            self.pick_lane = CurvePickLane::Boundaries;
        }
        self.boundaries.extend(
            source
                .sketch
                .curves
                .iter()
                .filter(|curve| selected.contains(&curve.id) && Some(curve.id) != self.target)
                .map(|curve| curve.id),
        );
    }
    pub fn request(
        &self,
        source: &Source,
        selected: &[EntityId],
    ) -> Result<OperationRequest, String> {
        let tool = self.tool.ok_or("Choose a sketch edit tool")?;
        if tool.curve_operation() {
            return self.curve_request(source, tool);
        }
        let selection: BTreeSet<_> = selected.iter().copied().collect();
        if selection.is_empty() {
            return Err("Select sketch points or curves before previewing this operation".into());
        }
        if selection.iter().any(|id| {
            !source.sketch.points.iter().any(|point| point.id == *id)
                && !source.sketch.curves.iter().any(|curve| curve.id == *id)
        }) {
            return Err("Selection contains an unavailable sketch point or curve".into());
        }
        if self
            .delta_mm
            .iter()
            .chain(&self.center_mm)
            .chain([&self.angle_deg])
            .any(|value| !value.is_finite())
        {
            return Err("Operation values must be finite".into());
        }
        let transform = match tool {
            OperationTool::Move => Some(Transform::Translate {
                delta_mm: self.delta_mm,
            }),
            OperationTool::Rotate => Some(Transform::Rotate {
                center_mm: self.center_mm,
                angle_rad: self.angle_deg.to_radians(),
            }),
            OperationTool::Mirror => Some(Transform::Mirror {
                point_mm: self.center_mm,
                direction: match self.mirror_axis {
                    MirrorAxis::Horizontal => [1., 0.],
                    MirrorAxis::Vertical => [0., 1.],
                    MirrorAxis::Angle => {
                        let angle = self.angle_deg.to_radians();
                        [angle.cos(), angle.sin()]
                    }
                },
            }),
            _ => None,
        };
        if let Some(transform) = transform {
            return Ok(OperationRequest::Transform(TransformRequest {
                selection,
                transform,
                options: TransformOptions {
                    expand_connected: self.expand_connected,
                    ..Default::default()
                },
            }));
        }
        if !(2..=MAX_PATTERN_INSTANCES).contains(&self.count) {
            return Err(format!(
                "Pattern count must be 2–{MAX_PATTERN_INSTANCES}, including the original"
            ));
        }
        let pattern = match tool {
            OperationTool::LinearPattern => Pattern::Linear {
                step_mm: self.delta_mm,
                count: self.count,
            },
            OperationTool::CircularPattern => Pattern::Circular {
                center_mm: self.center_mm,
                step_angle_rad: self.angle_deg.to_radians(),
                count: self.count,
            },
            _ => return Err("Choose a transform or pattern operation".into()),
        };
        Ok(OperationRequest::Pattern(PatternRequest {
            selection,
            pattern,
            options: PatternOptions {
                expand_connected: self.expand_connected,
            },
        }))
    }
    fn curve_request(
        &self,
        source: &Source,
        tool: OperationTool,
    ) -> Result<OperationRequest, String> {
        let target = self.target.ok_or("Choose the curve to trim or extend")?;
        let curve = source
            .sketch
            .curves
            .iter()
            .find(|curve| curve.id == target)
            .ok_or("The target curve no longer exists")?;
        if tool == OperationTool::Extend && matches!(curve.kind, CurveKind::Circle { .. }) {
            return Err("A closed circle has no endpoint to extend; choose a line or arc".into());
        }
        if self.boundaries.is_empty() {
            return Err("Choose at least one boundary curve".into());
        }
        if self.boundaries.contains(&target) {
            return Err("A curve cannot be its own trim or extension boundary".into());
        }
        for id in &self.boundaries {
            source
                .sketch
                .curves
                .iter()
                .find(|curve| curve.id == *id)
                .ok_or("A boundary no longer exists")?;
        }
        let request = match tool {
            OperationTool::Trim => {
                if self.pick_mm.iter().any(|value| !value.is_finite()) {
                    return Err("Pick coordinates must be finite".into());
                }
                TrimRequest::TrimCurve(TrimCurve {
                    target,
                    pick_mm: self.pick_mm,
                    boundaries: self.boundaries.clone(),
                })
            }
            OperationTool::Extend => {
                let request = ExtendLine {
                    target,
                    endpoint: self.endpoint,
                    boundaries: self.boundaries.clone(),
                };
                if matches!(curve.kind, CurveKind::BSpline { .. }) {
                    if !self.max_length_mm.is_finite()
                        || self.max_length_mm <= 0.
                        || self.max_length_mm > 1e6
                    {
                        return Err("Maximum continuation must be finite, greater than zero and at most 1000000 mm".into());
                    }
                    let mut spline = self.extension_spec().ok_or("Choose a spline target")?;
                    if let Some((base, contacts)) = &self.extension_contacts {
                        if *base != spline {
                            return Err(
                                "Operation inputs changed; Preview exact contacts again".into()
                            );
                        }
                        if contacts.is_empty() {
                            return Err(
                                "No exact boundary contact within the maximum continuation".into(),
                            );
                        }
                        if contacts.len() > 1 && self.extension_contact.is_none() {
                            return Err("Choose an exact contact, then Preview again".into());
                        }
                        if self
                            .extension_contact
                            .is_some_and(|index| index >= contacts.len())
                        {
                            return Err("The chosen exact contact is no longer available".into());
                        }
                        spline.contact_index = self.extension_contact;
                    }
                    TrimRequest::ExtendSpline(spline)
                } else if matches!(curve.kind, CurveKind::Arc { .. }) {
                    TrimRequest::ExtendArc(request)
                } else {
                    TrimRequest::Extend(request)
                }
            }
            _ => return Err("Choose Trim or Extend".into()),
        };
        Ok(OperationRequest::Trim(request, TrimPolicy::default()))
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        source: &Source,
        selected: &[EntityId],
        enabled: bool,
    ) -> Option<OperationRequest> {
        let tool = self.tool?;
        ui.strong(tool.label());
        ui.small("All distances use sketch-plane millimeters. Preview validates constraints before Apply changes the document.");
        ui.add_enabled_ui(enabled, |ui| {
            if tool.curve_operation() {
                self.seed_target_selection(source, selected);
                self.curve_controls(ui, source, tool);
            } else {
                ui.label(format!("{} selected sketch entities", selected.len()));
                self.transform_controls(ui, tool);
                if matches!(tool, OperationTool::LinearPattern | OperationTool::CircularPattern) {
                    ui.horizontal(|ui| {
                        ui.label("Instances");
                        ui.add(egui::DragValue::new(&mut self.count).range(2..=MAX_PATTERN_INSTANCES));
                    }).response.on_hover_text("Count includes the original; spacing and angle are increments, not a total span");
                }
                ui.checkbox(&mut self.expand_connected, "Include connected geometry")
                    .on_hover_text("Explicitly permit the solver to expand selection through shared endpoints; affected geometry and constraints are reported in the preview");
            }

        });
        use crate::inspector_actions::{self, Button, Owner};
        let clicked = inspector_actions::row(
            ui,
            Owner::Operation,
            Some(Button::new(
                "Preview",
                enabled,
                "Build a source-neutral numerical preview before Apply",
            )),
            Button::new(
                "Apply",
                false,
                "Preview the sketch operation before applying the checked draft",
            ),
            Button::new(
                "Cancel operation",
                true,
                "Discard these uncommitted operation controls",
            ),
        );
        let preview = clicked.preview;
        if clicked.cancel {
            self.cancel();
            return None;
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if !preview {
            return None;
        }
        match self.request(source, selected) {
            Ok(request) => {
                self.error = None;
                Some(request)
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }
    fn transform_controls(&mut self, ui: &mut egui::Ui, tool: OperationTool) {
        match tool {
            OperationTool::Move | OperationTool::LinearPattern => {
                vector_input(
                    ui,
                    if tool == OperationTool::Move {
                        "Displacement"
                    } else {
                        "Step per instance"
                    },
                    &mut self.delta_mm,
                );
            }
            OperationTool::Rotate | OperationTool::CircularPattern => {
                vector_input(ui, "Center", &mut self.center_mm);
                number_input(
                    ui,
                    if tool == OperationTool::Rotate {
                        "Angle"
                    } else {
                        "Angle per instance"
                    },
                    &mut self.angle_deg,
                    "°",
                );
            }
            OperationTool::Mirror => {
                vector_input(ui, "Point on mirror axis", &mut self.center_mm);
                egui::ComboBox::from_id_salt("mirror-axis")
                    .selected_text(match self.mirror_axis {
                        MirrorAxis::Horizontal => "Horizontal",
                        MirrorAxis::Vertical => "Vertical",
                        MirrorAxis::Angle => "Custom angle",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.mirror_axis,
                            MirrorAxis::Horizontal,
                            "Horizontal",
                        );
                        ui.selectable_value(
                            &mut self.mirror_axis,
                            MirrorAxis::Vertical,
                            "Vertical",
                        );
                        ui.selectable_value(
                            &mut self.mirror_axis,
                            MirrorAxis::Angle,
                            "Custom angle",
                        );
                    })
                    .response
                    .on_hover_text(
                        "Mirror the selected geometry in place about this sketch-plane axis",
                    );
                if self.mirror_axis == MirrorAxis::Angle {
                    number_input(ui, "Axis angle", &mut self.angle_deg, "°");
                }
            }
            OperationTool::Trim | OperationTool::Extend => {}
        }
    }
    fn curve_controls(&mut self, ui: &mut egui::Ui, source: &Source, tool: OperationTool) {
        let prior_spec = self.extension_spec();
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.pick_lane, CurvePickLane::Target, "Pick target")
                .on_hover_text("Click a curve in the viewer to choose the curve to edit");
            ui.add_enabled_ui(self.target.is_some(), |ui| {
                if tool == OperationTool::Trim {
                    ui.selectable_value(&mut self.pick_lane, CurvePickLane::Span, "Pick span")
                        .on_hover_text("Click the selected target in the span to remove");
                }
                ui.selectable_value(
                    &mut self.pick_lane,
                    CurvePickLane::Boundaries,
                    "Pick boundaries",
                )
                .on_hover_text(
                    "Click a boundary curve; Shift-click adds or removes individual boundaries",
                );
            });
        });
        ui.small(match self.pick_lane {
            CurvePickLane::Target => "Click the curve to edit in the viewer.",
            CurvePickLane::Span => "Click the selected target in the span to remove.",
            CurvePickLane::Boundaries => {
                "Click a boundary. Shift-click adds or removes boundaries."
            }
        });
        let prior = self.target;
        egui::ComboBox::from_id_salt("curve-target")
            .selected_text(
                self.target
                    .map(|id| curve_label(source, id))
                    .unwrap_or_else(|| "Choose target curve".into()),
            )
            .show_ui(ui, |ui| {
                for (index, curve) in source.sketch.curves.iter().enumerate() {
                    let supported = target_supported(tool, &curve.kind);
                    ui.add_enabled_ui(supported, |ui| {
                        if ui.selectable_value(&mut self.target, Some(curve.id), curve_label_at(index, &curve.kind))
                            .on_disabled_hover_text("A closed circle has no endpoint to extend").clicked() { ui.close(); }
                    });
                }
            })
            .response
            .on_hover_text(
                "Trim supports lines, arcs, circles and splines. Extend supports open lines, arcs and splines. Preview checks exact intersections",
            );
        if prior != self.target
            && let Some(target) = self.target
        {
            self.boundaries.remove(&target);
            self.pick_lane = CurvePickLane::Boundaries;
        }
        if tool == OperationTool::Trim {
            vector_input(ui, "Point in span to remove", &mut self.pick_mm);
            ui.small("Enter a point on the current solved curve in the span to remove.");
            if self
                .target
                .and_then(|id| source.sketch.curves.iter().find(|curve| curve.id == id))
                .is_some_and(|curve| matches!(curve.kind, CurveKind::Circle { .. }))
            {
                ui.small("A circle needs two distinct intersection contacts. Preview reports missing or ambiguous contacts.");
            }
        } else {
            egui::ComboBox::from_id_salt("extend-endpoint")
                .selected_text(match self.endpoint {
                    Endpoint::Start => "Start endpoint",
                    Endpoint::End => "End endpoint",
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut self.endpoint, Endpoint::Start, "Start endpoint")
                        .clicked()
                    {
                        ui.close();
                    }
                    if ui
                        .selectable_value(&mut self.endpoint, Endpoint::End, "End endpoint")
                        .clicked()
                    {
                        ui.close();
                    }
                });
        }
        ui.horizontal(|ui| {
            ui.label("Boundary curves");
            if ui
                .small_button("Clear")
                .on_hover_text("Remove all boundary choices from this source-neutral form")
                .clicked()
            {
                self.boundaries.clear();
            }
        });
        let boundaries: Vec<_> = source
            .sketch
            .curves
            .iter()
            .enumerate()
            .filter(|(_, curve)| Some(curve.id) != self.target)
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("operation-boundaries")
            .max_height(160.)
            .show_rows(ui, 20., boundaries.len(), |ui, range| {
                for index in range {
                    let (source_index, curve) = boundaries[index];
                    let mut checked = self.boundaries.contains(&curve.id);
                    if ui
                        .checkbox(&mut checked, curve_label_at(source_index, &curve.kind))
                        .on_hover_text("Use this exact line, arc, circle or spline as a boundary")
                        .changed()
                    {
                        if checked {
                            self.boundaries.insert(curve.id);
                        } else {
                            self.boundaries.remove(&curve.id);
                        }
                    }
                }
            });
        if prior_spec != self.extension_spec() {
            self.clear_extension_contacts();
        }
        let spline_extension = tool == OperationTool::Extend
            && self
                .target
                .and_then(|id| source.sketch.curves.iter().find(|curve| curve.id == id))
                .is_some_and(|curve| matches!(curve.kind, CurveKind::BSpline { .. }));
        if spline_extension {
            let prior_length = self.max_length_mm;
            number_input(ui, "Maximum continuation", &mut self.max_length_mm, " mm");
            if prior_length != self.max_length_mm {
                self.clear_extension_contacts();
            }
            ui.small("Keeps the original spline segment exactly and adds a straight continuation along its geometric endpoint tangent. Maximum: 1000000 mm.");
            if let Some((_, contacts)) = &self.extension_contacts {
                if contacts.is_empty() {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "No exact boundary contact within this distance.",
                    );
                } else {
                    ui.label(format!("{} exact boundary contact(s)", contacts.len()));
                    if contacts.len() > 1 {
                        egui::ComboBox::from_id_salt("spline-extension-contact")
                            .selected_text(self.extension_contact.map(|i| format!("Contact {}", i + 1)).unwrap_or_else(|| "Choose exact contact".into()))
                            .show_ui(ui, |ui| {
                                for (index, contact) in contacts.iter().enumerate() {
                                    if ui.selectable_value(&mut self.extension_contact, Some(index), format!("Contact {} · X {:.6} / Y {:.6} mm", index + 1, contact.point_mm[0], contact.point_mm[1])).on_hover_text("Select this exact boundary contact, then Preview to check the resulting spline and constraints").clicked() {
                                        self.error = None;
                                        ui.close();
                                    }
                                }
                            }).response.on_hover_text("Multiple contacts require an explicit choice; no nearest contact is chosen for you");
                    }
                    ui.small(
                        "Preview checks the chosen endpoint and affected constraints before Apply.",
                    );
                }
            } else {
                ui.small("Preview finds exact contacts. If there is more than one, choose a contact and Preview again.");
            }
        }
        ui.small(if tool == OperationTool::Trim { "Trim a picked span against exact line, arc, circle or spline boundaries." } else { "Extend an open curve endpoint to an exact boundary. Missing, tangent or ambiguous contacts keep the source unchanged." });
    }
}
fn target_supported(tool: OperationTool, kind: &CurveKind) -> bool {
    match tool {
        OperationTool::Trim => true,
        OperationTool::Extend => !matches!(kind, CurveKind::Circle { .. }),
        _ => false,
    }
}
fn curve_label(source: &Source, id: EntityId) -> String {
    source
        .sketch
        .curves
        .iter()
        .position(|curve| curve.id == id)
        .map(|index| curve_label_at(index, &source.sketch.curves[index].kind))
        .unwrap_or_else(|| "Missing curve".into())
}
fn curve_label_at(index: usize, kind: &CurveKind) -> String {
    format!(
        "{} {}",
        match kind {
            CurveKind::Line { .. } => "Line",
            CurveKind::Circle { .. } => "Circle",
            CurveKind::Arc { .. } => "Arc",
            CurveKind::BSpline { .. } => "Spline",
        },
        index + 1
    )
}
fn vector_input(ui: &mut egui::Ui, label: &str, value: &mut [f64; 2]) {
    ui.label(label);
    for (index, axis) in ["X / U", "Y / V"].iter().enumerate() {
        number_input(ui, axis, &mut value[index], " mm");
    }
}
fn number_input(ui: &mut egui::Ui, label: &str, value: &mut f64, suffix: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.5).suffix(suffix));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::constraint_sketch::{Curve, Sketch, SplineBasis};
    fn source() -> (Source, EntityId, EntityId) {
        let mut source = Source {
            u: [1., 0., 0.],
            sketch: Sketch::default(),
        };
        let a = source.add_point([0., 0.]);
        let b = source.add_point([20., 0.]);
        let line = source.add_line(a, b);
        let c = source.add_point([5., -10.]);
        let d = source.add_point([5., 10.]);
        let boundary = source.add_line(c, d);
        (source, line, boundary)
    }
    #[test]
    fn transform_forms_build_typed_local_requests_without_mutating_canonical_source() {
        let (source, line, _) = source();
        let before = source.clone();
        let mut panel = Panel::default();
        panel.begin(OperationTool::Move);
        panel.delta_mm = [3., -4.];
        panel.expand_connected = true;
        let OperationRequest::Transform(request) = panel.request(&source, &[line]).unwrap() else {
            panic!("transform")
        };
        assert_eq!(request.selection, BTreeSet::from([line]));
        assert_eq!(
            request.transform,
            Transform::Translate {
                delta_mm: [3., -4.]
            }
        );
        assert!(request.options.expand_connected);
        panel.begin(OperationTool::Rotate);
        panel.center_mm = [2., 3.];
        panel.angle_deg = 90.;
        let OperationRequest::Transform(request) = panel.request(&source, &[line]).unwrap() else {
            panic!("rotate")
        };
        assert_eq!(
            request.transform,
            Transform::Rotate {
                center_mm: [2., 3.],
                angle_rad: std::f64::consts::FRAC_PI_2
            }
        );
        panel.begin(OperationTool::Mirror);
        panel.mirror_axis = MirrorAxis::Horizontal;
        let OperationRequest::Transform(request) = panel.request(&source, &[line]).unwrap() else {
            panic!("mirror")
        };
        assert_eq!(
            request.transform,
            Transform::Mirror {
                point_mm: [0.; 2],
                direction: [1., 0.]
            }
        );
        assert_eq!(source, before);
        panel.cancel();
        assert!(panel.tool().is_none());
        assert!(panel.request(&source, &[line]).is_err());
    }
    #[test]
    fn patterns_keep_count_and_increment_semantics_and_validate_selection() {
        let (source, line, _) = source();
        let mut panel = Panel::default();
        panel.begin(OperationTool::LinearPattern);
        panel.delta_mm = [25., 5.];
        panel.count = 4;
        let OperationRequest::Pattern(request) = panel.request(&source, &[line]).unwrap() else {
            panic!("pattern")
        };
        assert_eq!(
            request.pattern,
            Pattern::Linear {
                step_mm: [25., 5.],
                count: 4
            }
        );
        panel.begin(OperationTool::CircularPattern);
        panel.center_mm = [4., 3.];
        panel.angle_deg = 45.;
        panel.count = 8;
        let OperationRequest::Pattern(request) = panel.request(&source, &[line]).unwrap() else {
            panic!("circular")
        };
        assert_eq!(
            request.pattern,
            Pattern::Circular {
                center_mm: [4., 3.],
                step_angle_rad: std::f64::consts::FRAC_PI_4,
                count: 8
            }
        );
        panel.count = 1;
        assert!(panel.request(&source, &[line]).is_err());
        panel.count = 3;
        assert!(panel.request(&source, &[]).is_err());
        assert!(panel.request(&source, &[EntityId::new()]).is_err());
        panel.angle_deg = f64::NAN;
        assert!(panel.request(&source, &[line]).is_err());
    }
    #[test]
    fn line_trim_extend_require_explicit_target_boundary_and_current_pick_values() {
        let (source, line, boundary) = source();
        let before = source.clone();
        let mut panel = Panel::default();
        panel.begin(OperationTool::Trim);
        assert!(panel.request(&source, &[line]).is_err());
        panel.target = Some(line);
        panel.boundaries.insert(boundary);
        panel.pick_mm = [2., 0.];
        let OperationRequest::Trim(TrimRequest::TrimCurve(request), policy) =
            panel.request(&source, &[line]).unwrap()
        else {
            panic!("trim")
        };
        assert_eq!(request.target, line);
        assert_eq!(request.pick_mm, [2., 0.]);
        assert_eq!(request.boundaries, BTreeSet::from([boundary]));
        assert_eq!(policy, TrimPolicy::default());
        panel.tool = Some(OperationTool::Extend);
        panel.endpoint = Endpoint::Start;
        let OperationRequest::Trim(TrimRequest::Extend(request), _) =
            panel.request(&source, &[]).unwrap()
        else {
            panic!("extend")
        };
        assert_eq!(request.endpoint, Endpoint::Start);
        panel.boundaries.insert(line);
        assert!(panel.request(&source, &[]).is_err());
        assert_eq!(source, before);
    }
    #[test]
    fn spline_targets_and_boundaries_build_explicit_exact_query_inputs() {
        let (mut source, line, _) = source();
        let poles: Vec<_> = [[0., 1.], [4., 3.], [7., 2.], [10., 1.]]
            .into_iter()
            .map(|p| source.add_point(p))
            .collect();
        let spline = EntityId::new();
        source.sketch.curves.push(Curve {
            id: spline,
            construction: false,
            kind: CurveKind::BSpline {
                poles,
                basis: SplineBasis::open_uniform(3, 4).unwrap(),
            },
        });
        let mut panel = Panel::default();
        panel.begin(OperationTool::Trim);
        panel.target = Some(line);
        panel.boundaries.insert(spline);
        assert!(matches!(
            panel.request(&source, &[]).unwrap(),
            OperationRequest::Trim(TrimRequest::TrimCurve(_), _)
        ));
        panel.target = Some(spline);
        panel.boundaries = BTreeSet::from([line]);
        assert!(matches!(
            panel.request(&source, &[]).unwrap(),
            OperationRequest::Trim(TrimRequest::TrimCurve(_), _)
        ));
        panel.tool = Some(OperationTool::Extend);
        let OperationRequest::Trim(TrimRequest::ExtendSpline(request), _) =
            panel.request(&source, &[]).unwrap()
        else {
            panic!("Spline extension")
        };
        assert_eq!(request.max_length_mm, 100.);
        assert_eq!(request.contact_index, None);
    }
    #[test]
    fn every_panel_builds_headlessly_without_emitting_a_request_or_changing_source() {
        let (source, line, boundary) = source();
        let before = source.clone();
        let ctx = egui::Context::default();
        for tool in OperationTool::ALL {
            let mut panel = Panel::default();
            panel.begin(tool);
            ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(panel.show(ui, &source, &[line, boundary], true).is_none());
            })
            .drop_without_applying_deltas();
            assert_eq!(source, before);
        }
    }
    #[test]
    fn analytic_curve_controls_build_generic_trim_and_arc_extension_without_changing_source() {
        let (mut source, _, boundary) = source();
        let center = source.add_point([0., 0.]);
        let start = source.add_point([10., 0.]);
        let end = source.add_point([0., 10.]);
        let arc = EntityId::new();
        let circle = EntityId::new();
        source.sketch.curves.extend([
            Curve {
                id: arc,
                construction: false,
                kind: CurveKind::Arc {
                    center,
                    start,
                    end,
                    clockwise: false,
                },
            },
            Curve {
                id: circle,
                construction: false,
                kind: CurveKind::Circle {
                    center,
                    radius_mm: 10.,
                },
            },
        ]);
        let before = source.clone();
        let mut panel = Panel::default();
        panel.begin(OperationTool::Trim);
        panel.boundaries.insert(boundary);
        panel.pick_mm = [std::f64::consts::FRAC_1_SQRT_2 * 10.; 2];
        for target in [arc, circle] {
            panel.target = Some(target);
            let OperationRequest::Trim(TrimRequest::TrimCurve(request), _) =
                panel.request(&source, &[]).unwrap()
            else {
                panic!("generic curve trim")
            };
            assert_eq!(request.target, target);
            assert_eq!(request.boundaries, BTreeSet::from([boundary]));
        }
        panel.begin(OperationTool::Extend);
        panel.target = Some(arc);
        panel.boundaries.insert(boundary);
        panel.endpoint = Endpoint::End;
        let OperationRequest::Trim(TrimRequest::ExtendArc(request), _) =
            panel.request(&source, &[]).unwrap()
        else {
            panic!("arc extension")
        };
        assert_eq!(request.target, arc);
        assert_eq!(request.endpoint, Endpoint::End);
        panel.target = Some(circle);
        assert!(
            panel
                .request(&source, &[])
                .unwrap_err()
                .contains("closed circle")
        );
        assert!(!target_supported(
            OperationTool::Extend,
            &source
                .sketch
                .curves
                .iter()
                .find(|curve| curve.id == circle)
                .unwrap()
                .kind
        ));
        assert_eq!(source, before);
    }
}

#[cfg(test)]
#[path = "sketch_operations_spline_tests.rs"]
mod spline_tests;
