//! Placing an aid on the finished part in the viewer: pick the contact on a
//! face, aim the direction in the build plane, and see the contact marker,
//! direction handle and aid outline drawn from the Inspector values. After
//! Preview the native inspection marks crowded Parts and refuses Apply for
//! an invalid placement. Picks only fill the form's fields; nothing reaches
//! the document before Apply.
use super::{FdmAidPanel, finished_parts};
use crate::{
    egui,
    modeling::{FormStamp, TerminationViewport},
    viewport::{AidReview, PickFilter, PickOptions, Scene, SurfacePoint},
};
use multicad_core::{
    document::{Document, EntityId},
    model::{AidKind, ResolvedAid},
};

/// What the next viewer click sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickRole {
    /// Tab contact, bridge start or support footprint centre.
    Contact,
    /// Bridge end.
    End,
    /// Tab or support direction, aimed in the build plane.
    Direction,
}
/// The native placement review of one preview, for the form.
#[derive(Clone, Debug)]
pub(crate) struct Review {
    pub stamp: FormStamp,
    pub result: Result<AidReview, String>,
    /// Why Apply is refused, if it is.
    pub blocked: Option<String>,
}
/// The candidate preview's placement block: a missing or failed inspection,
/// or any placement finding, refuses Apply.
pub(crate) fn placement_block(scene: &Scene, aid: EntityId) -> Option<String> {
    match scene.fdm_review(aid) {
        None => {
            Some("The native preview carried no contact check of this aid; Apply needs one".into())
        }
        Some(Err(error)) => Some(format!("The aid contact check failed: {error}")),
        Some(Ok(review)) if !review.contact_issues.is_empty() => {
            Some(review.contact_issues.join("; "))
        }
        Some(Ok(_)) => None,
    }
}

/// A hovered pick, drawn before it is clicked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Hover {
    Surface {
        part: EntityId,
        surface: SurfacePoint,
    },
    Aim {
        point: [f64; 3],
    },
}

const MARKER: egui::Color32 = egui::Color32::from_rgb(64, 196, 255);
const HOVER: egui::Color32 = egui::Color32::from_rgb(255, 176, 64);
const BAD: egui::Color32 = egui::Color32::from_rgb(255, 80, 80);
const GOOD: egui::Color32 = egui::Color32::from_rgb(96, 220, 120);

/// Round away display noise: micrometres for exact values.
fn exact(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0. { 0. } else { r }
}
fn set(field: &mut crate::modeling::dimension::DimensionField, value: f64) {
    field.text = format!("{}", exact(value));
}
/// A point in the build plane at `z` under the pointer ray.
fn on_plane(view: &TerminationViewport<'_>, pointer: egui::Pos2, z: f64) -> Option<[f64; 3]> {
    let ray = view.camera.ray(pointer, view.rect)?;
    let (o, d) = (ray.origin(), ray.direction());
    if d[2].abs() < 1e-9 {
        return None;
    }
    let t = (z - o[2]) / d[2];
    (t > 0.).then(|| ray.point(t))
}

impl FdmAidPanel {
    pub fn picking(&self) -> bool {
        self.picking.is_some()
    }
    pub(crate) fn set_review(&mut self, review: Option<Review>) {
        self.review = review;
    }
    /// The review of the current inputs' preview, if any.
    pub(crate) fn current_review(&self) -> Option<&Review> {
        self.review.as_ref().filter(|r| r.stamp == self.stamp)
    }
    fn resolved(&self, document: &Document) -> Option<ResolvedAid> {
        let parameters = multicad_core::parameters::evaluate(document).ok()?;
        self.request(document, true)
            .ok()?
            .request
            .aid
            .resolve(&parameters)
            .ok()
    }
    /// The support footprint direction (radians), length and width: the
    /// resolved values, or the typed numbers while the form is incomplete.
    fn footprint(&self, document: &Document) -> (f64, f64, f64) {
        if let Some(ResolvedAid::Support {
            angle_rad,
            length_mm,
            width_mm,
            ..
        }) = self.resolved(document)
        {
            return (angle_rad, length_mm, width_mm);
        }
        let number = |f: &crate::modeling::dimension::DimensionField, default: f64| {
            f.text.trim().parse::<f64>().unwrap_or(default)
        };
        (
            number(&self.angle, 0.).to_radians(),
            number(&self.length, 10.),
            number(&self.width, 10.),
        )
    }
    fn placed(&mut self) {
        self.stamp.changed();
        self.error = None;
    }
    /// Apply a pick on a finished Part's face. `base_z` is the Part's exact
    /// lowest height, where a tab sits. Returns the status message, or why
    /// the pick was refused.
    pub(crate) fn pick_surface(
        &mut self,
        document: &Document,
        part: EntityId,
        surface: SurfacePoint,
        base_z: Option<f64>,
    ) -> Result<String, String> {
        let role = self.picking.ok_or("Choose what to pick first")?;
        if !finished_parts(document).iter().any(|(p, _)| *p == part) {
            return Err("Pick a finished Part; aids cannot serve aids".into());
        }
        // Contacts go on the chosen finished Part; only a bridge end may
        // land on a second Part.
        if role != PickRole::End
            && let Some(target) = self.target
            && target != part
        {
            let name = document
                .get(target)
                .map_or("the chosen Part", |e| e.name.as_str());
            return Err(format!(
                "Pick on {name}, the chosen finished Part; choose another Part in the list to switch"
            ));
        }
        let p = surface.point_mm;
        let how = if surface.exact_plane {
            "on the exact plane"
        } else {
            "from the display surface; Preview checks the exact contact"
        };
        let message = match (self.kind, role) {
            (AidKind::Tab, PickRole::Contact) => {
                let n = surface.normal;
                let h = n[0].hypot(n[1]);
                if h < 0.25 {
                    return Err(
                        "Pick a side face: a tab grows outward from the part outline".into(),
                    );
                }
                let z = base_z.unwrap_or(p[2]);
                set(&mut self.origin[0], p[0]);
                set(&mut self.origin[1], p[1]);
                set(&mut self.origin[2], z);
                set(&mut self.angle, n[1].atan2(n[0]).to_degrees());
                self.target = Some(part);
                self.picking = None;
                format!("Tab contact picked {how}, at the part's base; outward along the face")
            }
            (AidKind::Bridge, PickRole::Contact) => {
                for (f, v) in self.origin.iter_mut().zip(p) {
                    set(f, v);
                }
                self.target = Some(part);
                if self.other == Some(part) {
                    self.other = None;
                }
                self.picking = Some(PickRole::End);
                format!("Bridge start picked {how}; pick the end")
            }
            (AidKind::Bridge, PickRole::End) => {
                for (f, v) in self.end.iter_mut().zip(p) {
                    set(f, v);
                }
                self.other = (self.target != Some(part)).then_some(part);
                self.picking = None;
                format!("Bridge end picked {how}")
            }
            (AidKind::Support, PickRole::Contact) => {
                let (a, l, w) = self.footprint(document);
                let (c, s) = (a.cos(), a.sin());
                set(&mut self.origin[0], p[0] - c * l / 2. + s * w / 2.);
                set(&mut self.origin[1], p[1] - s * l / 2. - c * w / 2.);
                self.target = Some(part);
                self.picking = None;
                "Support footprint centred under the pick; Preview derives its height".into()
            }
            _ => return Err("This pick does not apply to the chosen aid".into()),
        };
        self.seed_note = None;
        self.placed();
        Ok(message)
    }
    /// Aim the tab or support direction at a build-plane point.
    pub(crate) fn aim(&mut self, document: &Document, point: [f64; 3]) -> Result<String, String> {
        let from = self
            .handle_origin(document)
            .ok_or("Place the contact first")?;
        let (dx, dy) = (point[0] - from[0], point[1] - from[1]);
        if dx.hypot(dy) < 1e-6 {
            return Err("Aim away from the contact point".into());
        }
        set(&mut self.angle, dy.atan2(dx).to_degrees().round());
        self.picking = None;
        self.placed();
        Ok("Direction aimed in the build plane (whole degrees)".into())
    }
    /// The authored contact (tab), start (bridge) or footprint corner
    /// (support) in millimetres.
    #[cfg(test)]
    pub(crate) fn origin_mm(&self, document: &Document) -> Option<[f64; 3]> {
        match self.resolved(document)? {
            ResolvedAid::Tab { origin, .. } => Some(origin),
            ResolvedAid::Bridge { start, .. } => Some(start),
            ResolvedAid::Support {
                origin, base_mm, ..
            } => Some([origin[0], origin[1], base_mm]),
        }
    }
    #[cfg(test)]
    pub(crate) fn angle_text(&self) -> String {
        self.angle.text.clone()
    }
    /// Snap a tab to the exact contact the native inspection found.
    pub(crate) fn snap_exact(&mut self) -> bool {
        let Some(contact) = self
            .current_review()
            .and_then(|r| r.result.as_ref().ok())
            .and_then(|r| r.exact_contact)
        else {
            return false;
        };
        if self.kind != AidKind::Tab {
            return false;
        }
        for i in 0..3 {
            set(&mut self.origin[i], contact.point_mm[i]);
        }
        set(&mut self.angle, contact.angle_deg);
        self.placed();
        true
    }
    /// The point the direction handle starts from.
    fn handle_origin(&self, document: &Document) -> Option<[f64; 3]> {
        match self.resolved(document)? {
            ResolvedAid::Tab { origin, .. } => Some(origin),
            ResolvedAid::Support {
                origin,
                angle_rad,
                length_mm,
                width_mm,
                base_mm,
                ..
            } => {
                let (c, s) = (angle_rad.cos(), angle_rad.sin());
                Some([
                    origin[0] + c * length_mm / 2. - s * width_mm / 2.,
                    origin[1] + s * length_mm / 2. + c * width_mm / 2.,
                    base_mm,
                ])
            }
            ResolvedAid::Bridge { .. } => None,
        }
    }

    /// Paint the placement overlay and handle a pick. Returns a status
    /// message when a click changed the inputs.
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        view: TerminationViewport<'_>,
    ) -> Option<String> {
        let scene = view
            .scene
            .filter(|s| s.stamp().document == document.id() && s.stamp().revision == revision);
        self.paint(document, scene, &view);
        let (Some(scene), Some(role)) = (scene, self.picking) else {
            self.hover = None;
            return None;
        };
        let Some(pointer) = view.pointer.filter(|p| view.rect.contains(*p)) else {
            self.hover = None;
            return None;
        };
        let result = if role == PickRole::Direction {
            let z = self.handle_origin(document).map_or(0., |o| o[2]);
            self.hover = on_plane(&view, pointer, z).map(|point| Hover::Aim { point });
            match self.hover {
                Some(Hover::Aim { point }) if view.clicked => Some(self.aim(document, point)),
                _ => None,
            }
        } else {
            let finished: Vec<EntityId> = match (role, self.target) {
                (PickRole::End, _) | (_, None) => {
                    finished_parts(document).iter().map(|p| p.0).collect()
                }
                (_, Some(target)) => vec![target],
            };
            let hit = scene.pick_screen_matching(
                view.camera,
                view.rect,
                pointer,
                PickOptions {
                    filter: PickFilter::Faces,
                    include_hidden: false,
                    section: view.section,
                },
                |instance, _| finished.contains(&instance),
            );
            self.hover = hit.as_ref().and_then(|hit| {
                scene.surface_at(hit).map(|surface| Hover::Surface {
                    part: hit.instance,
                    surface,
                })
            });
            match self.hover {
                Some(Hover::Surface { part, surface }) if view.clicked => {
                    let base = scene.native_instance_bounds(part).map(|b| b[0][2]);
                    Some(self.pick_surface(document, part, surface, base))
                }
                _ => None,
            }
        };
        match result? {
            Ok(message) => {
                self.hover = None;
                Some(message)
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }

    fn paint(&self, document: &Document, scene: Option<&Scene>, view: &TerminationViewport<'_>) {
        let screen = |p: [f64; 3]| {
            let s = view
                .camera
                .project([p[0] as f32, p[1] as f32, p[2] as f32], view.rect);
            egui::pos2(s[0], s[1])
        };
        let painter = view.painter;
        let font = egui::FontId::proportional(11.);
        let polygon = |corners: &[[f64; 3]], color: egui::Color32| {
            for i in 0..corners.len() {
                painter.line_segment(
                    [screen(corners[i]), screen(corners[(i + 1) % corners.len()])],
                    egui::Stroke::new(2., color),
                );
            }
        };
        let arrow = |from: [f64; 3], to: [f64; 3], color: egui::Color32| {
            let (a, b) = (screen(from), screen(to));
            painter.arrow(a, b - a, egui::Stroke::new(2., color));
        };
        let marker = |p: [f64; 3], color: egui::Color32| {
            painter.circle_stroke(screen(p), 5., egui::Stroke::new(2., color));
        };
        let review = self.current_review();
        let bad_contact = review.is_some_and(|r| {
            r.result
                .as_ref()
                .map_or(true, |r| !r.contact_issues.is_empty())
        });
        let color = if bad_contact { BAD } else { MARKER };
        if let Some(aid) = self.resolved(document) {
            let rect = |o: [f64; 3], a: f64, from: f64, to: f64, w: f64| {
                let (c, s) = (a.cos(), a.sin());
                let at = |u: f64, v: f64| [o[0] + c * u - s * v, o[1] + s * u + c * v, o[2]];
                [
                    at(from, -w / 2.),
                    at(to, -w / 2.),
                    at(to, w / 2.),
                    at(from, w / 2.),
                ]
            };
            match aid {
                ResolvedAid::Tab {
                    origin,
                    angle_rad,
                    length_mm,
                    width_mm,
                    overlap_mm,
                    ..
                } => {
                    polygon(
                        &rect(origin, angle_rad, -overlap_mm, length_mm, width_mm),
                        color,
                    );
                    marker(origin, color);
                    let reach = length_mm.max(5.);
                    arrow(
                        origin,
                        [
                            origin[0] + angle_rad.cos() * reach,
                            origin[1] + angle_rad.sin() * reach,
                            origin[2],
                        ],
                        color,
                    );
                    painter.text(
                        screen(origin) + egui::vec2(8., -10.),
                        egui::Align2::LEFT_BOTTOM,
                        format!(
                            "({:.3}, {:.3}, {:.3}) mm · {:.1}°",
                            origin[0],
                            origin[1],
                            origin[2],
                            angle_rad.to_degrees()
                        ),
                        font.clone(),
                        color,
                    );
                }
                ResolvedAid::Bridge { start, end, .. } => {
                    painter
                        .line_segment([screen(start), screen(end)], egui::Stroke::new(2., color));
                    marker(start, color);
                    marker(end, color);
                }
                ResolvedAid::Support {
                    origin,
                    angle_rad,
                    length_mm,
                    width_mm,
                    base_mm,
                    ..
                } => {
                    let (c, s) = (angle_rad.cos(), angle_rad.sin());
                    let corner = [origin[0], origin[1], base_mm];
                    let centre = [
                        origin[0] + c * length_mm / 2. - s * width_mm / 2.,
                        origin[1] + s * length_mm / 2. + c * width_mm / 2.,
                        base_mm,
                    ];
                    polygon(
                        &rect(
                            [centre[0], centre[1], base_mm],
                            angle_rad,
                            -length_mm / 2.,
                            length_mm / 2.,
                            width_mm,
                        ),
                        color,
                    );
                    marker(corner, color);
                    arrow(
                        centre,
                        [
                            centre[0] + c * length_mm / 2.,
                            centre[1] + s * length_mm / 2.,
                            base_mm,
                        ],
                        color,
                    );
                }
            }
        }
        // Native review: crowded Parts and the exact contact.
        if let (Some(Ok(r)), Some(scene)) = (review.map(|r| &r.result), scene) {
            for (part, distance) in &r.crowded {
                let Some([lo, hi]) = scene.native_instance_bounds(*part) else {
                    continue;
                };
                let corner = |i: usize| {
                    [
                        if i & 1 == 0 { lo[0] } else { hi[0] },
                        if i & 2 == 0 { lo[1] } else { hi[1] },
                        if i & 4 == 0 { lo[2] } else { hi[2] },
                    ]
                };
                for (a, b) in [
                    (0, 1),
                    (1, 3),
                    (3, 2),
                    (2, 0),
                    (4, 5),
                    (5, 7),
                    (7, 6),
                    (6, 4),
                    (0, 4),
                    (1, 5),
                    (2, 6),
                    (3, 7),
                ] {
                    painter.line_segment(
                        [screen(corner(a)), screen(corner(b))],
                        egui::Stroke::new(2., BAD),
                    );
                }
                let name = document.get(*part).map_or("Part", |e| e.name.as_str());
                painter.text(
                    screen(corner(7)),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{name}: {distance:.3} mm"),
                    font.clone(),
                    BAD,
                );
            }
            if self.kind == AidKind::Tab
                && let Some(contact) = r.exact_contact
            {
                let p = screen(contact.point_mm);
                let d = 5.;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        p + egui::vec2(0., -d),
                        p + egui::vec2(d, 0.),
                        p + egui::vec2(0., d),
                        p + egui::vec2(-d, 0.),
                    ],
                    egui::Color32::TRANSPARENT,
                    egui::Stroke::new(2., GOOD),
                ));
            }
        }
        match self.hover {
            Some(Hover::Surface { surface, .. }) => {
                let p = surface.point_mm;
                marker(p, HOVER);
                let n = surface.normal;
                arrow(p, std::array::from_fn(|i| p[i] + n[i] * 8.), HOVER);
            }
            Some(Hover::Aim { point }) => {
                if let Some(from) = self.handle_origin(document) {
                    arrow(from, [point[0], point[1], from[2]], HOVER);
                }
            }
            None => {}
        }
    }

    /// The Inspector's placement block: pick buttons, the hover state and
    /// the native contact check. Returns whether the inputs changed.
    pub(crate) fn show_placement(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        let toggle =
            |ui: &mut egui::Ui, picking: &mut Option<PickRole>, role, label: &str, hint: &str| {
                if ui
                    .selectable_label(*picking == Some(role), label)
                    .on_hover_text(hint)
                    .clicked()
                {
                    *picking = if *picking == Some(role) {
                        None
                    } else {
                        Some(role)
                    };
                }
            };
        ui.horizontal_wrapped(|ui| match self.kind {
            AidKind::Tab => {
                toggle(ui, &mut self.picking, PickRole::Contact, "Pick contact", "Click a side face of the finished part; the contact sits at the part's base and points outward");
                toggle(ui, &mut self.picking, PickRole::Direction, "Aim direction", "Click in the build plane to point the tab");
            }
            AidKind::Bridge => {
                toggle(ui, &mut self.picking, PickRole::Contact, "Pick start", "Click the finished part where the bar starts");
                toggle(ui, &mut self.picking, PickRole::End, "Pick end", "Click where the bar ends; a different Part becomes the second Part");
            }
            AidKind::Support => {
                toggle(ui, &mut self.picking, PickRole::Contact, "Pick footprint", "Click the part's underside to centre the footprint there");
                toggle(ui, &mut self.picking, PickRole::Direction, "Aim direction", "Click in the base plane to set the footprint length direction");
            }
        });
        if let Some(role) = self.picking {
            ui.small(match (self.kind, role) {
                (AidKind::Tab, PickRole::Contact) => {
                    "Click a side face of the finished part in the viewer."
                }
                (AidKind::Bridge, PickRole::Contact) => {
                    "Click the start point on the finished part."
                }
                (AidKind::Bridge, PickRole::End) => {
                    "Click the end point on the same or a second Part."
                }
                (AidKind::Support, PickRole::Contact) => {
                    "Click the part where the support should hold it."
                }
                _ => "Click in the build plane to aim the direction.",
            });
            if let Some(Hover::Surface { part, surface }) = self.hover {
                let name = document.get(part).map_or("Part", |e| e.name.as_str());
                let p = surface.point_mm;
                ui.small(format!(
                    "{name}: ({:.3}, {:.3}, {:.3}) mm{}",
                    p[0],
                    p[1],
                    p[2],
                    if surface.exact_plane {
                        " · exact plane"
                    } else {
                        ""
                    }
                ));
            }
        }
        let Some(review) = self.current_review().cloned() else {
            return changed;
        };
        ui.separator();
        ui.label("Native contact check");
        match &review.result {
            Ok(r) => {
                let target = document
                    .get(r.target)
                    .map_or("the part", |e| e.name.as_str());
                if r.touches_target {
                    ui.colored_label(GOOD, format!("Touches {target}"));
                } else {
                    ui.colored_label(BAD, format!("{:.3} mm from {target}", r.target_distance_mm));
                }
                for (part, d) in &r.crowded {
                    let name = document.get(*part).map_or("Part", |e| e.name.as_str());
                    ui.colored_label(BAD, format!("{name}: {d:.3} mm · inside the clearance"));
                }
                for issue in r.issues.iter().filter(|i| !r.contact_issues.contains(i)) {
                    ui.colored_label(egui::Color32::from_rgb(230, 190, 80), issue);
                }
                if self.kind == AidKind::Tab
                    && let Some(c) = r.exact_contact
                {
                    ui.small(format!(
                        "Exact contact ({:.4}, {:.4}, {:.4}) mm · {:.3}°",
                        c.point_mm[0], c.point_mm[1], c.point_mm[2], c.angle_deg
                    ));
                    if ui
                        .button("Snap to exact contact")
                        .on_hover_text(
                            "Use the outline point the native solid gives near this contact",
                        )
                        .clicked()
                    {
                        changed |= self.snap_exact();
                    }
                }
            }
            Err(error) => {
                ui.colored_label(BAD, error);
            }
        }
        if let Some(blocked) = &review.blocked {
            ui.colored_label(BAD, format!("Apply refused: {blocked}"));
        }
        changed
    }
}
