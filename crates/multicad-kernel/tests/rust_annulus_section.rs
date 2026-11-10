use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    materials::HatchPattern,
};
use multicad_kernel::{
    DrawingHatch, DrawingProjection, DrawingSolid, GeometryErrorKind, PlaneFrame, ProjectedVector,
    Solid, project_drawing,
};

fn stock(frame: PlaneFrame) -> Solid {
    Solid::rust_annulus(2., 5., 8., EntityId::new(), frame, Default::default()).unwrap()
}

fn draw(solid: &Solid, options: DrawingProjection, pattern: HatchPattern) -> Vec<ProjectedVector> {
    project_drawing(
        &[DrawingSolid {
            solid,
            hatch: Some(DrawingHatch {
                angle_rad: 0.13,
                spacing_mm: 0.5,
                line_width_mm: 0.15,
                pattern,
            }),
        }],
        options,
    )
    .unwrap()
}

fn options(frame: PlaneFrame, depth: f64) -> DrawingProjection {
    DrawingProjection {
        frame,
        section_offset_mm: Some(depth),
        section_only: true,
        ..Default::default()
    }
}

#[test]
fn transverse_section_draws_two_exact_circles_and_hatches_only_material() {
    let solid = stock(PlaneFrame {
        origin: [3., -1., 4.],
        ..Default::default()
    });
    let options = options(PlaneFrame::default(), 5.);
    let vectors = draw(&solid, options, HatchPattern::Lines);
    let radii: Vec<_> = vectors
        .iter()
        .filter(|v| v.role == VectorRole::Section)
        .map(|v| {
            let VectorCurve::Ellipse {
                center,
                major_mm,
                minor_mm,
                ..
            } = v.curve
            else {
                panic!("Expected a circle");
            };
            assert_eq!(center, [3., -1.]);
            assert_eq!(major_mm, minor_mm);
            major_mm
        })
        .collect();
    assert_eq!(radii, [5., 2.]);
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
    assert_cut(&vectors, options, [3., -1., 4.]);
    assert!(
        draw(
            &solid,
            DrawingProjection {
                section_offset_mm: Some(15.),
                ..options
            },
            HatchPattern::None
        )
        .is_empty()
    );
}

#[test]
fn longitudinal_sections_keep_the_gap_and_the_clipped_rectangular_regions() {
    let solid = stock(PlaneFrame::default());
    let frame = PlaneFrame {
        v: [0., 0., 1.],
        ..Default::default()
    };
    let options = options(frame, 0.);
    let vectors = draw(&solid, options, HatchPattern::Solid);
    assert_eq!(
        vectors
            .iter()
            .filter(|v| v.role == VectorRole::Section)
            .count(),
        8
    );
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
    assert_cut(&vectors, options, [0.; 3]);
    let clipped = DrawingProjection {
        section_offset_mm: Some(3.),
        section_u_mm: Some([-2., 2.]),
        section_v_mm: Some([1., 4.]),
        ..options
    };
    let vectors = draw(&solid, clipped, HatchPattern::Lines);
    assert_eq!(
        vectors
            .iter()
            .filter(|v| v.role == VectorRole::Section)
            .count(),
        4
    );
    assert_cut(&vectors, clipped, [0.; 3]);
    let tangent = DrawingProjection {
        section_offset_mm: Some(5.),
        ..options
    };
    assert!(draw(&solid, tangent, HatchPattern::None).is_empty());
}

#[test]
fn oblique_section_trims_exact_ellipses_at_stock_caps_and_window_edges() {
    let solid = stock(PlaneFrame::default());
    let frame = PlaneFrame {
        u: [0.6, 0., 0.8],
        ..Default::default()
    };
    let full = options(frame, 2.4);
    let vectors = draw(&solid, full, HatchPattern::Cross);
    assert!(
        vectors
            .iter()
            .any(|v| v.role == VectorRole::Section && matches!(v.curve, VectorCurve::Line { .. }))
    );
    assert!(
        vectors.iter().any(
            |v| v.role == VectorRole::Section && matches!(v.curve, VectorCurve::Ellipse { .. })
        )
    );
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
    assert_cut(&vectors, full, [0.; 3]);
    let clipped = DrawingProjection {
        section_u_mm: Some([0., 5.]),
        section_v_mm: Some([-4., 3.5]),
        ..full
    };
    let vectors = draw(&solid, clipped, HatchPattern::Solid);
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
    assert_cut(&vectors, clipped, [0.; 3]);
}

#[test]
fn annulus_sections_respect_output_budgets() {
    let solid = stock(PlaneFrame::default());
    let err = project_drawing(
        &[DrawingSolid {
            solid: &solid,
            hatch: None,
        }],
        DrawingProjection {
            maximum_curves: 1,
            ..options(PlaneFrame::default(), 4.)
        },
    )
    .unwrap_err();
    assert_eq!(err.kind, GeometryErrorKind::Limit);
}

#[test]
fn section_normalizes_the_accepted_roundoff_in_the_stock_frame() {
    let solid = stock(PlaneFrame {
        u: [1. + 1e-11, 0., 0.],
        ..Default::default()
    });
    let vectors = draw(
        &solid,
        options(PlaneFrame::default(), 4.),
        HatchPattern::None,
    );
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().all(|vector| {
        matches!(vector.curve, VectorCurve::Ellipse { major_mm, minor_mm, .. } if major_mm == minor_mm)
    }));
}

#[test]
fn rotating_oblique_window_and_hatch_follow_the_finite_cut() {
    let solid = stock(PlaneFrame::default());
    for tilt in [0.05_f64, 0.2, 0.5, 0.95, 1.3, 1.53] {
        let (sin, cos) = tilt.sin_cos();
        for angle in [0.11_f64, 0.55, 1.9, 3.2, 5.3] {
            let (s, c) = angle.sin_cos();
            let frame = PlaneFrame {
                u: [c * cos, s, c * sin],
                v: [-s * cos, c, -s * sin],
                ..Default::default()
            };
            for windows in [
                [Some([-1., 3.]), Some([-4., 3.5])],
                [Some([-5., 5.]), None],
                [None, Some([-1., 2.])],
            ] {
                let options = DrawingProjection {
                    section_u_mm: windows[0],
                    section_v_mm: windows[1],
                    ..options(frame, cos * 4.)
                };
                assert_cut(
                    &draw(&solid, options, HatchPattern::Solid),
                    options,
                    [0.; 3],
                );
            }
        }
    }
}

fn assert_cut(vectors: &[ProjectedVector], options: DrawingProjection, base: [f64; 3]) {
    let check = |[u, v]: [f64; 2]| {
        let xy = options.frame.point([u, v]);
        let normal = options.frame.normal();
        let [x, y, z] = std::array::from_fn(|i| {
            xy[i] + normal[i] * options.section_offset_mm.unwrap() - base[i]
        });
        let r = x.hypot(y);
        assert!(
            (2. - 1e-8..=5. + 1e-8).contains(&r),
            "radial sample {r}, drawing point {:?}",
            [u, v]
        );
        assert!((-1e-8..=8. + 1e-8).contains(&z), "axial sample {z}");
        for (x, [a, b]) in [u, v]
            .into_iter()
            .zip([options.section_u_mm, options.section_v_mm])
            .filter_map(|(x, w)| w.map(|w| (x, w)))
        {
            assert!((a - 1e-8..=b + 1e-8).contains(&x));
        }
    };
    for vector in vectors {
        for &point in &vector.preview_mm {
            check(point);
        }
        match (vector.role, &vector.curve) {
            (VectorRole::Hatch, VectorCurve::Line { a, b }) => {
                for i in 1..10 {
                    let t = f64::from(i) / 10.;
                    check(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])));
                }
            }
            (VectorRole::Hatch, VectorCurve::HatchTriangle { points: [a, b, c] }) => {
                for i in 1..10 {
                    for j in 1..10 - i {
                        let t = f64::from(i) / 10.;
                        let s = f64::from(j) / 10.;
                        check(std::array::from_fn(|i| {
                            a[i] + t * (b[i] - a[i]) + s * (c[i] - a[i])
                        }));
                    }
                }
            }
            _ => {}
        }
    }
}
