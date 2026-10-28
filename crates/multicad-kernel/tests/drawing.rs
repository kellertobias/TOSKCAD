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
        cross: false,
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
            cross: true,
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
                    cross: true,
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
