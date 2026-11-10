use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
};
use multicad_kernel::{
    BooleanOperation, DrawingProjection, DrawingSolid, GeometryErrorKind, GeometryPolicy,
    PlaneFrame, ProjectedVector, RigidTransform, Solid, project_drawing, rust_kernel::Rational,
};

fn box_solid(rust: bool, size: [i64; 3], origin: [f64; 3]) -> Solid {
    let policy = GeometryPolicy::default();
    if rust {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|value| Rational::new(value, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    } else {
        Solid::box_solid(size.map(|value| value as f64), policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: origin,
                ..Default::default()
            })
            .unwrap()
    }
}

#[test]
fn a_depth_section_clips_through_a_real_hole_without_filling_it() {
    let build = |rust| {
        box_solid(rust, [32, 26, 10], [0.; 3])
            .boolean(
                &box_solid(rust, [7, 6, 14], [10., 8., -2.]),
                BooleanOperation::Subtract,
                GeometryPolicy::default(),
            )
            .unwrap()
            .unwrap()
    };
    let options = DrawingProjection {
        section_offset_mm: Some(7.),
        section_depth_mm: Some(5.),
        section_u_mm: Some([12., 29.]),
        section_v_mm: Some([2., 23.]),
        hidden_lines: true,
        ..Default::default()
    };
    let rust = project(&[build(true)], options);
    let native = project(&[build(false)], options);
    let cut = section_lines(&rust);
    assert_eq!(cut.len(), 8);
    assert_same_lines(&cut, &section_lines(&native));
    assert!(rust.iter().any(|v| v.role == VectorRole::Hidden));
}

fn project(solids: &[Solid], options: DrawingProjection) -> Vec<ProjectedVector> {
    let sources: Vec<_> = solids
        .iter()
        .map(|solid| DrawingSolid { solid, hatch: None })
        .collect();
    project_drawing(&sources, options).unwrap()
}

fn section_lines(vectors: &[ProjectedVector]) -> Vec<[[f64; 2]; 2]> {
    vectors
        .iter()
        .filter(|vector| vector.role == VectorRole::Section)
        .map(|vector| match vector.curve {
            VectorCurve::Line { a, b } => [a, b],
            ref other => panic!("Expected a straight section boundary, got {other:?}"),
        })
        .collect()
}

fn assert_same_lines(actual: &[[[f64; 2]; 2]], expected: &[[[f64; 2]; 2]]) {
    let matches = |a: [[f64; 2]; 2], b: [[f64; 2]; 2]| {
        a.into_iter()
            .flatten()
            .zip(b.into_iter().flatten())
            .all(|(a, b)| (a - b).abs() < 1e-6)
    };
    let mut unmatched = expected.to_vec();
    for &a in actual {
        let hit = unmatched
            .iter()
            .position(|&[c, d]| matches(a, [c, d]) || matches(a, [d, c]))
            .unwrap_or_else(|| panic!("Unmatched line {a:?} in {expected:?}"));
        unmatched.swap_remove(hit);
    }
    assert!(unmatched.is_empty(), "{unmatched:?}");
}

#[test]
fn depth_and_lateral_limits_clip_actual_volume_before_visibility() {
    let build = |rust| {
        [
            box_solid(rust, [10, 8, 6], [0.; 3]),
            box_solid(rust, [4, 4, 2], [1., 1., -10.]),
            box_solid(rust, [4, 4, 2], [2., 2., 10.]),
        ]
    };
    let options = DrawingProjection {
        section_offset_mm: Some(3.),
        section_depth_mm: Some(2.),
        section_u_mm: Some([1., 9.]),
        section_v_mm: Some([2., 6.]),
        ..Default::default()
    };
    let rust = project(&build(true), options);
    let native = project(&build(false), options);
    assert!(rust.iter().all(|v| v.source_index == 0));
    assert!(rust.iter().any(|v| v.role == VectorRole::Visible));
    assert_eq!(section_lines(&rust).len(), 4);
    assert_same_lines(&section_lines(&rust), &section_lines(&native));
    for point in rust.iter().flat_map(|vector| &vector.preview_mm) {
        assert!((1. - 1e-7..=9. + 1e-7).contains(&point[0]));
        assert!((2. - 1e-7..=6. + 1e-7).contains(&point[1]));
    }
    let far_from_all = project(
        &build(true),
        DrawingProjection {
            section_offset_mm: Some(50.),
            section_depth_mm: Some(1.),
            ..options
        },
    );
    assert!(far_from_all.is_empty());
}

#[test]
fn oblique_section_is_exact_in_its_own_plane_and_matches_native_geometry() {
    let a = 0.5_f64.sqrt();
    let b = (1. / 6_f64).sqrt();
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [a, -a, 0.],
        v: [b, b, -2. * b],
    };
    let options = DrawingProjection {
        frame,
        section_offset_mm: Some(1.2),
        section_only: true,
        ..Default::default()
    };
    let rust = project(&[box_solid(true, [6, 5, 4], [0.; 3])], options);
    let native = project(&[box_solid(false, [6, 5, 4], [0.; 3])], options);
    assert_eq!(rust.len(), 3);
    assert_same_lines(&section_lines(&rust), &section_lines(&native));
    for [[x, y], [u, v]] in section_lines(&rust) {
        assert!(((x - u).hypot(y - v) - 1.2 * 6_f64.sqrt()).abs() < 1e-7);
    }
}

#[test]
fn section_and_background_share_the_same_vector_budget() {
    let solid = box_solid(true, [10, 8, 6], [0.; 3]);
    let error = project_drawing(
        &[DrawingSolid {
            solid: &solid,
            hatch: None,
        }],
        DrawingProjection {
            section_offset_mm: Some(3.),
            maximum_curves: 7,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Limit);
}
