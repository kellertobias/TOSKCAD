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
