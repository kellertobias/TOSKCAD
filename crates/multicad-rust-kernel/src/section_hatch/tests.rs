use super::*;

const OPTIONS: Options = Options {
    angle_rad: 0.,
    spacing_mm: 1.,
    line_width_mm: 0.18,
    tolerance_mm: 1e-7,
    stroke_limit: 20_000,
};

fn ring(min: Point, max: Point) -> [Segment; 4] {
    let points = [min, [max[0], min[1]], max, [min[0], max[1]]];
    std::array::from_fn(|i| [points[i], points[(i + 1) % 4]])
}

fn lines(strokes: Vec<Stroke>) -> Vec<Segment> {
    strokes
        .into_iter()
        .map(|stroke| match stroke {
            Stroke::Line(edge) => edge,
            Stroke::Triangle(_) => panic!("unexpected filled triangle"),
        })
        .collect()
}

fn on_material(point: Point) -> bool {
    let [x, y] = point;
    (-1e-7..=20. + 1e-7).contains(&x)
        && (-1e-7..=12. + 1e-7).contains(&y)
        && !((6. + 1e-7..14. - 1e-7).contains(&x) && (3. + 1e-7..9. - 1e-7).contains(&y))
}

fn sample(edge: Segment, t: f64) -> Point {
    std::array::from_fn(|i| (edge[1][i] - edge[0][i]).mul_add(t, edge[0][i]))
}

#[test]
fn all_stroke_styles_clip_around_a_hole_and_retain_their_distinct_marks() {
    let boundary = [ring([0., 0.], [20., 12.]), ring([6., 3.], [14., 9.])].concat();
    let patterns = [
        Pattern::Lines,
        Pattern::Cross,
        Pattern::Alternating,
        Pattern::Dots,
        Pattern::SmallCrosses,
        Pattern::Brick,
        Pattern::Concrete,
        Pattern::Grass,
    ];
    let mut definitions = Vec::new();
    for pattern in patterns {
        let output = lines(hatch(&boundary, pattern, OPTIONS).unwrap());
        assert!(!output.is_empty(), "{pattern:?}");
        assert!(!definitions.contains(&output), "{pattern:?}");
        for edge in &output {
            assert!(
                [0., 0.25, 0.5, 0.75, 1.]
                    .into_iter()
                    .all(|t| on_material(sample(*edge, t))),
                "{pattern:?} {edge:?}"
            );
        }
        definitions.push(output);
    }
    let horizontal = &definitions[0];
    assert!(horizontal.contains(&[[0., 6.], [6., 6.]]));
    assert!(horizontal.contains(&[[14., 6.], [20., 6.]]));
    assert!(!horizontal.contains(&[[0., 6.], [20., 6.]]));
    let delta = |edge: &Segment| sub(edge[1], edge[0]);
    let has_vertical = |edges: &[Segment]| {
        edges
            .iter()
            .map(delta)
            .any(|[x, y]| x.abs() < 1e-7 && y.abs() > 1e-7)
    };
    let has_long = |edges: &[Segment]| edges.iter().map(delta).any(|[x, y]| x.hypot(y) > 2.);
    assert!(has_vertical(&definitions[1]));
    assert!(has_vertical(&definitions[4]));
    assert!(has_vertical(&definitions[5]));
    assert!(!has_vertical(&definitions[3]));
    assert!(has_long(&definitions[2]));
    assert!(!has_long(&definitions[3]));
    assert!(!has_long(&definitions[6]));
    assert!(!has_long(&definitions[7]));
}

#[test]
fn rotated_line_hatch_and_touching_components_keep_exact_boundaries() {
    let boundary = [ring([0., 0.], [2., 2.]), ring([2., 2.], [4., 4.])].concat();
    let angle = std::f64::consts::FRAC_PI_4;
    let output = lines(
        hatch(
            &boundary,
            Pattern::Lines,
            Options {
                angle_rad: angle,
                ..OPTIONS
            },
        )
        .unwrap(),
    );
    assert!(output.iter().any(|edge| {
        (edge[0][0].abs() + edge[0][1].abs()).abs() < 1e-7
            && (edge[1][0] - 4.).abs() + (edge[1][1] - 4.).abs() < 1e-7
    }));
    for edge in &output {
        let [dx, dy] = sub(edge[1], edge[0]);
        assert!((dx - dy).abs() < 1e-7);
    }
    let triangles = hatch(&boundary, Pattern::Solid, OPTIONS).unwrap();
    assert_eq!(triangles.len(), 4);
}

#[test]
fn constrained_fill_preserves_area_for_holes_nested_islands_and_disjoint_parts() {
    let boundary = [
        ring([0., 0.], [20., 12.]),
        ring([6., 3.], [14., 9.]),
        ring([8., 4.], [11., 6.]),
        ring([22., 1.], [25., 4.]),
    ]
    .concat();
    let fill = hatch(&boundary, Pattern::Solid, OPTIONS).unwrap();
    let area: f64 = fill
        .iter()
        .map(|stroke| {
            let Stroke::Triangle([a, b, c]) = *stroke else {
                panic!("expected fill");
            };
            let [u, v] = sub(b, a);
            let [x, y] = sub(c, a);
            (u * y - v * x).abs() / 2.
        })
        .sum();
    assert_eq!(area, 240. - 48. + 6. + 9.);
}

#[test]
fn invalid_boundaries_and_limits_return_errors_without_partial_output() {
    let square = ring([0., 0.], [20., 12.]);
    assert!(hatch(&square, Pattern::None, OPTIONS).unwrap().is_empty());
    assert!(hatch(&[], Pattern::Lines, OPTIONS).unwrap().is_empty());
    assert_eq!(
        hatch(
            &square,
            Pattern::Dots,
            Options {
                line_width_mm: f64::NAN,
                ..OPTIONS
            }
        ),
        Err(HatchError::Invalid)
    );
    assert!(
        hatch(
            &square,
            Pattern::Lines,
            Options {
                line_width_mm: 0.,
                ..OPTIONS
            }
        )
        .is_ok()
    );
    assert_eq!(
        hatch(&square[..3], Pattern::Lines, OPTIONS),
        Err(HatchError::Invalid)
    );
    let overlap = [square, ring([6., 3.], [24., 9.])].concat();
    assert_eq!(
        hatch(&overlap, Pattern::Solid, OPTIONS),
        Err(HatchError::Invalid)
    );
    let mut duplicate = square.to_vec();
    duplicate.push(square[0].map(|p| p.map(|x| if x == 0. { -0. } else { x })));
    assert_eq!(
        hatch(&duplicate, Pattern::Solid, OPTIONS),
        Err(HatchError::Invalid)
    );
    assert_eq!(
        hatch(&[[[0., 0.]; 2]], Pattern::Lines, OPTIONS),
        Err(HatchError::Invalid)
    );
    let too_many = vec![square[0]; MAX_BOUNDARY + 1];
    assert_eq!(
        hatch(&too_many, Pattern::Lines, OPTIONS),
        Err(HatchError::Limit)
    );
    for pattern in [Pattern::Lines, Pattern::Solid] {
        assert_eq!(
            hatch(
                &square,
                pattern,
                Options {
                    stroke_limit: 1,
                    ..OPTIONS
                }
            ),
            Err(HatchError::Limit)
        );
    }
    assert_eq!(
        hatch(
            &square,
            Pattern::Lines,
            Options {
                spacing_mm: 1e-5,
                ..OPTIONS
            }
        ),
        Err(HatchError::Limit)
    );
    let square = ring([0., 0.], [500., 500.]);
    assert_eq!(
        hatch(&square, Pattern::Grass, OPTIONS),
        Err(HatchError::Limit)
    );
    let mut state = Some(true);
    assert_eq!(assign(&mut state, false), Err(HatchError::Invalid));
}

#[test]
fn candidate_and_boundary_work_are_independently_bounded() {
    let boundary = ring([0., 0.], [20., 12.]);
    let mut budget = Budget {
        candidates: 2,
        work: 3,
    };
    let mut output = Vec::new();
    let mut clipper = Clipper {
        boundary: &boundary,
        center: [0., 0.],
        axes: [[1., 0.], [0., 1.]],
        tolerance: OPTIONS.tolerance_mm,
        stroke_limit: OPTIONS.stroke_limit,
        crossings: Vec::new(),
        spans: Vec::new(),
        output: &mut output,
        budget: &mut budget,
    };
    assert_eq!(clipper.line([[1., 1.]; 2]), Ok(()));
    assert_eq!(clipper.line(boundary[0]), Err(HatchError::Limit));
    assert_eq!(clipper.line(boundary[1]), Err(HatchError::Limit));
    assert!(output.is_empty());
}
