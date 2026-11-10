use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    materials::HatchPattern,
};
use multicad_kernel::{
    DrawingHatch, DrawingProjection, DrawingSolid, GeometryErrorKind, PlaneFrame, Solid,
    project_drawing,
};
use std::f64::consts::TAU;

fn sphere() -> Solid {
    Solid::rust_sphere_named(
        5.,
        PlaneFrame {
            origin: [7., -2., 11.],
            ..PlaneFrame::default()
        },
        EntityId::new(),
        Default::default(),
    )
    .unwrap()
}

#[test]
fn sphere_views_publish_one_exact_silhouette_and_clip_details() {
    let solid = sphere();
    let frame = PlaneFrame {
        origin: [2., 3., 4.],
        u: [0., 0., 1.],
        v: [1., 0., 0.],
    };
    let inputs = [DrawingSolid {
        solid: &solid,
        hatch: None,
    }];
    let options = DrawingProjection {
        frame,
        hidden_lines: true,
        tangent_edges: true,
        ..Default::default()
    };
    let output = project_drawing(&inputs, options).unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].role, VectorRole::Visible);
    assert_eq!(
        output[0].curve,
        VectorCurve::Ellipse {
            center: [7., 5.],
            u: [1., 0.],
            v: [0., 1.],
            major_mm: 5.,
            minor_mm: 5.,
            start_rad: 0.,
            end_rad: TAU,
        }
    );

    let detail = project_drawing(
        &inputs,
        DrawingProjection {
            detail: Some(([12., 5.], 2.)),
            ..options
        },
    )
    .unwrap();
    assert_eq!(detail.len(), 2);
    assert!(detail.iter().flat_map(|v| &v.preview_mm).all(|p| {
        ((p[0] - 7.).hypot(p[1] - 5.) - 5.).abs() < 1e-10
            && (p[0] - 12.).hypot(p[1] - 5.) <= 2. + 1e-10
    }));
}

#[test]
fn sphere_sections_publish_the_cut_disk_and_any_larger_retained_silhouette() {
    let solid = sphere();
    let hatch = DrawingHatch {
        angle_rad: 0.,
        spacing_mm: 0.5,
        line_width_mm: 0.18,
        pattern: HatchPattern::Lines,
    };
    let inputs = [DrawingSolid {
        solid: &solid,
        hatch: Some(hatch),
    }];
    let options = DrawingProjection {
        section_offset_mm: Some(14.),
        ..Default::default()
    };
    let output = project_drawing(&inputs, options).unwrap();
    assert!(output.iter().any(|v| v.role == VectorRole::Section
        && matches!(
            v.curve,
            VectorCurve::Ellipse {
                center: [7., -2.],
                major_mm: 4.,
                minor_mm: 4.,
                ..
            }
        )));
    assert!(output.iter().any(|v| v.role == VectorRole::Visible
        && matches!(
            v.curve,
            VectorCurve::Ellipse {
                major_mm: 5.,
                minor_mm: 5.,
                ..
            }
        )));
    let hatches: Vec<_> = output
        .iter()
        .filter(|v| v.role == VectorRole::Hatch)
        .collect();
    assert!(!hatches.is_empty());
    assert!(
        hatches
            .iter()
            .flat_map(|v| &v.preview_mm)
            .all(|p| (p[0] - 7.).hypot(p[1] + 2.) <= 4. + 1e-10)
    );

    let half = project_drawing(
        &inputs,
        DrawingProjection {
            section_only: true,
            section_u_mm: Some([7., 12.]),
            ..options
        },
    )
    .unwrap();
    assert!(
        half.iter()
            .any(|v| v.role == VectorRole::Section && matches!(v.curve, VectorCurve::Line { .. }))
    );
    assert!(
        half.iter()
            .flat_map(|v| &v.preview_mm)
            .all(|p| p[0] >= 7. - 1e-10)
    );
    let tangent = project_drawing(
        &inputs,
        DrawingProjection {
            section_only: true,
            section_offset_mm: Some(16.),
            ..options
        },
    )
    .unwrap();
    assert!(tangent.is_empty());
    let limited = project_drawing(
        &inputs,
        DrawingProjection {
            maximum_curves: 1,
            ..options
        },
    )
    .unwrap_err();
    assert_eq!(limited.kind, GeometryErrorKind::Limit);
}

fn stock(origin: [f64; 3], dimensions: [i64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        dimensions.map(|value| multicad_kernel::rust_kernel::Rational::new(value, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap()
}

#[test]
fn sphere_and_stock_hide_each_others_edges_without_losing_their_sources() {
    let sphere = sphere();
    let front = stock([7., -8., 18.], [6, 12, 1]);
    let rear = stock([0., -2., 7.], [14, 2, 1]);
    let options = DrawingProjection {
        hidden_lines: true,
        chord_tolerance_mm: 1e-4,
        ..Default::default()
    };
    let project = |sphere: &Solid, other: &Solid, options| {
        project_drawing(
            &[sphere, other].map(|solid| DrawingSolid { solid, hatch: None }),
            options,
        )
        .unwrap()
    };
    let front_view = project(&sphere, &front, options);
    let ellipse = |vector: &multicad_kernel::ProjectedVector| match vector.curve {
        VectorCurve::Ellipse {
            center: [7., -2.],
            major_mm: 5.,
            minor_mm: 5.,
            start_rad,
            end_rad,
            ..
        } if vector.source_index == 0 => Some([start_rad, end_rad]),
        _ => None,
    };
    let [visible, hidden] = [VectorRole::Visible, VectorRole::Hidden].map(|role| {
        front_view
            .iter()
            .filter(|edge| edge.role == role)
            .filter_map(ellipse)
            .map(|[start, end]| end - start)
            .sum::<f64>()
    });
    assert!((visible - TAU / 2.).abs() < 1e-7);
    assert!((hidden - TAU / 2.).abs() < 1e-7);
    let rear_view = project(&sphere, &rear, options);
    assert!(rear_view.iter().any(|v| v.source_index == 1
        && v.role == VectorRole::Hidden
        && matches!(v.curve, VectorCurve::Line { a, b }
            if [a,b].into_iter().all(|p| (p[1]+2.).abs() < 1e-9)
                && (a[0].min(b[0])-2.).abs() < 1e-6 && (a[0].max(b[0])-12.).abs() < 1e-6)));
    assert!(
        rear_view
            .iter()
            .any(|v| v.role == VectorRole::Visible && ellipse(v) == Some([0., TAU]))
    );
    let detail = project(
        &sphere,
        &rear,
        DrawingProjection {
            detail: Some(([2., -2.], 2.)),
            ..options
        },
    );
    assert!(detail.iter().any(|v| ellipse(v).is_some()));
    assert!(
        detail
            .iter()
            .flat_map(|v| &v.preview_mm)
            .all(|p| (p[0] - 2.).hypot(p[1] + 2.) < 2.000001)
    );

    #[cfg(feature = "occt")]
    {
        let directory = tempfile::tempdir().unwrap();
        let native = |solid: &Solid| {
            let path = directory.path().join("part.step");
            solid.write_step(&path).unwrap();
            Solid::read_step(&path, Default::default()).unwrap()
        };
        let native_sphere = native(&sphere);
        let visible_only = DrawingProjection {
            hidden_lines: false,
            ..options
        };
        let native_front = project(&native_sphere, &native(&front), visible_only);
        let arc = native_front
            .iter()
            .filter_map(ellipse)
            .map(|[start, end]| end - start)
            .sum::<f64>();
        assert!((arc - TAU / 2.).abs() < 1e-7, "{native_front:?}");
        let native_rear = project(&native_sphere, &native(&rear), visible_only);
        for output in [&native_rear, &rear_view] {
            let mut intervals: Vec<_> = output
                .iter()
                .filter_map(|v| match v.curve {
                    VectorCurve::Line { a, b }
                        if v.source_index == 1
                            && v.role == VectorRole::Visible
                            && [a, b].into_iter().all(|p| (p[1] + 2.).abs() < 1e-9) =>
                    {
                        Some([a[0].min(b[0]), a[0].max(b[0])])
                    }
                    _ => None,
                })
                .collect();
            intervals.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
            intervals.dedup_by(|a, b| (0..2).all(|i| (a[i] - b[i]).abs() < 1e-6));
            assert_eq!(intervals.len(), 2, "{intervals:?}");
            for (a, b) in intervals.into_iter().flatten().zip([0., 2., 12., 14.]) {
                assert!((a - b).abs() < 1e-6, "{a} != {b}");
            }
        }
    }
}

#[test]
fn sphere_and_planar_stock_publish_independent_section_loops_and_hatches() {
    let sphere = sphere();
    let stock = stock([15., -5., 10.], [3, 4, 6]);
    let hatch = DrawingHatch {
        angle_rad: 0.,
        spacing_mm: 0.5,
        line_width_mm: 0.18,
        pattern: HatchPattern::Lines,
    };
    let output = project_drawing(
        &[&stock, &sphere].map(|solid| DrawingSolid {
            solid,
            hatch: Some(hatch),
        }),
        DrawingProjection {
            section_only: true,
            section_offset_mm: Some(14.),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(output.iter().any(|v| v.source_index == 0
        && v.role == VectorRole::Section
        && matches!(v.curve, VectorCurve::Line { .. })));
    assert!(output.iter().any(|v| v.source_index == 1
        && v.role == VectorRole::Section
        && matches!(
            v.curve,
            VectorCurve::Ellipse {
                center: [7., -2.],
                major_mm: 4.,
                ..
            }
        )));
    for source in [0, 1] {
        assert!(
            output
                .iter()
                .any(|v| v.source_index == source && v.role == VectorRole::Hatch)
        );
    }
}
