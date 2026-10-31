//! View-local selection of current solved sketch geometry and exact profiles.
//! Screen segments participate only in pointer hit testing, never manufacture.
use crate::camera::Camera;
use eframe::egui::{self, Pos2, Rect};
use multicad_core::{
    constraint_sketch::Point,
    document::EntityId,
    sketch::{Frame, Geometry},
    sketch_regions::{Extraction, RegionIntent},
};

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Point(EntityId),
    Curve(EntityId),
    Region(RegionIntent),
}

pub fn pick(
    pointer: Pos2,
    camera: Camera,
    rect: Rect,
    frame: Frame,
    points: &[Point],
    geometry: &[Geometry],
    profiles: &Extraction,
) -> Result<Option<Hit>, String> {
    if let Some(hit) = pick_geometry(pointer, camera, rect, frame, points, geometry) {
        return Ok(Some(hit));
    }
    let Some(point) = camera.on_plane(pointer, rect, frame) else {
        return Ok(None);
    };
    for region in &profiles.regions {
        if region.contains(point).map_err(|error| error.to_string())? {
            return Ok(Some(Hit::Region(region.intent())));
        }
    }
    Ok(None)
}
pub fn pick_geometry(
    pointer: Pos2,
    camera: Camera,
    rect: Rect,
    frame: Frame,
    points: &[Point],
    geometry: &[Geometry],
) -> Option<Hit> {
    let project = |p| {
        let screen = camera.project(frame.point(p).map(|v| v as f32), rect);
        Pos2::new(screen[0], screen[1])
    };
    if let Some((_, id)) = points
        .iter()
        .map(|point| (pointer.distance(project(point.position_mm)), point.id))
        .filter(|(distance, _)| *distance <= 7.)
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
    {
        return Some(Hit::Point(id));
    }
    let mut nearest: Option<(f32, EntityId)> = None;
    for curve in geometry {
        for [a, b] in curve.segments() {
            let Some([a, b]) = camera.segment(frame.point(a), frame.point(b), rect) else {
                continue;
            };
            let vector = b - a;
            let t =
                ((pointer - a).dot(vector) / vector.length_sq().max(f32::EPSILON)).clamp(0., 1.);
            let distance = pointer.distance(a + vector * t);
            if distance <= 7.
                && nearest.is_none_or(|(best, id)| {
                    distance < best || (distance == best && curve.id() < id)
                })
            {
                nearest = Some((distance, curve.id()));
            }
        }
    }
    if let Some((_, id)) = nearest {
        return Some(Hit::Curve(id));
    }
    None
}

#[derive(Default)]
pub struct ProfileSelection {
    owner: Option<EntityId>,
    selected: Vec<RegionIntent>,
    invalidated: bool,
}
impl ProfileSelection {
    pub fn clear(&mut self) {
        self.selected.clear();
        self.invalidated = false;
    }
    /// Resolve exact authored boundaries, including holes. A new inner loop
    /// invalidates the old selection rather than silently changing its meaning.
    pub fn reconcile(&mut self, owner: EntityId, profiles: &Extraction) -> usize {
        if self.owner != Some(owner) {
            self.owner = Some(owner);
            self.selected.clear();
            self.invalidated = false;
            return 0;
        }
        let before = self.selected.len();
        self.selected
            .retain(|intent| profiles.resolve(intent).is_some());
        let removed = before - self.selected.len();
        self.invalidated |= removed > 0;
        removed
    }
    pub fn choose(&mut self, intent: RegionIntent, extend: bool) {
        self.invalidated = false;
        if !extend {
            self.selected.clear();
            self.selected.push(intent);
        } else if let Some(index) = self.selected.iter().position(|old| old == &intent) {
            self.selected.remove(index);
        } else {
            self.selected.push(intent);
        }
    }
    pub fn selected(&self) -> &[RegionIntent] {
        &self.selected
    }
    pub fn boundary_selected(&self, curve: EntityId) -> bool {
        self.selected.iter().any(|intent| {
            intent.outer.curves.contains(&curve)
                || intent.holes.iter().any(|hole| hole.curves.contains(&curve))
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui, owner: EntityId, profiles: &Extraction) -> bool {
        let mut changed = self.reconcile(owner, profiles) > 0;
        ui.collapsing(
            format!("Profiles · {} closed areas", profiles.regions.len()),
            |ui| {
                ui.small("Select areas independently. Shift-click adds or removes an area.");
                if self.invalidated {
                    ui.colored_label(
                        egui::Color32::LIGHT_YELLOW,
                        "A selected boundary changed; select the profile again.",
                    );
                }
                for (i, region) in profiles.regions.iter().enumerate() {
                    let intent = region.intent();
                    let selected = self.selected.contains(&intent);
                    let response = ui
                        .selectable_label(
                            selected,
                            format!(
                                "Area {} · {:.2} mm² · {} holes",
                                i + 1,
                                region.area_mm2,
                                region.holes.len()
                            ),
                        )
                        .on_hover_text(
                            "Exact closed profile from current solved curves; holes remain cutouts",
                        );
                    if response.clicked() {
                        let modifiers = crate::selection_input::click_modifiers(ui, &response);
                        self.choose(intent, modifiers.shift || modifiers.command);
                        changed = true;
                    }
                }
                if profiles.regions.is_empty() {
                    ui.small("No closed profiles. Close a boundary before making a solid.");
                }
                for diagnostic in &profiles.diagnostics {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, &diagnostic.message);
                }
                if !self.selected.is_empty() && ui.small_button("Clear profile selection").clicked()
                {
                    self.clear();
                    changed = true;
                }
            },
        );
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        constraint_sketch::{Curve, CurveKind, source::Source},
        sketch_regions,
    };
    fn fixture() -> Source {
        let mut source = Source {
            u: [1., 0., 0.],
            sketch: Default::default(),
        };
        source.add_rectangle([0., 0.], [80., 60.]).unwrap();
        let center = source.add_point([20., 20.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 6.,
            },
        });
        source
    }
    #[test]
    fn inspector_profile_rows_preserve_release_modifiers_when_keys_are_released_in_the_same_frame()
    {
        fn frame(
            ctx: &egui::Context,
            selection: &mut ProfileSelection,
            owner: EntityId,
            profiles: &Extraction,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(450., 600.))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    selection.show(ui, owner, profiles);
                },
            );
            output.textures_delta.clear();
            output
        }
        fn click(
            ctx: &egui::Context,
            selection: &mut ProfileSelection,
            owner: EntityId,
            profiles: &Extraction,
            label: &str,
            modifiers: egui::Modifiers,
        ) {
            let output = frame(ctx, selection, owner, profiles, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing Inspector row {label}"));
            frame(
                ctx,
                selection,
                owner,
                profiles,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    },
                ],
            );
            frame(
                ctx,
                selection,
                owner,
                profiles,
                vec![
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
            );
        }
        let source = fixture();
        let before = source.clone();
        let profiles = sketch_regions::extract(&source);
        assert_eq!(profiles.regions.len(), 2);
        let owner = EntityId::new();
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.animation_time = 0.);
        let mut selection = ProfileSelection::default();
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            "Profiles · 2 closed areas",
            egui::Modifiers::NONE,
        );
        let row = |index: usize| {
            let region = &profiles.regions[index];
            format!(
                "Area {} · {:.2} mm² · {} holes",
                index + 1,
                region.area_mm2,
                region.holes.len()
            )
        };
        let inner = profiles
            .regions
            .iter()
            .position(|r| r.holes.is_empty())
            .unwrap();
        let outer = profiles
            .regions
            .iter()
            .position(|r| r.holes.len() == 1)
            .unwrap();
        let a = profiles.regions[inner].intent();
        let b = profiles.regions[outer].intent();
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            &row(inner),
            egui::Modifiers::NONE,
        );
        assert_eq!(selection.selected(), std::slice::from_ref(&a));
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            &row(outer),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(selection.selected(), &[a.clone(), b.clone()]);
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            &row(inner),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(selection.selected(), std::slice::from_ref(&b));
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            &row(inner),
            egui::Modifiers::COMMAND,
        );
        assert_eq!(selection.selected(), &[b.clone(), a.clone()]);
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            &row(outer),
            egui::Modifiers::COMMAND,
        );
        assert_eq!(selection.selected(), std::slice::from_ref(&a));
        click(
            &ctx,
            &mut selection,
            owner,
            &profiles,
            &row(outer),
            egui::Modifiers::NONE,
        );
        assert_eq!(selection.selected(), &[b]);
        assert_eq!(source, before);
    }
    #[test]
    fn picking_prioritizes_points_and_curves_then_distinct_exact_nested_regions() {
        let source = fixture();
        let profiles = sketch_regions::extract(&source);
        let geometry = source.display_geometry().unwrap();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000., 700.));
        let camera = Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            target: [40., 30., 0.],
            scale: 5.,
            ..Default::default()
        };
        let frame = multicad_core::sketch::Datum::Xy.frame();
        let hit = |point| {
            let p = camera.project(frame.point(point).map(|v| v as f32), rect);
            pick(
                Pos2::new(p[0], p[1]),
                camera,
                rect,
                frame,
                &source.sketch.points,
                &geometry,
                &profiles,
            )
            .unwrap()
        };
        assert_eq!(hit([0., 0.]), Some(Hit::Point(source.sketch.points[0].id)));
        assert_eq!(hit([40., 0.]), Some(Hit::Curve(source.sketch.curves[0].id)));
        let Some(Hit::Region(panel)) = hit([50., 30.]) else {
            panic!("Panel region not picked")
        };
        let Some(Hit::Region(hole)) = hit([22., 20.]) else {
            panic!("Inner region not picked")
        };
        assert_ne!(panel, hole);
        assert_eq!(panel.holes.len(), 1);
        assert!(hole.holes.is_empty());
        assert!(hit([100., 100.]).is_none());
    }
    #[test]
    fn profile_selection_is_additive_stable_under_dimensions_and_invalidates_new_holes() {
        let source = fixture();
        let profiles = sketch_regions::extract(&source);
        let owner = EntityId::new();
        let mut selection = ProfileSelection::default();
        selection.reconcile(owner, &profiles);
        selection.choose(profiles.regions[0].intent(), false);
        selection.choose(profiles.regions[1].intent(), true);
        assert_eq!(selection.selected().len(), 2);
        assert!(selection.boundary_selected(source.sketch.curves[0].id));
        let mut resized = source.clone();
        for point in &mut resized.sketch.points {
            point.position_mm[0] *= 2.;
        }
        assert_eq!(
            selection.reconcile(owner, &sketch_regions::extract(&resized)),
            0
        );
        let mut extra = source.clone();
        let center = extra.add_point([50., 30.]);
        extra.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 4.,
            },
        });
        assert_eq!(
            selection.reconcile(owner, &sketch_regions::extract(&extra)),
            1
        );
        assert_eq!(selection.selected().len(), 1);
        selection.reconcile(EntityId::new(), &profiles);
        assert!(selection.selected().is_empty());
    }

    #[test]
    fn diagonal_cells_pick_and_toggle_independently_across_the_shared_stroke() {
        use multicad_core::sketch_regions::{Edge, ExactCurve, extract_line_cells};
        let mut edges: Vec<_> = [
            ([0., 0.], [20., 0.]),
            ([20., 0.], [20., 20.]),
            ([20., 20.], [0., 20.]),
            ([0., 20.], [0., 0.]),
            ([0., 0.], [20., 20.]),
        ]
        .into_iter()
        .map(|(a, b)| Edge {
            authored: EntityId::new(),
            reversed: false,
            fragment: None,
            curve: ExactCurve::Line { a, b },
        })
        .collect();
        let profiles = extract_line_cells(&edges, 1e-7, None).unwrap();
        let camera = Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            target: [10., 10., 0.],
            scale: 15.,
            ..Default::default()
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        let frame = multicad_core::sketch::Datum::Xy.frame();
        let hit = |point| {
            let p = camera.project(frame.point(point).map(|v| v as f32), rect);
            let Some(Hit::Region(intent)) = pick(
                Pos2::new(p[0], p[1]),
                camera,
                rect,
                frame,
                &[],
                &[],
                &profiles,
            )
            .unwrap() else {
                panic!("Expected triangular area");
            };
            intent
        };
        let lower = hit([15., 5.]);
        let upper = hit([5., 15.]);
        assert_ne!(lower, upper);
        let owner = EntityId::new();
        let mut selected = ProfileSelection::default();
        selected.reconcile(owner, &profiles);
        selected.choose(lower.clone(), false);
        selected.choose(upper.clone(), true);
        assert_eq!(selected.selected(), &[lower.clone(), upper.clone()]);
        selected.choose(lower.clone(), true);
        assert_eq!(selected.selected(), std::slice::from_ref(&upper));
        assert!(selected.boundary_selected(edges[4].authored));
        selected.choose(lower, true);
        edges.pop();
        assert_eq!(
            selected.reconcile(owner, &extract_line_cells(&edges, 1e-7, None).unwrap()),
            2
        );
        assert!(selected.selected().is_empty());
    }
}
