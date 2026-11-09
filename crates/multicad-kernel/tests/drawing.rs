use multicad_core::drawings::{VectorCurve, VectorRole};
use multicad_kernel::*;
fn options() -> DrawingProjection {
    DrawingProjection {
        frame: PlaneFrame {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        hidden_lines: true,
        ..Default::default()
    }
}
fn limits(v: &[ProjectedVector]) -> ([f64; 2], [f64; 2]) {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in v.iter().flat_map(|v| &v.preview_mm) {
        for i in 0..2 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    (min, max)
}
#[test]
fn real_brep_projection_has_model_dimensions_hidden_lines_and_source_attribution() {
    let box_ = Solid::box_solid([30., 20., 10.], Default::default()).unwrap();
    let v = project_drawing(
        &[DrawingSolid {
            solid: &box_,
            hatch: None,
        }],
        options(),
    )
    .unwrap();
    assert!(v.iter().all(|v| v.source_index == 0));
    assert!(v.iter().any(|v| v.role == VectorRole::Visible));
    let (min, max) = limits(&v);
    assert!((min[0]).abs() < 1e-6);
    assert!((min[1]).abs() < 1e-6);
    assert!((max[0] - 30.).abs() < 1e-6);
    assert!((max[1] - 10.).abs() < 1e-6);
    // An oblique projection separates rear edges from the visible edges.
    let iso = DrawingProjection {
        frame: PlaneFrame {
            origin: [0.; 3],
            u: [1. / 2f64.sqrt(), -1. / 2f64.sqrt(), 0.],
            v: [1. / 6f64.sqrt(), 1. / 6f64.sqrt(), -2. / 6f64.sqrt()],
        },
        ..options()
    };
    let a = project_drawing(
        &[DrawingSolid {
            solid: &box_,
            hatch: None,
        }],
        iso,
    )
    .unwrap();
    assert!(a.iter().any(|v| v.role == VectorRole::Hidden));
    let b = project_drawing(
        &[DrawingSolid {
            solid: &box_,
            hatch: None,
        }],
        DrawingProjection {
            hidden_lines: false,
            ..iso
        },
    )
    .unwrap();
    assert!(!b.iter().any(|v| v.role == VectorRole::Hidden));
    assert!(b.len() < a.len());
}
#[test]
fn actual_cylinder_projection_retains_exact_circle_and_elliptic_silhouette() {
    let cylinder = Solid::cylinder(PlaneFrame::default(), 10., 20., Default::default()).unwrap();
    let top = project_drawing(
        &[DrawingSolid {
            solid: &cylinder,
            hatch: None,
        }],
        DrawingProjection::default(),
    )
    .unwrap();
    assert!(top.iter().any(|c|matches!(&c.curve,VectorCurve::Ellipse{major_mm,minor_mm,..} if (*major_mm-10.).abs()<1e-6 && (*minor_mm-10.).abs()<1e-6)));
    let s = 2f64.sqrt().recip();
    let oblique = project_drawing(
        &[DrawingSolid {
            solid: &cylinder,
            hatch: None,
        }],
        DrawingProjection {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., s, s],
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(oblique.iter().any(
        |v| matches!(&v.curve,VectorCurve::Ellipse{major_mm,minor_mm,..} if major_mm>minor_mm)
    ));
}
#[test]
fn section_hatches_only_cut_material_and_respects_a_real_hole() {
    use multicad_core::materials::HatchPattern;
    let outer = Solid::box_solid([40., 30., 20.], Default::default()).unwrap();
    let tool = Solid::cylinder(
        PlaneFrame {
            origin: [20., 15., -1.],
            ..Default::default()
        },
        5.,
        22.,
        Default::default(),
    )
    .unwrap();
    let hollow = outer
        .boolean(&tool, BooleanOperation::Subtract, Default::default())
        .unwrap()
        .unwrap();
    let behind = Solid::box_solid([6., 6., 2.], Default::default()).unwrap();
    let hatch = DrawingHatch {
        angle_rad: 0.,
        spacing_mm: 2.,
        line_width_mm: 0.18,
        pattern: multicad_core::materials::HatchPattern::Lines,
    };
    let v = project_drawing(
        &[
            DrawingSolid {
                solid: &hollow,
                hatch: Some(hatch),
            },
            DrawingSolid {
                solid: &behind,
                hatch: Some(hatch),
            },
        ],
        DrawingProjection {
            section_offset_mm: Some(10.),
            ..Default::default()
        },
    )
    .unwrap();
    let lines: Vec<_> = v.iter().filter(|v| v.role == VectorRole::Hatch).collect();
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|v| v.source_index == 0));
    assert!(v.iter().any(|v| v.role == VectorRole::Section));
    for line in lines {
        let VectorCurve::Line { a, b } = line.curve else {
            panic!("Hatch must remain exact line")
        };
        for t in [0.25, 0.5, 0.75] {
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            assert!((p[0] - 20.).hypot(p[1] - 15.) >= 5. - 1e-6);
        }
    }
    for pattern in [
        HatchPattern::Alternating,
        HatchPattern::Dots,
        HatchPattern::SmallCrosses,
        HatchPattern::Solid,
    ] {
        let result = project_drawing(
            &[DrawingSolid {
                solid: &hollow,
                hatch: Some(DrawingHatch { pattern, ..hatch }),
            }],
            DrawingProjection {
                section_offset_mm: Some(10.),
                ..Default::default()
            },
        )
        .unwrap();
        let hatches: Vec<_> = result
            .iter()
            .filter(|v| v.role == VectorRole::Hatch)
            .collect();
        assert!(!hatches.is_empty(), "{pattern:?}");
        if pattern == HatchPattern::Solid {
            let area: f64 = hatches
                .iter()
                .map(|v| {
                    let VectorCurve::HatchTriangle { points: [a, b, c] } = v.curve else {
                        panic!("solid hatch should emit filled faces");
                    };
                    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
                })
                .sum();
            assert!(
                (area - (40. * 30. - 25. * std::f64::consts::PI)).abs() < 2.,
                "{area}"
            );
        } else {
            for vector in hatches {
                let VectorCurve::Line { a, b } = vector.curve else {
                    panic!("expected a clipped stroke")
                };
                let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                assert!((midpoint[0] - 20.).hypot(midpoint[1] - 15.) >= 5. - 1e-6);
            }
        }
    }
}
#[test]
fn detail_crops_exact_curves_and_limits_fail_explicitly() {
    let solid = Solid::box_solid([40., 30., 20.], Default::default()).unwrap();
    let input = [DrawingSolid {
        solid: &solid,
        hatch: None,
    }];
    let detail = project_drawing(
        &input,
        DrawingProjection {
            detail: Some(([0., 15.], 8.)),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!detail.is_empty());
    assert!(
        detail
            .iter()
            .flat_map(|v| &v.preview_mm)
            .all(|p| p[0].hypot(p[1] - 15.) <= 8. + 1e-6)
    );
    assert!(
        project_drawing(
            &input,
            DrawingProjection {
                maximum_curves: 1,
                ..options()
            }
        )
        .is_err()
    );
    assert!(
        project_drawing(
            &input,
            DrawingProjection {
                chord_tolerance_mm: f64::NAN,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn section_can_remove_all_sources_and_uncut_sections_do_not_generate_hatches() {
    let solid = Solid::box_solid([10., 10., 10.], Default::default()).unwrap();
    let input = [DrawingSolid {
        solid: &solid,
        hatch: Some(DrawingHatch {
            angle_rad: 0.3,
            spacing_mm: 2.,
            line_width_mm: 0.18,
            pattern: multicad_core::materials::HatchPattern::Cross,
        }),
    }];
    assert!(
        project_drawing(
            &input,
            DrawingProjection {
                section_offset_mm: Some(-5.),
                ..Default::default()
            }
        )
        .unwrap()
        .is_empty()
    );
    let uncut = project_drawing(
        &input,
        DrawingProjection {
            section_offset_mm: Some(15.),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        uncut
            .iter()
            .all(|v| !matches!(v.role, VectorRole::Hatch | VectorRole::Section))
    );
}

#[test]
fn oblique_triangular_section_hatches_in_its_actual_plane_and_clips_a_real_hole() {
    let body = Solid::box_solid([10.; 3], Default::default()).unwrap();
    let s = 2f64.sqrt().recip();
    let t = 6f64.sqrt().recip();
    let n = 3f64.sqrt().recip();
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [s, -s, 0.],
        v: [t, t, -2. * t],
    };
    // x+y+z=1 cuts a triangle with world AABB center (.5,.5,.5),
    // which is not on that plane. A circular bore crosses its centroid.
    let bore = Solid::cylinder(
        PlaneFrame {
            origin: [1. / 3. - n; 3],
            ..frame
        },
        0.1,
        20.,
        Default::default(),
    )
    .unwrap();
    let hollow = body
        .boolean(&bore, BooleanOperation::Subtract, Default::default())
        .unwrap()
        .unwrap();
    for (solid, hole) in [(&body, false), (&hollow, true)] {
        let vectors = project_drawing(
            &[DrawingSolid {
                solid,
                hatch: Some(DrawingHatch {
                    angle_rad: 0.37,
                    spacing_mm: 0.05,
                    line_width_mm: 0.18,
                    pattern: multicad_core::materials::HatchPattern::Cross,
                }),
            }],
            DrawingProjection {
                frame,
                section_offset_mm: Some(n),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(vectors.iter().any(|v| v.role == VectorRole::Section));
        let hatches: Vec<_> = vectors
            .iter()
            .filter(|v| v.role == VectorRole::Hatch)
            .collect();
        assert!(
            hatches.len() > 5,
            "An oblique cut must not silently lose all its hatching"
        );
        for hatch in hatches {
            let VectorCurve::Line { a, b } = hatch.curve else {
                panic!("Hatches are exact lines");
            };
            for q in [0., 0.25, 0.5, 0.75, 1.] {
                let p = [a[0] + (b[0] - a[0]) * q, a[1] + (b[1] - a[1]) * q];
                let world = [
                    1. / 3. + s * p[0] + t * p[1],
                    1. / 3. - s * p[0] + t * p[1],
                    1. / 3. - 2. * t * p[1],
                ];
                assert!(
                    world.iter().all(|v| *v >= -1e-6 && *v <= 1. + 1e-6),
                    "Hatch escaped triangular cut: {world:?}"
                );
                assert!((world.iter().sum::<f64>() - 1.).abs() < 1e-7);
                if hole {
                    assert!(
                        p[0].hypot(p[1]) >= 0.1 - 1e-6,
                        "Hatch crosses the real bore"
                    );
                }
            }
        }
    }
}

/// Solids along the view direction of a front section at y = 0 (the view
/// looks along +Y and keeps y >= 0): A straddles the cut, B lies within a
/// 30 mm depth, C is a ramp crossing the far limit at y = 30 whose height
/// grows with y (z <= y - 20), and D lies wholly beyond it.
fn range_fixture() -> Vec<Solid> {
    let placed = |size: [f64; 3], at: [f64; 3]| {
        Solid::box_solid(size, Default::default())
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: at,
                ..Default::default()
            })
            .unwrap()
    };
    let s = 2f64.sqrt().recip();
    // Local (a, b, c) maps to world x = a, y = s(b + c), z = s(c - b); local
    // b >= 20s is exactly the half-space z <= y - 20.
    let keep = Solid::box_solid([300.; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., s, s], [0., -s, s]],
            translation_mm: [-100., s * (20. * s - 150.), s * (-150. - 20. * s)],
        })
        .unwrap();
    let ramp = placed([10., 20., 20.], [40., 20., 0.])
        .boolean(&keep, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .unwrap();
    // Analytic ramp volume: 10 * (20 * 20 / 2).
    assert!((ramp.volume_mm3().unwrap() - 2000.).abs() < 1e-6);
    vec![
        placed([10., 10., 10.], [0., -5., 0.]),
        placed([10., 10., 10.], [20., 10., 0.]),
        ramp,
        placed([10., 10., 10.], [60., 50., 0.]),
    ]
}
fn front_section() -> DrawingProjection {
    DrawingProjection {
        frame: PlaneFrame {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        section_offset_mm: Some(0.),
        ..Default::default()
    }
}
fn run(solids: &[Solid], options: DrawingProjection) -> Vec<ProjectedVector> {
    let hatch = DrawingHatch {
        angle_rad: std::f64::consts::FRAC_PI_4,
        spacing_mm: 2.,
        line_width_mm: 0.18,
        pattern: multicad_core::materials::HatchPattern::Lines,
    };
    let inputs: Vec<_> = solids
        .iter()
        .map(|solid| DrawingSolid {
            solid,
            hatch: Some(hatch),
        })
        .collect();
    project_drawing(&inputs, options).unwrap()
}
fn sources(v: &[ProjectedVector]) -> std::collections::BTreeSet<usize> {
    v.iter().map(|v| v.source_index).collect()
}
fn of(v: &[ProjectedVector], source: usize) -> Vec<ProjectedVector> {
    v.iter()
        .filter(|v| v.source_index == source)
        .cloned()
        .collect()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn section_only_draws_the_exact_plane_intersection_and_nothing_behind_it() {
    let solids = range_fixture();
    let v = run(
        &solids,
        DrawingProjection {
            section_only: true,
            hidden_lines: true,
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [0].into());
    assert!(
        v.iter()
            .all(|v| matches!(v.role, VectorRole::Section | VectorRole::Hatch))
    );
    let outline: Vec<_> = v
        .iter()
        .filter(|v| v.role == VectorRole::Section)
        .cloned()
        .collect();
    assert_eq!(outline.len(), 4, "the 10 x 10 cut rectangle");
    let (min, max) = limits(&outline);
    near(min[0], 0.);
    near(min[1], 0.);
    near(max[0], 10.);
    near(max[1], 10.);
    let hatch: Vec<_> = v
        .iter()
        .filter(|v| v.role == VectorRole::Hatch)
        .cloned()
        .collect();
    assert!(!hatch.is_empty());
    let (min, max) = limits(&hatch);
    assert!(min[0] >= -1e-7 && min[1] >= -1e-7 && max[0] <= 10. + 1e-7 && max[1] <= 10. + 1e-7);
    // The same plane well inside B cuts only B.
    let v = run(
        &solids,
        DrawingProjection {
            section_only: true,
            section_offset_mm: Some(-15.),
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [1].into());
}

#[test]
fn finite_depth_keeps_the_range_clips_at_the_far_plane_and_excludes_farther_solids() {
    let solids = range_fixture();
    let unlimited = run(&solids, front_section());
    assert_eq!(sources(&unlimited), [0, 1, 2, 3].into());
    near(limits(&of(&unlimited, 2)).1[1], 20.);

    let v = run(
        &solids,
        DrawingProjection {
            section_depth_mm: Some(30.),
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [0, 1, 2].into(), "D lies beyond the range");
    // Only the straddling solid has a cut face and hatching.
    for source in [1, 2] {
        assert!(of(&v, source).iter().all(|v| v.role == VectorRole::Visible));
    }
    assert!(of(&v, 0).iter().any(|v| v.role == VectorRole::Section));
    assert!(of(&v, 0).iter().any(|v| v.role == VectorRole::Hatch));
    // The ramp is cut at y = 30, where its height is 30 - 20 = 10.
    let (min, max) = limits(&of(&v, 2));
    near(min[0], 40.);
    near(max[0], 50.);
    near(max[1], 10.);
    let (min, max) = limits(&of(&v, 1));
    near(min[0], 20.);
    near(max[0], 30.);
    near(max[1], 10.);
    // A depth ending inside B clips it but keeps it; one ending before it
    // excludes it.
    let v = run(
        &solids,
        DrawingProjection {
            section_depth_mm: Some(12.),
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [0, 1].into());
    let v = run(
        &solids,
        DrawingProjection {
            section_depth_mm: Some(8.),
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [0].into());
}

#[test]
fn lateral_windows_limit_a_section_to_a_finite_line() {
    let solids = range_fixture();
    let v = run(
        &solids,
        DrawingProjection {
            section_u_mm: Some([15., 35.]),
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [1].into());
    let v = run(
        &solids,
        DrawingProjection {
            section_u_mm: Some([25., 100.]),
            section_v_mm: Some([-1., 5.]),
            ..front_section()
        },
    );
    assert_eq!(sources(&v), [1, 2, 3].into());
    let (min, max) = limits(&of(&v, 1));
    near(min[0], 25.);
    near(max[1], 5.);
    let empty = run(
        &solids,
        DrawingProjection {
            section_only: true,
            section_u_mm: Some([20., 30.]),
            ..front_section()
        },
    );
    assert!(
        empty.is_empty(),
        "no solid crosses the plane in this window"
    );
}

#[test]
fn inconsistent_section_ranges_are_refused() {
    let solids = range_fixture();
    let inputs = [DrawingSolid {
        solid: &solids[0],
        hatch: None,
    }];
    for options in [
        DrawingProjection {
            section_only: true,
            ..Default::default()
        },
        DrawingProjection {
            section_only: true,
            section_depth_mm: Some(5.),
            ..front_section()
        },
        DrawingProjection {
            section_depth_mm: Some(0.),
            ..front_section()
        },
        DrawingProjection {
            section_u_mm: Some([5., 1.]),
            ..front_section()
        },
        DrawingProjection {
            section_v_mm: Some([0., f64::NAN]),
            ..front_section()
        },
    ] {
        assert!(project_drawing(&inputs, options).is_err());
    }
}
