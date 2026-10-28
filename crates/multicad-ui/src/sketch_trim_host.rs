//! Current-source bridge between the operation form and exact worker replies.
use super::*;
use crate::{
    sketch_evaluation::EvaluationStamp,
    sketch_operations::{OperationRequest, OperationTool},
};
use multicad_core::{
    constraint_sketch::CurveKind,
    sketch_regions::ExactCurve,
    sketch_trim::{TrimPolicy, TrimRequest},
};

pub(super) struct PickView {
    curve: EntityId,
    point: [f64; 2],
    camera: Camera,
    frame: Frame,
    viewport: Rect,
}

impl SketchEditor {
    pub(super) fn cancel_operation(&mut self) {
        self.operations.cancel();
        self.trim_worker.cancel();
        self.operation_context = None;
        self.operation_guard = None;
        self.operation_pick_view = None;
        self.operation_report = None;
    }
    pub(super) fn exact_operation(&self, source: &Source, request: &TrimRequest) -> bool {
        self.operations.viewport_span_pick().is_some()
            || matches!(request, TrimRequest::ExtendSpline(_))
            || source.sketch.curves.iter().any(|curve| {
                (curve.id == request.target() || request.boundaries().contains(&curve.id))
                    && matches!(curve.kind, CurveKind::BSpline { .. })
            })
    }
    fn current_operation_context(&self) -> Option<(EntityId, EvaluationStamp)> {
        self.selected
            .and_then(|id| self.sketch_evaluation.current(id).map(|p| (id, p.stamp)))
    }
    pub(super) fn poll_exact_operation(&mut self, session: &Session) {
        let current = self.current_operation_context();
        let invalid_guard = self
            .operation_guard
            .as_ref()
            .is_some_and(|g| session.validate_preview(g).is_err());
        if (self.operations.tool().is_none()
            && (self.trim_worker.pending() || self.operation_context.is_some()))
            || self.operation_context.is_some() && self.operation_context != current
            || invalid_guard
        {
            self.trim_worker.cancel();
            self.operations.clear_extension_contacts();
            self.operations.clear_viewport_pick();
            self.operation_report = None;
            self.operation_context = None;
            self.operation_guard = None;
            self.operation_pick_view = None;
        }
        if let Some(expected) = self.trim_worker.expected()
            && (current != Some((expected.sketch, expected.evaluation))
                || expected.evaluation.document != session.document().id()
                || expected.evaluation.revision != session.revision())
        {
            self.trim_worker.cancel();
        }
        let Some(reply) = self.trim_worker.poll() else {
            return;
        };
        if current != Some((reply.stamp.sketch, reply.stamp.evaluation))
            || self.operations.tool().is_none()
            || self
                .operation_guard
                .as_ref()
                .is_none_or(|g| session.validate_preview(g).is_err())
        {
            return;
        }
        let Ok((source, _, _)) =
            self.read_source(session.document(), reply.stamp.sketch, session.revision())
        else {
            return;
        };
        if self
            .operations
            .request(&source, self.constraints.selection())
            .ok()
            != Some(OperationRequest::Trim(reply.request.clone(), reply.policy))
            || self.operations.viewport_span_pick().is_some() != reply.viewport_pick
        {
            self.operations.clear_extension_contacts();
            self.operation_error =
                Some("Operation controls changed. Preview the current choices again.".into());
            return;
        }
        let result = match reply.result {
            Ok(trim_worker::Outcome::Contacts { request, contacts }) => {
                let count = contacts.len();
                self.operations
                    .set_extension_contacts(&request, contacts)
                    .map(|()| {
                        self.message = format!(
                            "{count} exact extension contacts · choose a contact, then Preview"
                        );
                        self.operation_error = None;
                    })
            }
            Ok(trim_worker::Outcome::Edit(result)) => {
                let result = *result;
                self.operation_report = Some(result.report);
                self.adopt_operation_edit(
                    session,
                    reply.stamp.sketch,
                    self.operations.tool().expect("checked active operation"),
                    result.edit,
                )
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.operation_error = Some(error.clone());
            self.message = error;
        }
    }
    pub(super) fn request_exact_operation(
        &mut self,
        session: &Session,
        request: TrimRequest,
        policy: TrimPolicy,
    ) -> Result<(), String> {
        let id = self.selected.ok_or("Select an active sketch")?;
        let presentation = self
            .sketch_evaluation
            .current(id)
            .ok_or("Resolve the current sketch before editing exact curves")?;
        let context = presentation.stamp;
        let pick = if let Some((curve, point)) = self.operations.viewport_span_pick() {
            let raw = matches!(&request, TrimRequest::Trim(r) | TrimRequest::TrimCurve(r) if r.target == curve && r.pick_mm == point);
            if !raw {
                return Err("The recorded span pick no longer matches these controls".into());
            }
            let geometry = presentation
                .geometry
                .iter()
                .find(|g| g.id() == curve)
                .ok_or("The picked curve has no current solved geometry")?;
            let exact = match geometry {
                Geometry::Line { a, b, .. } => ExactCurve::Line { a: *a, b: *b },
                Geometry::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                    ..
                } => ExactCurve::Arc {
                    center: *center,
                    a: *a,
                    b: *b,
                    clockwise: *clockwise,
                },
                Geometry::Circle {
                    center, radius_mm, ..
                } => ExactCurve::Circle {
                    center: *center,
                    radius_mm: *radius_mm,
                },
                Geometry::BSpline { poles, basis, .. } => ExactCurve::BSpline {
                    poles: poles.clone(),
                    basis: basis.clone(),
                },
                Geometry::Rectangle { .. } => {
                    return Err("Open an editable constrained curve before trimming".into());
                }
            };
            let view = self
                .operation_pick_view
                .as_ref()
                .filter(|v| v.curve == curve && v.point == point)
                .ok_or("Pick the current curve span in the viewport again")?;
            let frame = view.frame;
            let rect = view.viewport;
            let screen = view
                .camera
                .project(frame.point(point).map(|v| v as f32), rect);
            let radius = [
                egui::pos2(screen[0] + 8., screen[1]),
                egui::pos2(screen[0], screen[1] + 8.),
            ]
            .into_iter()
            .filter_map(|p| view.camera.on_plane(p, rect, frame))
            .map(|p| (p[0] - point[0]).hypot(p[1] - point[1]))
            .filter(|r| r.is_finite() && *r > 0.)
            .fold(0., f64::max);
            if radius <= 0. {
                return Err("This view cannot resolve a finite sketch span pick".into());
            }
            Some(trim_worker::PickProjection {
                curve: exact,
                maximum_distance_mm: radius.clamp(policy.tolerance_mm, 1e6),
                camera: view.camera,
                frame,
                viewport: rect,
            })
        } else {
            None
        };
        self.trim_worker.request(
            session,
            trim_worker::Input {
                sketch: id,
                evaluation: context,
                report: self.evaluated.clone(),
                request,
                policy,
                queries: self.sketch_evaluation.query_factory(),
                pick,
            },
        )?;
        self.operation_context = Some((id, context));
        self.operation_guard = Some(
            session
                .prepare(Request {
                    expected_revision: session.revision(),
                    actor: Actor::local(Channel::Ui),
                    label: "Guard exact sketch operation".into(),
                    edits: vec![],
                })
                .map_err(|e| e.to_string())?,
        );
        self.operation_report = None;
        self.operation_error = None;
        self.message = "Checking exact curves on the background worker · document unchanged".into();
        Ok(())
    }
    pub(super) fn adopt_operation_edit(
        &mut self,
        session: &Session,
        id: EntityId,
        tool: OperationTool,
        edit: multicad_solver::SketchEdit,
    ) -> Result<(), String> {
        self.operation_context = self.current_operation_context();
        let ready = edit.preview.is_some();
        self.constraints.accept_edit(
            session,
            id,
            edit,
            &format!("Apply {}", tool.label().to_lowercase()),
        )?;
        self.clear_drawing_progress();
        self.operation_error = None;
        self.message = if ready {
            format!(
                "{} preview · Apply commits one undoable edit, Discard keeps the original",
                tool.label()
            )
        } else {
            format!(
                "{} preview has unresolved constraints · inspect diagnostics or Discard",
                tool.label()
            )
        };
        Ok(())
    }
    pub(super) fn pick_operation_curve(
        &mut self,
        session: &Session,
        curve: EntityId,
        point: [f64; 2],
        toggle: bool,
        viewport: bool,
    ) -> Result<(), String> {
        let id = self
            .selected
            .ok_or("Open a sketch before choosing operation curves")?;
        let context = self
            .sketch_evaluation
            .current(id)
            .map(|p| p.stamp)
            .ok_or("Wait for the current solved sketch")?;
        if context.document != session.document().id()
            || context.revision != session.revision()
            || context.source_hash
                != multicad_core::project_file::ContentHash::document(session.document())?
        {
            return Err(
                "Wait for the current solved sketch before choosing operation curves".into(),
            );
        }
        let (source, frame, _) = self
            .read_source(session.document(), id, session.revision())
            .map_err(|e| e.to_string())?;
        let capture_view = viewport
            && self.operations.tool() == Some(OperationTool::Trim)
            && matches!(
                self.operations.curve_pick_lane(),
                Some(
                    crate::sketch_operations::CurvePickLane::Target
                        | crate::sketch_operations::CurvePickLane::Span
                )
            );
        let viewport_rect = if capture_view {
            Some(
                self.viewport_rect
                    .ok_or("Open the sketch viewport before picking a span")?,
            )
        } else {
            None
        };
        if viewport {
            self.operations
                .set_viewport_curve_pick(&source, curve, point, toggle)?;
        } else {
            self.operations
                .set_curve_pick(&source, curve, point, toggle)?;
        }
        if let Some(viewport) = viewport_rect {
            self.operation_pick_view = Some(PickView {
                curve,
                point,
                camera: self.camera,
                frame,
                viewport,
            });
        } else if self.operations.viewport_span_pick().is_none() {
            self.operation_pick_view = None;
        }
        self.element = Some(curve);
        self.constraints.clear_geometry_selection();
        self.profiles.clear();
        self.cancel_profile_clicks();
        self.operation_report = None;
        self.operation_error = None;
        self.operation_context = Some((id, context));
        self.operation_guard = Some(
            session
                .prepare(Request {
                    expected_revision: session.revision(),
                    actor: Actor::local(Channel::Ui),
                    label: "Guard sketch curve selection".into(),
                    edits: vec![],
                })
                .map_err(|e| e.to_string())?,
        );
        self.message =
            "Operation curve selected · choose boundaries and the span or endpoint, then Preview"
                .into();
        Ok(())
    }
    pub(super) fn show_operation_report(&self, ui: &mut egui::Ui) {
        if self.trim_worker.pending() {
            ui.small("Checking exact curve geometry…");
        }
        if let Some(report) = &self.operation_report {
            ui.small(format!(
                "Checked curve edit: {} removed, {} created; {} constraints preserved.",
                report.removed_curves.len(),
                report.created_curves.len(),
                report.preserved_constraints.len()
            ));
            if !report.removed_curves.is_empty() {
                ui.small("References to removed curves require explicit repair. Unchanged source geometry remains linked.");
            }
        }
    }
}
