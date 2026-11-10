use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    materials::HatchPattern,
};
use multicad_kernel::{
    BooleanOperation, DrawingHatch, DrawingProjection, DrawingSolid, GeometryPolicy, PlaneFrame,
    ProjectedVector, RigidTransform, Solid, project_drawing, rust_kernel::Rational,
};
use std::collections::BTreeSet;

fn box_at(rust: bool, size: [i64; 3], origin: [f64; 3]) -> Solid {
    let policy = GeometryPolicy::default();
    if rust {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    } else {
        Solid::box_solid(size.map(|n| n as f64), policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: origin,
                ..Default::default()
            })
            .unwrap()
    }
}

fn solids(rust: bool) -> [Solid; 3] {
    let stock = box_at(rust, [12, 10, 3], [0.; 3]);
    let through = box_at(rust, [4, 4, 5], [4., 3., -1.]);
    let plate = stock
        .boolean(
            &through,
            BooleanOperation::Subtract,
            GeometryPolicy::default(),
        )
        .unwrap()
        .unwrap();
    [
        plate,
        box_at(rust, [3, 4, 3], [14., 0., 0.]),
        box_at(rust, [3, 3, 2], [2., 2., -3.]),
    ]
}

fn project(solids: &[Solid; 3], pattern: HatchPattern) -> Vec<ProjectedVector> {
    let hatch = DrawingHatch {
        pattern,
        angle_rad: 0.,
        spacing_mm: 2.,
        line_width_mm: 0.18,
    };
    project_drawing(
        &[
            DrawingSolid {
                solid: &solids[0],
                hatch: Some(hatch),
            },
            DrawingSolid {
                solid: &solids[1],
                hatch: None,
            },
            DrawingSolid {
                solid: &solids[2],
                hatch: Some(hatch),
            },
        ],
        DrawingProjection {
            section_offset_mm: Some(1.),
            section_only: true,
            section_u_mm: Some([1., 16.]),
            section_v_mm: Some([1., 9.]),
            ..Default::default()
        },
    )
    .unwrap()
}

fn line_key(points: [[f64; 2]; 2]) -> [[i64; 2]; 2] {
    let mut key = points.map(|point| point.map(|v| (v * 1e6).round() as i64));
    key.sort_unstable();
    key
}

fn outline(min: [f64; 2], max: [f64; 2]) -> [[[i64; 2]; 2]; 4] {
    let vertices = [min, [max[0], min[1]], max, [min[0], max[1]]];
    std::array::from_fn(|i| line_key([vertices[i], vertices[(i + 1) % 4]]))
}

fn section(vectors: &[ProjectedVector], source: usize) -> BTreeSet<[[i64; 2]; 2]> {
    vectors
        .iter()
        .filter(|v| v.source_index == source && v.role == VectorRole::Section)
        .map(|v| {
            let VectorCurve::Line { a, b } = v.curve else {
                panic!("expected exact straight section boundary")
            };
            assert_eq!(v.preview_mm, [a, b]);
            line_key([a, b])
        })
        .collect()
}

fn stays_outside_hole(a: [f64; 2], b: [f64; 2]) -> bool {
    let hole = [[4. + 1e-6, 8. - 1e-6], [3. + 1e-6, 7. - 1e-6]];
    let mut start = 0_f64;
    let mut end = 1_f64;
    for (i, [lower, upper]) in hole.into_iter().enumerate() {
        let delta = b[i] - a[i];
        if delta.abs() < 1e-12 {
            if a[i] < lower || a[i] > upper {
                return true;
            }
            continue;
        }
        let [u, v] = [(lower - a[i]) / delta, (upper - a[i]) / delta];
        start = start.max(u.min(v));
        end = end.min(u.max(v));
    }
    start >= end
}

fn assert_hatch(vectors: &[ProjectedVector], pattern: HatchPattern) {
    let strokes: Vec<_> = vectors
        .iter()
        .filter(|v| v.role == VectorRole::Hatch)
        .collect();
    assert_eq!(strokes.is_empty(), pattern == HatchPattern::None);
    let mut area = 0.;
    for vector in strokes {
        assert_eq!(vector.source_index, 0);
        for &[x, y] in &vector.preview_mm {
            assert!((1. - 1e-6..=12. + 1e-6).contains(&x));
            assert!((1. - 1e-6..=9. + 1e-6).contains(&y));
        }
        match vector.curve {
            VectorCurve::Line { a, b } if pattern != HatchPattern::Solid => {
                assert_eq!(vector.preview_mm, [a, b]);
                assert!(stays_outside_hole(a, b), "{pattern:?} {a:?} {b:?}");
            }
            VectorCurve::HatchTriangle { points: [a, b, c] } if pattern == HatchPattern::Solid => {
                assert_eq!(vector.preview_mm, [a, b, c]);
                area += ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5;
            }
            _ => panic!("unexpected {pattern:?} curve"),
        }
    }
    if pattern == HatchPattern::Solid {
        assert!((area - (11. * 8. - 4. * 4.)).abs() < 1e-6, "{area}");
    }
}

#[test]
fn rust_section_hatches_preserve_clipped_material_and_source_attribution() {
    let rust = solids(true);
    let native = solids(false);
    let first = outline([1., 1.], [12., 9.])
        .into_iter()
        .chain(outline([4., 3.], [8., 7.]))
        .collect();
    let second = outline([14., 1.], [16., 4.]).into_iter().collect();
    for pattern in HatchPattern::ALL {
        let actual = project(&rust, pattern);
        let expected = project(&native, pattern);
        for vectors in [&actual, &expected] {
            assert_eq!(section(vectors, 0), first);
            assert_eq!(section(vectors, 1), second);
            assert!(vectors.iter().all(|v| v.source_index < 2));
            assert!(
                vectors
                    .iter()
                    .all(|v| matches!(v.role, VectorRole::Section | VectorRole::Hatch))
            );
            assert_hatch(vectors, pattern);
        }
    }
}
