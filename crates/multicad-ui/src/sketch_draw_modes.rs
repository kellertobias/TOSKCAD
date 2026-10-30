//! Multi-click rectangle and circle drawing modes. Geometry comes from the
//! typed `multicad_core::sketch_draw` inputs, so the native gesture, command
//! line and external control share one solver-checked transaction path.
//! The gesture reuses the editor's `first` / `arc_start` progress slots, which
//! `clear_drawing_progress` resets on Escape or tool change.
use super::*;
use multicad_core::sketch_draw::{
    DrawInput, DrawRequest, PointInput, center_points_rectangle_corners, three_point_circle,
    three_point_rectangle_corners, two_point_circle_center,
};

pub(super) type Click = ([f64; 2], Option<EntityId>);

impl DrawTool {
    /// Modes whose gesture needs the dedicated multi-click path.
    pub fn multi_point(self) -> bool {
        matches!(
            self,
            Self::RectangleCenter
                | Self::RectangleCenterTwo
                | Self::RectangleThree
                | Self::CircleTwoRadius
                | Self::CircleThree
                | Self::Fillet
                | Self::Chamfer
                | Self::Text
        )
    }
    /// Corner treatments that modify existing lines instead of drawing.
    pub fn corner(self) -> bool {
        matches!(self, Self::Fillet | Self::Chamfer)
    }
    /// Number of clicks that complete the gesture.
    pub fn clicks(self) -> usize {
        match self {
            Self::RectangleCenter | Self::Fillet | Self::Chamfer => 2,
            Self::RectangleCenterTwo
            | Self::RectangleThree
            | Self::CircleTwoRadius
            | Self::CircleThree => 3,
            _ => 2,
        }
    }
    pub fn mode_prompt(self, clicked: usize) -> &'static str {
        match (self, clicked) {
            (Self::RectangleCenter, 0) => "Click the rectangle center · Esc cancels",
            (Self::RectangleCenter, _) => "Click a corner · Esc cancels",
            (Self::RectangleCenterTwo, 0) => "Click the rectangle center · Esc cancels",
            (Self::RectangleCenterTwo, 1) => "Click one corner · Esc cancels",
            (Self::RectangleCenterTwo, _) => {
                "Click toward the neighbouring corner (same distance from the center) · Esc cancels"
            }
            (Self::RectangleThree, 0) => "Click the first corner · Esc cancels",
            (Self::RectangleThree, 1) => "Click the second corner of the first edge · Esc cancels",
            (Self::RectangleThree, _) => "Click to set the rectangle height · Esc cancels",
            (Self::CircleTwoRadius, 0) => "Click the first point on the circle · Esc cancels",
            (Self::CircleTwoRadius, 1) => "Click the second point on the circle · Esc cancels",
            (Self::CircleTwoRadius, _) => {
                "Click to choose the center side and radius · Esc cancels"
            }
            (Self::CircleThree, 0) => "Click the first point on the circle · Esc cancels",
            (Self::CircleThree, 1) => "Click the second point on the circle · Esc cancels",
            (Self::CircleThree, _) => "Click the third point on the circle · Esc cancels",
            (Self::Fillet, _) => {
                "Click a corner shared by two lines to round it with the Inspector radius · Esc ends"
            }
            (Self::Chamfer, _) => {
                "Click a corner shared by two lines to chamfer it by the Inspector setback · Esc ends"
            }
            (Self::Text, _) => "Click the baseline origin to place the Inspector text · Esc ends",
            _ => "Click the first point, then the second point · Esc cancels",
        }
    }
}

fn input_point(click: Click) -> PointInput {
    click
        .1
        .map(PointInput::Existing)
        .unwrap_or(PointInput::Position(click.0))
}

/// Typed drawing input completing `tool` with the clicks so far plus `last`.
pub(super) fn mode_input(tool: DrawTool, clicks: &[Click], last: Click) -> Option<DrawInput> {
    Some(match (tool, clicks) {
        (DrawTool::RectangleCenter, [center]) => DrawInput::CenterRectangle {
            center: center.0,
            corner: input_point(last),
        },
        (DrawTool::RectangleCenterTwo, [center, corner]) => DrawInput::CenterPointsRectangle {
            center: center.0,
            corner: input_point(*corner),
            adjacent: last.0,
        },
        (DrawTool::RectangleThree, [a, b]) => DrawInput::ThreePointRectangle {
            a: input_point(*a),
            b: input_point(*b),
            height: last.0,
        },
        (DrawTool::CircleTwoRadius, [a, b]) => {
            let (center, left) = two_point_circle_pick(a.0, b.0, last.0)?;
            DrawInput::TwoPointCircle {
                a: a.0,
                b: b.0,
                radius_mm: distance(center, a.0),
                left,
            }
        }
        (DrawTool::CircleThree, [a, b]) => DrawInput::ThreePointCircle {
            a: a.0,
            b: b.0,
            c: last.0,
        },
        _ => return None,
    })
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Center on the perpendicular bisector of `a`-`b` nearest to `pick`, and
/// whether it lies left of the direction from `a` to `b`.
pub(super) fn two_point_circle_pick(
    a: [f64; 2],
    b: [f64; 2],
    pick: [f64; 2],
) -> Option<([f64; 2], bool)> {
    let chord = distance(a, b);
    if chord <= 1e-7 {
        return None;
    }
    let normal = [-(b[1] - a[1]) / chord, (b[0] - a[0]) / chord];
    let mid = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
    let offset = (pick[0] - mid[0]) * normal[0] + (pick[1] - mid[1]) * normal[1];
    Some((
        [mid[0] + normal[0] * offset, mid[1] + normal[1] * offset],
        offset >= 0.,
    ))
}

fn circle_segments(center: [f64; 2], radius: f64) -> Vec<[[f64; 2]; 2]> {
    let steps = 72;
    (0..steps)
        .map(|i| {
            let at = |k: usize| {
                let angle = k as f64 / steps as f64 * std::f64::consts::TAU;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            };
            [at(i), at(i + 1)]
        })
        .collect()
}

fn loop_segments(corners: [[f64; 2]; 4]) -> Vec<[[f64; 2]; 2]> {
    (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect()
}

/// Preview segments for the gesture in progress; empty when the cursor does
/// not yet define valid geometry.
pub(super) fn mode_ghost(tool: DrawTool, clicks: &[Click], cursor: [f64; 2]) -> Vec<[[f64; 2]; 2]> {
    let positions: Vec<[f64; 2]> = clicks.iter().map(|c| c.0).collect();
    match (tool, positions.as_slice()) {
        (_, []) => vec![],
        (DrawTool::RectangleCenter, [center]) => {
            let opposite = [2. * center[0] - cursor[0], 2. * center[1] - cursor[1]];
            loop_segments([
                cursor,
                [opposite[0], cursor[1]],
                opposite,
                [cursor[0], opposite[1]],
            ])
        }
        (DrawTool::RectangleCenterTwo, [center]) => vec![
            [*center, cursor],
            [
                *center,
                [2. * center[0] - cursor[0], 2. * center[1] - cursor[1]],
            ],
        ],
        (DrawTool::RectangleCenterTwo, [center, corner]) => {
            center_points_rectangle_corners(*center, *corner, cursor)
                .map(loop_segments)
                .unwrap_or_default()
        }
        (DrawTool::RectangleThree, [a]) => vec![[*a, cursor]],
        (DrawTool::RectangleThree, [a, b]) => three_point_rectangle_corners(*a, *b, cursor)
            .map(loop_segments)
            .unwrap_or_else(|_| vec![[*a, *b]]),
        (DrawTool::CircleTwoRadius | DrawTool::CircleThree, [a]) => vec![[*a, cursor]],
        (DrawTool::CircleTwoRadius, [a, b]) => two_point_circle_pick(*a, *b, cursor)
            .and_then(|(center, left)| {
                two_point_circle_center(*a, *b, distance(center, *a), left).ok()
            })
            .map(|center| circle_segments(center, distance(center, *a)))
            .unwrap_or_else(|| vec![[*a, *b]]),
        (DrawTool::CircleThree, [a, b]) => three_point_circle(*a, *b, cursor)
            .map(|(center, radius)| circle_segments(center, radius))
            .unwrap_or_else(|_| vec![[*a, *b]]),
        _ => vec![],
    }
}

pub(super) struct ModePointer {
    pub frame: Frame,
    pub rect: Rect,
    pub screen: Pos2,
    pub point: [f64; 2],
    pub anchor: Option<EntityId>,
    pub clicked: bool,
}

impl SketchEditor {
    fn mode_clicks(&self) -> Vec<Click> {
        self.first
            .map(|first| (first, self.first_anchor))
            .into_iter()
            .chain(self.arc_start)
            .collect()
    }
    pub(super) fn mode_pointer(
        &mut self,
        painter: &egui::Painter,
        session: &mut Session,
        tool: DrawTool,
        input: ModePointer,
    ) {
        if tool.corner() {
            self.corner_pointer(painter, session, tool, input);
            return;
        }
        if tool == DrawTool::Text {
            self.text_pointer(painter, session, input);
            return;
        }
        let clicks = self.mode_clicks();
        for [a, b] in mode_ghost(tool, &clicks, input.point) {
            if let Some(segment) =
                self.camera
                    .segment(input.frame.point(a), input.frame.point(b), input.rect)
            {
                painter.line_segment(segment, Stroke::new(1., Color32::LIGHT_YELLOW));
            }
        }
        if !input.clicked {
            return;
        }
        let click = (input.point, input.anchor);
        if clicks.len() + 1 < tool.clicks() {
            if clicks.iter().any(|c| distance(c.0, click.0) < 1e-7) {
                self.message = "Choose a different point".into();
                return;
            }
            if clicks.is_empty() {
                self.first = Some(click.0);
                self.first_anchor = click.1;
            } else {
                self.arc_start = Some(click);
            }
            self.message = tool.mode_prompt(clicks.len() + 1).into();
            return;
        }
        let result = mode_input(tool, &clicks, click)
            .ok_or_else(|| "Choose a point away from the first points".to_string())
            .and_then(|input| {
                self.draw(
                    session,
                    &DrawRequest {
                        input,
                        construction: self.construction,
                    },
                    Channel::Ui,
                )
            });
        match result {
            // The gesture stays on its last points so a corrected final click
            // can retry; Escape clears it.
            Err(error) => self.message = error,
            Ok(()) => {
                self.first = None;
                self.first_anchor = None;
                self.arc_start = None;
                self.message = tool.mode_prompt(0).into();
            }
        }
    }
}

impl SketchEditor {
    pub(super) fn corner_size_ui(&mut self, ui: &mut egui::Ui, tool: DrawTool) {
        ui.horizontal(|ui| {
            ui.label(if tool == DrawTool::Fillet {
                "Radius"
            } else {
                "Setback"
            });
            ui.add(
                egui::DragValue::new(&mut self.corner_size_mm)
                    .range(0.001..=1.0e6)
                    .speed(0.1)
                    .suffix(" mm"),
            )
            .on_hover_text(
                "Applied to the next corner you click. The result keeps a solver dimension you can edit later.",
            );
        });
    }
    /// Nearest corner shared by exactly two lines within 10 px of the pointer.
    fn hovered_corner(
        &self,
        session: &Session,
        input: &ModePointer,
    ) -> Option<(EntityId, [f64; 2])> {
        let id = self.selected?;
        let (source, _, _) = self
            .read_solved_source(session.document(), id, session.revision())
            .ok()?;
        let corners = multicad_core::sketch_corner::treatable_corners(&source);
        let camera = self.camera;
        source
            .sketch
            .points
            .iter()
            .filter(|point| corners.contains(&point.id))
            .filter_map(|point| {
                let p = camera.project(
                    input.frame.point(point.position_mm).map(|v| v as f32),
                    input.rect,
                );
                let distance = Pos2::new(p[0], p[1]).distance(input.screen);
                (distance <= 10.).then_some((distance, point.id, point.position_mm))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id, position)| (id, position))
    }
    fn corner_pointer(
        &mut self,
        painter: &egui::Painter,
        session: &mut Session,
        tool: DrawTool,
        input: ModePointer,
    ) {
        let hovered = input
            .anchor
            .and_then(|anchor| {
                self.hovered_corner(session, &input)
                    .filter(|(id, _)| *id == anchor)
            })
            .or_else(|| self.hovered_corner(session, &input));
        if let Some((_, position)) = hovered {
            let p = self
                .camera
                .project(input.frame.point(position).map(|v| v as f32), input.rect);
            painter.circle_stroke(
                Pos2::new(p[0], p[1]),
                7.,
                Stroke::new(1.5, Color32::LIGHT_YELLOW),
            );
        }
        if !input.clicked {
            return;
        }
        let result = match hovered {
            None => Err("Click a corner shared by exactly two lines".to_string()),
            Some((corner, _)) => {
                self.treat_corner(session, corner, corner_treatment(tool, self.corner_size_mm))
            }
        };
        self.message = match result {
            Ok(()) => format!("{} applied · Undo restores the sharp corner", tool.label()),
            Err(error) => error,
        };
    }
    /// One solver-checked transaction; failures change nothing.
    pub fn treat_corner(
        &mut self,
        session: &mut Session,
        corner: EntityId,
        treatment: multicad_core::sketch_corner::CornerTreatment,
    ) -> Result<(), String> {
        if !self.editing || self.has_pending_edits() {
            return Err("Open a sketch and apply or discard pending edits first".into());
        }
        let id = self.selected.ok_or("Select a sketch")?;
        let (source, _, _) = self
            .read_source(session.document(), id, session.revision())
            .map_err(|e| e.to_string())?;
        let draft = multicad_core::sketch_corner::corner_draft(&source, corner, treatment)
            .map_err(|e| e.to_string())?;
        let edit = multicad_solver::prepare_edit_with_report(
            session,
            session.revision(),
            Actor::local(Channel::Ui),
            id,
            draft.source,
            self.evaluated.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        let preview = edit.preview.ok_or_else(|| {
            format!(
                "The corner treatment conflicts with existing constraints ({:?}); nothing changed",
                edit.evaluation.diagnostics.state
            )
        })?;
        session.commit(preview).map_err(|e| e.to_string())?;
        self.element = draft.created_curves.first().copied();
        self.revision = session.revision();
        Ok(())
    }
}

pub(super) fn corner_treatment(
    tool: DrawTool,
    size: f64,
) -> multicad_core::sketch_corner::CornerTreatment {
    use multicad_core::sketch_corner::CornerTreatment;
    if tool == DrawTool::Chamfer {
        CornerTreatment::Chamfer { distance_mm: size }
    } else {
        CornerTreatment::Fillet { radius_mm: size }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Click {
        ([x, y], None)
    }

    #[test]
    fn mode_inputs_need_their_full_click_sequence() {
        for tool in DrawTool::ALL
            .into_iter()
            .filter(|t| t.multi_point() && !t.corner() && *t != DrawTool::Text)
        {
            let clicks: Vec<Click> = (0..tool.clicks() - 1)
                .map(|i| at(i as f64 * 10., (i * i) as f64 * 7.))
                .collect();
            assert!(mode_input(tool, &clicks, at(3., 40.)).is_some(), "{tool:?}");
            assert!(mode_input(tool, &[], at(3., 40.)).is_none());
        }
        assert!(!DrawTool::Rectangle.multi_point());
        assert!(mode_input(DrawTool::Rectangle, &[at(0., 0.)], at(1., 1.)).is_none());
    }

    #[test]
    fn two_point_circle_pick_follows_the_cursor_side_and_distance() {
        let (center, left) = two_point_circle_pick([-4., 0.], [4., 0.], [1., 3.]).unwrap();
        assert_eq!((center, left), ([0., 3.], true));
        let (center, left) = two_point_circle_pick([-4., 0.], [4., 0.], [-2., -3.]).unwrap();
        assert_eq!((center, left), ([0., -3.], false));
        let Some(DrawInput::TwoPointCircle {
            radius_mm, left, ..
        }) = mode_input(
            DrawTool::CircleTwoRadius,
            &[at(-4., 0.), at(4., 0.)],
            at(0., 3.),
        )
        else {
            panic!()
        };
        assert_eq!((radius_mm, left), (5., true));
        assert!(two_point_circle_pick([1., 1.], [1., 1.], [0., 0.]).is_none());
    }

    #[test]
    fn ghosts_show_the_shape_that_the_final_click_would_create() {
        let ghost = mode_ghost(
            DrawTool::RectangleThree,
            &[at(0., 0.), at(10., 0.)],
            [4., 5.],
        );
        assert_eq!(
            ghost,
            vec![
                [[0., 0.], [10., 0.]],
                [[10., 0.], [10., 5.]],
                [[10., 5.], [0., 5.]],
                [[0., 5.], [0., 0.]]
            ]
        );
        let circle = mode_ghost(
            DrawTool::CircleThree,
            &[at(10., 0.), at(0., 10.)],
            [-10., 0.],
        );
        assert_eq!(circle.len(), 72);
        assert!(
            circle
                .iter()
                .all(|[a, _]| (distance(*a, [0., 0.]) - 10.).abs() < 1e-9)
        );
        // Collinear third point: only the chord is previewed, never a circle.
        assert_eq!(
            mode_ghost(DrawTool::CircleThree, &[at(0., 0.), at(5., 5.)], [9., 9.]).len(),
            1
        );
        assert!(mode_ghost(DrawTool::RectangleCenter, &[], [1., 1.]).is_empty());
    }

    #[test]
    fn command_line_modes_parse_to_the_typed_inputs() {
        let parse = |tool: DrawTool, args: &[&str]| tool.parse(args, false).map(|r| r.input);
        assert!(matches!(
            parse(DrawTool::RectangleCenter, &["0", "0", "10", "5"]),
            Ok(DrawInput::CenterRectangle {
                center: [0., 0.],
                ..
            })
        ));
        assert!(matches!(
            parse(DrawTool::CircleTwoRadius, &["-4", "0", "4", "0", "5", "right"]),
            Ok(DrawInput::TwoPointCircle { left: false, radius_mm, .. }) if radius_mm == 5.
        ));
        assert!(matches!(
            parse(DrawTool::CircleTwoRadius, &["-4", "0", "4", "0", "5"]),
            Ok(DrawInput::TwoPointCircle { left: true, .. })
        ));
        assert!(matches!(
            parse(DrawTool::CircleThree, &["1", "0", "0", "1", "-1", "0"]),
            Ok(DrawInput::ThreePointCircle { .. })
        ));
        assert!(matches!(
            parse(DrawTool::RectangleThree, &["0", "0", "10", "0", "5", "5"]),
            Ok(DrawInput::ThreePointRectangle { .. })
        ));
        assert!(matches!(
            parse(
                DrawTool::RectangleCenterTwo,
                &["0", "0", "10", "0", "0", "10"]
            ),
            Ok(DrawInput::CenterPointsRectangle { .. })
        ));
        for (tool, args) in [
            (DrawTool::RectangleCenter, vec!["0", "0", "1"]),
            (
                DrawTool::CircleTwoRadius,
                vec!["0", "0", "1", "1", "2", "up"],
            ),
            (DrawTool::CircleThree, vec!["0", "0", "1", "1", "2"]),
            (
                DrawTool::RectangleThree,
                vec!["0", "0", "1", "1", "NaN", "2"],
            ),
        ] {
            assert!(tool.parse(&args, false).is_err(), "{tool:?} {args:?}");
        }
    }
}
