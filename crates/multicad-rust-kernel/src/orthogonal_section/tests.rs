use super::*;

const TOLERANCE: f64 = 1e-7;
const PANEL: Box3 = Box3 {
    min: [0.; 3],
    max: [20., 16., 10.],
};

fn stock(negative: &[Box3]) -> (OrthogonalSolid, Boundary) {
    let stock = orthogonal::boolean(&[PANEL], negative, Operation::Subtract, TOLERANCE)
        .unwrap()
        .unwrap();
    let boundary = orthogonal_boundary::build(stock.quads()).unwrap();
    (stock, boundary)
}

fn face(boundary: &Boundary, normal: Point3) -> usize {
    boundary
        .faces
        .iter()
        .position(|face| face.normal == normal)
        .unwrap()
}

fn options(axis: usize, depth_mm: f64) -> Options {
    Options {
        axis,
        depth_mm,
        windows_mm: [None; 2],
        tolerance_mm: TOLERANCE,
        segment_limit: 100,
    }
}

fn cavities(boundary: &Boundary) -> ([OrthogonalPocket; 2], OrthogonalSlot) {
    let top = face(boundary, [0., 0., 1.]);
    let bottom = face(boundary, [0., 0., -1.]);
    (
        [
            OrthogonalPocket {
                face: top,
                center_mm: [16., 7., 10.],
                radius_mm: 1.5,
                depth_mm: 2.,
            },
            OrthogonalPocket {
                face: bottom,
                center_mm: [10., 13., 0.],
                radius_mm: 1.,
                depth_mm: 2.,
            },
        ],
        OrthogonalSlot {
            face: top,
            centers_mm: [[4., 7., 10.], [9., 7., 10.]],
            radius_mm: 1.5,
            depth_mm: 3.,
        },
    )
}

fn signed_area(wire: &[Edge]) -> f64 {
    wire.iter()
        .map(|edge| match *edge {
            Edge::Line([[ax, ay], [bx, by]]) => ax * by - ay * bx,
            Edge::Arc {
                center: [x, y],
                radius,
                start_rad,
                sweep_rad,
            } => {
                let end = start_rad + sweep_rad;
                radius
                    * (radius * sweep_rad + x * (end.sin() - start_rad.sin())
                        - y * (end.cos() - start_rad.cos()))
            }
        })
        .sum::<f64>()
        / 2.
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-10, "{actual} != {expected}");
}

#[test]
fn axial_sections_preserve_exact_round_and_stadium_holes_until_the_blind_floor() {
    let (stock, boundary) = stock(&[]);
    let (pockets, slot) = cavities(&boundary);
    let slice = |depth| section(&stock, &boundary, &pockets, &[slot], options(2, depth)).unwrap();
    for depth in [9., 10.] {
        let cut = slice(depth);
        assert_eq!(cut.len(), 3);
        near(signed_area(&cut[0]), 320.);
        assert_eq!(
            cut[1],
            [Edge::Arc {
                center: [16., 7.],
                radius: 1.5,
                start_rad: 0.,
                sweep_rad: -TAU,
            }]
        );
        near(signed_area(&cut[1]), -2.25 * PI);
        assert_eq!(cut[2].len(), 4);
        assert_eq!(
            cut[2]
                .iter()
                .filter(|edge| matches!(edge, Edge::Arc {sweep_rad, ..} if *sweep_rad == -PI))
                .count(),
            2
        );
        near(signed_area(&cut[2]), -15. - 2.25 * PI);
    }
    assert_eq!(slice(8.).len(), 2);
    assert_eq!(slice(7.).len(), 1);
    assert_eq!(slice(2.).len(), 1);
    let lower = slice(0.);
    assert_eq!(lower.len(), 2);
    near(signed_area(&lower[1]), -PI);
    assert!(slice(11.).is_empty());
}

#[test]
fn transverse_sections_remove_exact_stock_notches_and_respect_window() {
    let (stock, boundary) = stock(&[]);
    let (pockets, slot) = cavities(&boundary);
    let slice = |options| section(&stock, &boundary, &pockets, &[slot], options).unwrap();
    let bore = slice(options(0, 16.9));
    assert_eq!(bore.len(), 1);
    assert_eq!(bore[0].len(), 8);
    near(signed_area(&bore[0]), 160. - 2.4 * 2.);
    let mortise = slice(options(0, 6.));
    assert_eq!(mortise[0].len(), 8);
    near(signed_area(&mortise[0]), 160. - 9.);
    let both = slice(options(1, 7.));
    assert_eq!(both[0].len(), 12);
    near(signed_area(&both[0]), 200. - 24. - 6.);
    let tangent = slice(options(0, 17.5));
    near(signed_area(&tangent[0]), 160.);

    let window = Options {
        windows_mm: [Some([5., 10.]), Some([6., 9.])],
        ..options(0, 6.)
    };
    let cropped = slice(window);
    near(signed_area(&cropped[0]), 5. * 3. - 3. * 2.);
    let inside_void = Options {
        windows_mm: [Some([6., 8.]), Some([8., 9.])],
        ..window
    };
    assert!(slice(inside_void).is_empty());
}

#[test]
fn lateral_window_only_keeps_complete_curved_holes() {
    let (stock, boundary) = stock(&[]);
    let (pockets, slot) = cavities(&boundary);
    let slice = |window| {
        section(
            &stock,
            &boundary,
            &pockets,
            &[slot],
            Options {
                windows_mm: [Some(window), Some([2., 12.])],
                ..options(2, 9.)
            },
        )
    };
    let cropped = slice([1., 13.]).unwrap();
    assert_eq!(cropped.len(), 2);
    near(signed_area(&cropped[0]), 120.);
    near(signed_area(&cropped[1]), -15. - 2.25 * PI);
    let outside = slice([0., 2.5]).unwrap();
    assert_eq!(outside.len(), 1);
    near(signed_area(&outside[0]), 25.);
    assert!(slice([21., 24.]).unwrap().is_empty());
    for window in [[6., 12.], [2.5, 13.]] {
        assert_eq!(slice(window), Err(SectionError::Unsupported));
    }
}

#[test]
fn nonrectangular_orthogonal_stock_preserves_its_hole_and_disconnected_regions() {
    let (stock, boundary) = stock(&[Box3 {
        min: [3., 4., -1.],
        max: [7., 11., 12.],
    }]);
    let cut = section(&stock, &boundary, &[], &[], options(2, 5.)).unwrap();
    assert_eq!(cut.len(), 2);
    near(signed_area(&cut[0]), 320.);
    near(signed_area(&cut[1]), -28.);
    let cut = section(&stock, &boundary, &[], &[], options(0, 5.)).unwrap();
    assert_eq!(cut.len(), 2);
    assert!(cut.iter().all(|wire| signed_area(wire) > 0.));
    near(cut.iter().map(|wire| signed_area(wire)).sum(), 90.);
}

#[test]
fn rotated_stadium_chords_cover_strip_endcaps_and_tangent_contact() {
    let radius = 2_f64.sqrt();
    let rotated = Stadium::new([[0., 0.], [4., 4.]], radius).unwrap();
    for axis in 0..2 {
        let [[lo, hi], [end_lo, end_hi]] =
            [2., 0.].map(|depth| stadium_interval(rotated, axis, depth).unwrap().unwrap());
        near(lo, 0.);
        near(hi, 4.);
        near(end_lo, -radius);
        near(end_hi, 2.);
        let [lo, hi] = stadium_interval(rotated, axis, -1.).unwrap().unwrap();
        near(lo, -1.);
        near(hi, 1.);
        assert_eq!(stadium_interval(rotated, axis, -radius), Ok(None));
    }
    let horizontal = Stadium::new([[1., 2.], [6., 2.]], 1.).unwrap();
    assert_eq!(stadium_interval(horizontal, 1, 1.), Ok(None));
    assert_eq!(stadium_interval(horizontal, 1, 3.), Ok(None));
    assert_eq!(stadium_interval(horizontal, 1, 2.), Ok(Some([0., 7.])));
    assert_eq!(
        stadium_interval(horizontal, 2, 0.),
        Err(SectionError::Invalid)
    );
    assert_eq!(
        stadium_interval(horizontal, 0, f64::NAN),
        Err(SectionError::Invalid)
    );
    let wide = Stadium::new([[1e16, 0.], [1e16, 4.]], 8.).unwrap();
    assert_eq!(
        stadium_interval(wide, 1, (-8_f64).next_up()),
        Err(SectionError::Unsupported)
    );
    near(signed_area(&stadium_edges(horizontal)), 10. + PI);
}

#[test]
fn rejects_invalid_geometry_and_enforces_work_and_output_limits() {
    let (stock, mut boundary) = stock(&[]);
    let (pockets, slot) = cavities(&boundary);
    let good = options(2, 9.);
    for options in [
        Options { axis: 3, ..good },
        Options {
            depth_mm: f64::NAN,
            ..good
        },
        Options {
            tolerance_mm: 0.,
            ..good
        },
        Options {
            windows_mm: [Some([2., 2.]), None],
            ..good
        },
        Options {
            windows_mm: [Some([f64::NEG_INFINITY, 2.]), None],
            ..good
        },
    ] {
        assert_eq!(
            section(&stock, &boundary, &pockets, &[slot], options),
            Err(SectionError::Invalid)
        );
    }
    let budget = Options {
        segment_limit: 4,
        ..good
    };
    assert_eq!(
        section(&stock, &boundary, &pockets, &[slot], budget),
        Err(SectionError::Limit)
    );
    let too_many = vec![slot; orthogonal_slots::slot_capacity(&stock) + 1];
    assert_eq!(
        section(&stock, &boundary, &[], &too_many, good),
        Err(SectionError::Limit)
    );
    for pocket in [
        OrthogonalPocket {
            face: boundary.faces.len(),
            ..pockets[0]
        },
        OrthogonalPocket {
            radius_mm: f64::NAN,
            ..pockets[0]
        },
        OrthogonalPocket {
            depth_mm: 0.,
            ..pockets[0]
        },
        OrthogonalPocket {
            center_mm: [f64::NAN, 7., 10.],
            ..pockets[0]
        },
        OrthogonalPocket {
            center_mm: [16., 7., 9.],
            ..pockets[0]
        },
    ] {
        assert_eq!(
            section(&stock, &boundary, &[pocket], &[], good),
            Err(SectionError::Invalid)
        );
    }
    let degenerate = OrthogonalSlot {
        centers_mm: [slot.centers_mm[0]; 2],
        ..slot
    };
    assert_eq!(
        section(&stock, &boundary, &[], &[degenerate], good),
        Err(SectionError::Invalid)
    );
    boundary.faces[slot.face].normal = [0.; 3];
    assert_eq!(
        section(&stock, &boundary, &pockets, &[slot], good),
        Err(SectionError::Invalid)
    );

    let exact = Options {
        windows_mm: [Some([20. - 0.5 * TOLERANCE, 22.]), None],
        ..good
    };
    assert_eq!(
        section(&stock, &boundary, &[], &[], exact),
        Err(SectionError::Unsupported)
    );
}

#[test]
fn downstream_proof_errors_preserve_typed_categories() {
    for (cause, expected) in [
        (BooleanError::Invalid, SectionError::Invalid),
        (BooleanError::Unresolved, SectionError::Unsupported),
        (BooleanError::Limit, SectionError::Limit),
    ] {
        assert_eq!(boolean_error(cause), expected);
    }
    for (cause, expected) in [
        (BoundaryError::Invalid, SectionError::Invalid),
        (BoundaryError::Unresolved, SectionError::Unsupported),
        (BoundaryError::Limit, SectionError::Limit),
    ] {
        assert_eq!(boundary_error(cause), expected);
    }
}
