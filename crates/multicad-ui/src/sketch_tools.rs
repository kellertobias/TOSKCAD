//! One drawing-tool catalog for ribbon commands, command-line input and hints.
use crate::{icons::Icon, sketch_editor::DrawTool};
use multicad_core::sketch_draw::{DrawInput, DrawRequest, PointInput};

impl DrawTool {
    pub const ALL: [Self; 14] = [
        Self::Line,
        Self::Rectangle,
        Self::Circle,
        Self::Arc,
        Self::Polyline,
        Self::Spline,
        Self::RectangleCenter,
        Self::RectangleCenterTwo,
        Self::RectangleThree,
        Self::CircleTwoRadius,
        Self::CircleThree,
        Self::Fillet,
        Self::Chamfer,
        Self::Text,
    ];
    /// Rectangle drawing modes offered under one ribbon menu, default first.
    pub const RECTANGLE_MODES: [Self; 4] = [
        Self::Rectangle,
        Self::RectangleCenter,
        Self::RectangleCenterTwo,
        Self::RectangleThree,
    ];
    /// Circle drawing modes offered under one ribbon menu, default first.
    pub const CIRCLE_MODES: [Self; 3] = [Self::Circle, Self::CircleTwoRadius, Self::CircleThree];
    pub fn id(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Rectangle => "rectangle",
            Self::Circle => "circle",
            Self::Arc => "arc",
            Self::Polyline => "polyline",
            Self::Spline => "spline",
            Self::RectangleCenter => "rectangle-center",
            Self::RectangleCenterTwo => "rectangle-center-two",
            Self::RectangleThree => "rectangle-three",
            Self::CircleTwoRadius => "circle-two-radius",
            Self::CircleThree => "circle-three",
            Self::Fillet => "sketch-fillet",
            Self::Chamfer => "sketch-chamfer",
            Self::Text => "sketch-text",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Rectangle => "Rectangle",
            Self::Circle => "Circle",
            Self::Arc => "Arc",
            Self::Polyline => "Polyline",
            Self::Spline => "Spline",
            Self::RectangleCenter => "Center rectangle",
            Self::RectangleCenterTwo => "Center + 2 points rectangle",
            Self::RectangleThree => "3-point rectangle",
            Self::CircleTwoRadius => "2 points + radius circle",
            Self::CircleThree => "3-point circle",
            Self::Fillet => "Sketch fillet",
            Self::Chamfer => "Sketch chamfer",
            Self::Text => "Text",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Line => Icon::Line,
            Self::Rectangle => Icon::Rectangle,
            Self::Circle => Icon::Circle,
            Self::Arc => Icon::Arc,
            Self::Polyline => Icon::Polyline,
            Self::Spline => Icon::Spline,
            Self::RectangleCenter | Self::RectangleCenterTwo | Self::RectangleThree => {
                Icon::Rectangle
            }
            Self::CircleTwoRadius | Self::CircleThree => Icon::Circle,
            Self::Fillet | Self::Chamfer => Icon::Edge,
            Self::Text => Icon::Sketch,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Line => &["line"],
            Self::Rectangle => &["rect", "rectangle"],
            Self::Circle => &["circle"],
            Self::Arc => &["arc"],
            Self::Polyline => &["polyline"],
            Self::Spline => &["spline", "bspline"],
            Self::RectangleCenter => &["rect-center", "center-rectangle"],
            Self::RectangleCenterTwo => &["rect-center2"],
            Self::RectangleThree => &["rect3", "rotated-rectangle"],
            Self::CircleTwoRadius => &["circle2"],
            Self::CircleThree => &["circle3"],
            Self::Fillet => &["corner-fillet"],
            Self::Chamfer => &["corner-chamfer"],
            Self::Text => &["text"],
        }
    }
    pub fn usage(self) -> &'static str {
        match self {
            Self::Line => "line <x1> <y1> <x2> <y2> (mm)",
            Self::Rectangle => "rect <x1> <y1> <x2> <y2> (mm)",
            Self::Circle => "circle <cx> <cy> <radius> (mm)",
            Self::Arc => "arc <cx> <cy> <sx> <sy> <ex> <ey> [cw] (mm)",
            Self::Polyline => "polyline <x1> <y1> <x2> <y2> ... [closed] (mm)",
            Self::Spline => "spline <x1> <y1> ... <x4> <y4> ... [closed] (cubic control poles, mm)",
            Self::RectangleCenter => "rect-center <cx> <cy> <corner-x> <corner-y> (mm)",
            Self::RectangleCenterTwo => {
                "rect-center2 <cx> <cy> <corner-x> <corner-y> <toward-x> <toward-y> (second corner at the same distance from the center, mm)"
            }
            Self::RectangleThree => {
                "rect3 <x1> <y1> <x2> <y2> <hx> <hy> (first edge, then a point setting the height, mm)"
            }
            Self::CircleTwoRadius => {
                "circle2 <x1> <y1> <x2> <y2> <radius> [right] (two points on the circle; center left of 1→2 unless 'right', mm)"
            }
            Self::CircleThree => {
                "circle3 <x1> <y1> <x2> <y2> <x3> <y3> (three points on the circle, mm)"
            }
            Self::Fillet => {
                "sketch-fillet: set the radius in the Inspector, then click a corner shared by two lines or arcs"
            }
            Self::Chamfer => {
                "sketch-chamfer: set the setback in the Inspector, then click a corner shared by two lines or arcs"
            }
            Self::Text => {
                "text: set the string and cap height in the Inspector, then click the baseline origin"
            }
        }
    }
    /// Syntax and finite numeric input are checked here. The shared drawing
    /// draft validates geometry, bounds and source budgets before committing.
    pub fn parse(self, args: &[&str], construction: bool) -> Result<DrawRequest, String> {
        let usage = || format!("Usage: {}", self.usage());
        if self == Self::Text {
            return Err(format!(
                "Text is placed in the sketch view; {}",
                self.usage()
            ));
        }
        if self.corner() {
            return Err(format!(
                "{} picks its corner in the sketch view; {}",
                self.label(),
                self.usage()
            ));
        }
        let (coordinates, flag) = match self {
            Self::Line | Self::Rectangle => {
                if args.len() != 4 {
                    return Err(usage());
                }
                (args, false)
            }
            Self::Circle => {
                if args.len() != 3 {
                    return Err(usage());
                }
                (args, false)
            }
            Self::RectangleCenter => {
                if args.len() != 4 {
                    return Err(usage());
                }
                (args, false)
            }
            Self::RectangleCenterTwo | Self::RectangleThree | Self::CircleThree => {
                if args.len() != 6 {
                    return Err(usage());
                }
                (args, false)
            }
            Self::Fillet | Self::Chamfer | Self::Text => {
                unreachable!("view-placed tools return above")
            }
            Self::CircleTwoRadius => match args.len() {
                5 => (args, false),
                6 if args[5].eq_ignore_ascii_case("right") => (&args[..5], true),
                _ => return Err(usage()),
            },
            Self::Arc => match args.len() {
                6 => (args, false),
                7 if args[6].eq_ignore_ascii_case("cw") => (&args[..6], true),
                _ => return Err(usage()),
            },
            Self::Polyline | Self::Spline => {
                let closed = args
                    .last()
                    .is_some_and(|s| s.eq_ignore_ascii_case("closed"));
                let coordinates = if closed {
                    &args[..args.len() - 1]
                } else {
                    args
                };
                if coordinates.len() % 2 != 0
                    || coordinates.len()
                        < if self == Self::Spline {
                            8
                        } else if closed {
                            6
                        } else {
                            4
                        }
                    || coordinates.len() > 1024
                {
                    return Err(usage());
                }
                (coordinates, closed)
            }
        };
        let values: Vec<f64> = coordinates
            .iter()
            .map(|value| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        "Drawing coordinates and radii must be finite numbers in millimeters".into()
                    })
            })
            .collect::<Result<_, String>>()?;
        let point = |i| PointInput::Position([values[i], values[i + 1]]);
        let at = |i: usize| [values[i], values[i + 1]];
        let input = match self {
            Self::Line => DrawInput::Line {
                start: point(0),
                end: point(2),
            },
            Self::Rectangle => DrawInput::Rectangle {
                a: point(0),
                b: point(2),
            },
            Self::Circle => DrawInput::Circle {
                center: point(0),
                radius_mm: values[2],
            },
            Self::Arc => DrawInput::Arc {
                center: point(0),
                start: point(2),
                end: point(4),
                clockwise: flag,
            },
            Self::Polyline => DrawInput::Polyline {
                vertices: (0..values.len()).step_by(2).map(point).collect(),
                closed: flag,
            },
            Self::Spline => DrawInput::BSpline {
                poles: (0..values.len()).step_by(2).map(point).collect(),
                basis: multicad_core::constraint_sketch::SplineBasis::open_uniform(
                    3,
                    values.len() / 2 + usize::from(flag),
                )
                .map_err(|e| e.to_string())?,
                closed: flag,
            },
            Self::RectangleCenter => DrawInput::CenterRectangle {
                center: at(0),
                corner: point(2),
            },
            Self::RectangleCenterTwo => DrawInput::CenterPointsRectangle {
                center: at(0),
                corner: point(2),
                adjacent: at(4),
            },
            Self::RectangleThree => DrawInput::ThreePointRectangle {
                a: point(0),
                b: point(2),
                height: at(4),
            },
            Self::CircleTwoRadius => DrawInput::TwoPointCircle {
                a: at(0),
                b: at(2),
                radius_mm: values[4],
                left: !flag,
            },
            Self::CircleThree => DrawInput::ThreePointCircle {
                a: at(0),
                b: at(2),
                c: at(4),
            },
            Self::Fillet | Self::Chamfer | Self::Text => {
                unreachable!("view-placed tools return above")
            }
        };
        Ok(DrawRequest {
            input,
            construction,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_arguments_preserve_units_flags_and_direction() {
        for (tool, args) in [
            (DrawTool::Line, vec!["-1", "2", "3.5", "4"]),
            (DrawTool::Rectangle, vec!["0", "0", "40", "20"]),
            (DrawTool::Circle, vec!["10", "20", "5"]),
            (
                DrawTool::Spline,
                vec!["0", "0", "0", "30", "40", "30", "40", "0"],
            ),
            (DrawTool::Arc, vec!["0", "0", "10", "0", "0", "10", "CW"]),
            (
                DrawTool::Polyline,
                vec!["0", "0", "40", "0", "40", "20", "closed"],
            ),
        ] {
            let request = tool.parse(&args, true).unwrap();
            assert!(request.construction);
            match request.input {
                DrawInput::Line {
                    start: PointInput::Position(a),
                    end: PointInput::Position(b),
                } => {
                    assert_eq!(a, [-1., 2.]);
                    assert_eq!(b, [3.5, 4.]);
                }
                DrawInput::Arc { clockwise, .. } => assert!(clockwise),
                DrawInput::Polyline { vertices, closed } => {
                    assert!(closed);
                    assert_eq!(vertices.len(), 3);
                }
                DrawInput::BSpline {
                    poles,
                    basis,
                    closed,
                } => {
                    assert_eq!(poles.len(), 4);
                    assert_eq!(basis.degree, 3);
                    assert!(!closed);
                }
                _ => {}
            }
        }
        assert!(matches!(
            DrawTool::Arc
                .parse(&["0", "0", "10", "0", "0", "10"], false)
                .unwrap()
                .input,
            DrawInput::Arc {
                clockwise: false,
                ..
            }
        ));
        assert!(matches!(
            DrawTool::Polyline
                .parse(&["0", "0", "10", "0"], false)
                .unwrap()
                .input,
            DrawInput::Polyline { closed: false, .. }
        ));
    }
    #[test]
    fn malformed_counts_flags_and_nonfinite_values_are_rejected_before_dispatch() {
        for (tool, args) in [
            (DrawTool::Line, vec!["0", "0", "1"]),
            (DrawTool::Rectangle, vec!["0", "0", "1", "1", "extra"]),
            (DrawTool::Circle, vec!["0", "0", "NaN"]),
            (
                DrawTool::Spline,
                vec!["0", "0", "0", "30", "40", "30", "closed"],
            ),
            (DrawTool::Arc, vec!["0", "0", "10", "0", "0", "10", "typo"]),
            (DrawTool::Polyline, vec!["0", "0", "10", "0", "closed"]),
            (DrawTool::Polyline, vec!["0", "0", "10", "0", "20"]),
            (DrawTool::Polyline, vec!["0", "0", "inf", "0"]),
        ] {
            assert!(
                tool.parse(&args, false).is_err(),
                "{tool:?} accepted {args:?}"
            );
        }
        for tool in DrawTool::ALL {
            assert!(tool.parse(&[], false).is_err());
        }
    }
}
