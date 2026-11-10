use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    references::Origin,
};
use multicad_kernel::{
    CircularPocketTool, DrawingProjection, DrawingSolid, GeometryError, GeometryErrorKind,
    GeometryPolicy, PlaneFrame, ProjectedVector, RoundedSlotTool, Solid, project_drawing,
    rust_kernel::Rational,
};
use std::f64::consts::{PI, TAU};

fn stock() -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [20, 16, 12].map(|value| Rational::new(value, 1).unwrap()),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap()
}

fn origin(role: &str) -> Origin {
    Origin {
        source: EntityId::new(),
        role: role.into(),
    }
}

fn tool_frame(front: bool) -> (PlaneFrame, [f64; 3]) {
    (
        PlaneFrame {
            origin: [0., 0., if front { 13. } else { -1. }],
            ..Default::default()
        },
        [0., 0., if front { -5. } else { 3. }],
    )
}

fn mortise(front: bool, end: f64) -> RoundedSlotTool {
    let (frame, displacement_mm) = tool_frame(front);
    RoundedSlotTool {
        frame,
        centers_uv: [[5., 6.], [end, 6.]],
        radius_mm: 2.,
        displacement_mm,
        origin: origin("mortise"),
    }
}

fn bore(front: bool, center_uv: [f64; 2], radius_mm: f64) -> CircularPocketTool {
    let (frame, displacement_mm) = tool_frame(front);
    CircularPocketTool {
        frame,
        center_uv,
        radius_mm,
        displacement_mm,
        origin: origin("bore"),
    }
}

fn draw(solid: &Solid, options: DrawingProjection) -> Result<Vec<ProjectedVector>, GeometryError> {
    project_drawing(&[input(solid)], options)
}

fn input(solid: &Solid) -> DrawingSolid<'_> {
    DrawingSolid { solid, hatch: None }
}

fn with_front_mortise() -> Solid {
    stock()
        .rust_drill_rounded_slots(&[mortise(true, 9.)], GeometryPolicy::default())
        .unwrap()
}

#[test]
fn parallel_bores_and_mortise_publish_exact_rims_and_no_floor_duplicates() {
    let solid = with_front_mortise()
        .rust_drill_circular_pockets(
            &[bore(true, [14., 7.], 1.), bore(false, [13., 12.], 1.2)],
            GeometryPolicy::default(),
        )
        .unwrap();
    let vectors = draw(&solid, DrawingProjection::default()).unwrap();
    assert_eq!(vectors.len(), 9);
    assert!(vectors.iter().all(|v| v.role == VectorRole::Visible));
    assert_eq!(
        vectors
            .iter()
            .filter(|v| matches!(v.curve, VectorCurve::Line { .. }))
            .count(),
        6
    );

    let mut curves = vectors
        .iter()
        .filter_map(|vector| {
            let VectorCurve::Ellipse {
                center,
                u,
                v,
                major_mm,
                minor_mm,
                start_rad,
                end_rad,
            } = vector.curve
            else {
                return None;
            };
            assert_eq!(major_mm, minor_mm);
            assert_eq!(start_rad, 0.);
            assert_eq!(v, [-u[1], u[0]]);
            for point in &vector.preview_mm {
                assert!(
                    ((point[0] - center[0]).hypot(point[1] - center[1]) - major_mm).abs() < 1e-12
                );
            }
            if major_mm == 2. {
                // Both arcs have vertical tangencies and a convex outer half.
                assert!(u[0].abs() < 1e-12);
                assert_eq!(u[1], if center[0] == 5. { 1. } else { -1. });
            }
            Some((center, major_mm, end_rad))
        })
        .collect::<Vec<_>>();
    curves.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    assert_eq!(
        curves,
        [([5., 6.], 2., PI), ([9., 6.], 2., PI), ([14., 7.], 1., TAU)]
    );

    let back = DrawingProjection {
        frame: PlaneFrame {
            v: [0., -1., 0.],
            ..Default::default()
        },
        hidden_lines: true,
        ..Default::default()
    };
    let vectors = draw(&solid, back).unwrap();
    assert_eq!(vectors.len(), 10);
    assert_eq!(
        vectors
            .iter()
            .filter(|v| v.role == VectorRole::Hidden)
            .count(),
        5
    );
    assert!(vectors.iter().any(|v| {
        v.role == VectorRole::Visible
            && matches!(
                v.curve,
                VectorCurve::Ellipse {
                    center: [13., -12.],
                    major_mm: 1.2,
                    end_rad: TAU,
                    ..
                }
            )
    }));
    assert_eq!(
        draw(
            &solid,
            DrawingProjection {
                hidden_lines: false,
                ..back
            }
        )
        .unwrap()
        .len(),
        5
    );
}

#[test]
fn opposite_matching_mortises_publish_one_wire_under_a_rotated_view() {
    let solid = with_front_mortise()
        .rust_drill_rounded_slots(&[mortise(false, 9.)], GeometryPolicy::default())
        .unwrap();
    let options = DrawingProjection {
        frame: PlaneFrame {
            origin: [1., 2., 3.],
            u: [0.6, 0.8, 0.],
            v: [-0.8, 0.6, 0.],
        },
        hidden_lines: true,
        ..Default::default()
    };
    let vectors = draw(&solid, options).unwrap();
    assert_eq!(vectors.len(), 8);
    assert!(vectors.iter().all(|v| v.role == VectorRole::Visible));
    assert!(vectors.iter().any(
        |v| matches!(v.curve, VectorCurve::Ellipse { center, major_mm: 2., .. }
        if (center[0] - 5.6).abs() < 1e-12 && (center[1] + 0.8).abs() < 1e-12)
    ));

    let partial = with_front_mortise()
        .rust_drill_rounded_slots(&[mortise(false, 11.)], GeometryPolicy::default())
        .unwrap();
    assert_eq!(
        draw(&partial, options).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(
        draw(
            &partial,
            DrawingProjection {
                hidden_lines: false,
                ..options
            }
        )
        .unwrap()
        .len(),
        8
    );
}

#[test]
fn unsupported_view_types_and_output_limits_keep_typed_failures() {
    let solid = with_front_mortise();
    for options in [
        DrawingProjection {
            frame: PlaneFrame {
                u: [0.8, 0., 0.6],
                ..Default::default()
            },
            ..Default::default()
        },
        DrawingProjection {
            section_offset_mm: Some(9.),
            section_only: true,
            frame: PlaneFrame {
                u: [0.8, 0., 0.6],
                ..Default::default()
            },
            ..Default::default()
        },
        DrawingProjection {
            detail: Some(([6., 6.], 5.)),
            ..Default::default()
        },
    ] {
        assert_eq!(
            draw(&solid, options).unwrap_err().kind,
            GeometryErrorKind::Unsupported
        );
    }
    assert_eq!(
        project_drawing(&[input(&solid), input(&stock())], Default::default())
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
    for options in [
        DrawingProjection {
            maximum_curves: 7,
            ..Default::default()
        },
        DrawingProjection {
            maximum_points: 3,
            ..Default::default()
        },
    ] {
        assert_eq!(
            draw(&solid, options).unwrap_err().kind,
            GeometryErrorKind::Limit
        );
    }
}

#[test]
fn exact_section_retains_both_recess_profiles_and_hatches_only_material() {
    use multicad_core::materials::HatchPattern;
    use multicad_kernel::DrawingHatch;
    let solid = with_front_mortise()
        .rust_drill_circular_pockets(&[bore(true, [14., 7.], 1.)], GeometryPolicy::default())
        .unwrap();
    let options = DrawingProjection {
        section_offset_mm: Some(9.),
        section_only: true,
        ..Default::default()
    };
    let vectors = project_drawing(
        &[DrawingSolid {
            solid: &solid,
            hatch: Some(DrawingHatch {
                angle_rad: 0.,
                spacing_mm: 1.,
                line_width_mm: 0.18,
                pattern: HatchPattern::Lines,
            }),
        }],
        options,
    )
    .unwrap();
    let arcs: Vec<_> = vectors
        .iter()
        .filter_map(|vector| match (vector.role, &vector.curve) {
            (
                VectorRole::Section,
                VectorCurve::Ellipse {
                    major_mm,
                    start_rad,
                    end_rad,
                    ..
                },
            ) => Some((*major_mm, end_rad - start_rad)),
            _ => None,
        })
        .collect();
    assert_eq!(arcs.len(), 3);
    assert!(arcs.contains(&(1., TAU)));
    assert_eq!(arcs.iter().filter(|&&arc| arc == (2., PI)).count(), 2);
    let mut hatches = 0;
    for vector in vectors
        .iter()
        .filter(|vector| vector.role == VectorRole::Hatch)
    {
        let VectorCurve::Line { a, b } = vector.curve else {
            panic!("Expected a hatch line")
        };
        hatches += 1;
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            let [x, y] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
            assert!((x - x.clamp(5., 9.)).hypot(y - 6.) >= 2. - 1e-6);
            assert!((x - 14.).hypot(y - 7.) >= 1. - 1e-6);
        }
    }
    assert!(hatches > 16);
}

#[test]
fn retained_section_suppresses_a_rear_cut_only_with_complete_opaque_cover() {
    let solid = with_front_mortise();
    let policy = GeometryPolicy::default();
    let cover = |width| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            [width, 6, 2].map(|value| Rational::new(value, 1).unwrap()),
            PlaneFrame {
                origin: [2., 3., 12.5],
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let options = DrawingProjection {
        section_offset_mm: Some(13.),
        section_depth_mm: Some(7.),
        ..Default::default()
    };
    let visible = project_drawing(&[input(&solid), input(&cover(10))], options).unwrap();
    assert!(
        visible
            .iter()
            .all(|vector| matches!(vector.curve, VectorCurve::Line { .. }))
    );
    assert!(
        visible
            .iter()
            .any(|vector| vector.source_index == 1 && vector.role == VectorRole::Section)
    );
    let partial = project_drawing(&[input(&solid), input(&cover(4))], options).unwrap();
    let mut sweep = 0.;
    for vector in &partial {
        if let VectorCurve::Ellipse {
            center,
            major_mm,
            minor_mm,
            start_rad,
            end_rad,
            ..
        } = vector.curve
        {
            assert_eq!((vector.source_index, vector.role), (0, VectorRole::Visible));
            assert_eq!((center, major_mm, minor_mm), ([9., 6.], 2., 2.));
            assert!(vector.preview_mm.iter().all(|point| point[0] >= 9. - 1e-8));
            sweep += end_rad - start_rad;
        }
    }
    assert!((sweep - PI).abs() < 1e-8);
}
