use super::*;

fn panel(source: usize, min: [f64; 3], max: [f64; 3]) -> Body {
    Body {
        source,
        stock: vec![Box3 { min, max }],
        cuts: vec![],
        positive: vec![],
    }
}

fn profile(span: Span, centers: [Point; 2], radius: f64) -> AxialProfile {
    AxialProfile {
        span,
        profile: Profile::Stadium(Stadium::new(centers, radius).unwrap()),
    }
}

fn disk(span: Span, center: Point, radius: f64) -> AxialProfile {
    AxialProfile {
        span,
        profile: Profile::Circle { center, radius },
    }
}

fn options() -> Options {
    Options {
        depth_mm: 10.,
        far_mm: Some(0.),
        windows_mm: [None; 2],
        tolerance_mm: 1e-7,
        segment_limit: 1000,
    }
}

fn assert_edge(output: &[Segment], source: usize, ends: [Point; 2]) {
    assert!(
        output
            .iter()
            .any(|segment| segment.source == source && segment.ends == ends),
        "{source} {ends:?} missing: {output:?}"
    );
}

#[test]
fn crossing_rear_mortises_hide_one_another_in_the_retained_scene() {
    let mut left = panel(1, [0., 0., -5.], [18., 50., 20.]);
    let mut shelf = panel(2, [18., 0., -5.], [60., 50., 20.]);
    left.cuts
        .push(profile([3., 18.], [[16., 10.], [36., 10.]], 4.));
    shelf
        .cuts
        .push(profile([18., 46.], [[19., 10.], [33., 10.]], 4.));
    let insert = Body {
        source: 3,
        stock: vec![],
        cuts: vec![],
        positive: vec![profile([4.5, 44.5], [[19., 10.], [33., 10.]], 4.)],
    };
    let visible = project(std::slice::from_ref(&left), options()).unwrap();
    assert_edge(&visible, 1, [[18., 0.], [18., 50.]]);
    let visible = project(&[left, shelf, insert], options()).unwrap();
    assert_eq!(visible.len(), 20, "{visible:?}");
    assert_edge(&visible, 1, [[18., 0.], [18., 12.]]);
    assert_edge(&visible, 1, [[18., 40.], [18., 50.]]);
    assert_edge(&visible, 1, [[3., 12.], [3., 40.]]);
    assert_edge(&visible, 2, [[18., 0.], [18., 15.]]);
    assert_edge(&visible, 2, [[18., 37.], [18., 50.]]);
    assert_edge(&visible, 2, [[46., 15.], [46., 37.]]);
    assert_edge(&visible, 3, [[4.5, 15.], [4.5, 37.]]);
    assert_edge(&visible, 3, [[44.5, 15.], [44.5, 37.]]);
}

#[test]
fn cropping_stock_respects_holes_outside_the_retained_slab() {
    let mut solid = panel(5, [0., 0., 0.], [20., 15., 18.]);
    solid.stock.push(Box3 {
        min: [20., 0., 0.],
        max: [24., 15., 18.],
    });
    solid.cuts.push(disk([0., 3.], [2., 16.], 1.));
    solid.cuts.push(disk([0., 3.], [18., 6.], 1.));
    solid.cuts.push(disk([0., 3.], [4., 3.], 1.));
    let visible = project(
        &[solid],
        Options {
            far_mm: Some(5.),
            windows_mm: [Some([1., 23.]), Some([2., 12.])],
            ..options()
        },
    )
    .unwrap();
    assert_eq!(visible.len(), 4);
    assert_edge(&visible, 5, [[1., 2.], [23., 2.]]);
    assert_edge(&visible, 5, [[1., 12.], [23., 12.]]);
    assert_edge(&visible, 5, [[1., 2.], [1., 12.]]);
    assert_edge(&visible, 5, [[23., 2.], [23., 12.]]);
}

#[test]
fn positive_disk_and_negative_bore_keep_the_annular_cut_edges() {
    let solid = Body {
        source: 5,
        stock: vec![],
        positive: vec![disk([0., 10.], [4., 8.], 4.)],
        cuts: vec![disk([0., 10.], [4., 8.], 2.)],
    };
    let visible = project(
        &[solid],
        Options {
            depth_mm: 8.,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(visible.len(), 6);
    for y in [0., 2., 6., 8.] {
        assert_edge(&visible, 5, [[0., y], [10., y]]);
    }
    assert_edge(&visible, 5, [[0., 0.], [0., 8.]]);
    assert_edge(&visible, 5, [[10., 0.], [10., 8.]]);
}

#[test]
fn clipped_arc_slices_without_material_do_not_extend_the_silhouette() {
    let solid = Body {
        source: 1,
        stock: vec![],
        positive: vec![disk([0., 10.], [4., 8.], 4.)],
        cuts: vec![],
    };
    let visible = project(
        &[solid],
        Options {
            depth_mm: 11.,
            far_mm: Some(10.),
            ..options()
        },
    )
    .unwrap();
    let half = 12_f64.sqrt();
    assert_edge(&visible, 1, [[0., 4. - half], [0., 4. + half]]);
    assert_eq!(visible.len(), 6);
}

#[test]
fn intersecting_line_supports_split_the_upper_envelope() {
    let slope = Height::Line {
        origin: [0., 0.],
        tangent: [1., 1.],
    };
    let mut events = Vec::new();
    slope.intersections(Height::flat(2.), [0., 5.], &mut events);
    assert_eq!(events, vec![2.]);
}

#[test]
fn rotated_stadium_uses_exact_central_strip_and_smooth_tangency() {
    let solid = Body {
        source: 5,
        stock: vec![],
        positive: vec![profile([1., 8.], [[0., 0.], [4., 4.]], 2_f64.sqrt())],
        cuts: vec![],
    };
    let visible = project(
        &[solid],
        Options {
            depth_mm: 8.,
            far_mm: None,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(visible.len(), 4, "{visible:?}");
    let y = 2_f64.sqrt();
    assert_edge(&visible, 5, [[1., -y], [8., -y]]);
    assert_edge(&visible, 5, [[1., 4. + y], [8., 4. + y]]);
}

#[test]
fn invalid_empty_and_budgeted_requests_are_distinct() {
    let good = options();
    let small = [panel(1, [0.; 3], [2.; 3])];
    for work in [1, 8] {
        assert_eq!(
            project_bounded(&small, good, work),
            Err(ProjectionError::Limit)
        );
    }
    assert!(project(&[], good).unwrap().is_empty());
    for options in [
        Options {
            tolerance_mm: 0.,
            ..good
        },
        Options {
            depth_mm: f64::INFINITY,
            ..good
        },
        Options {
            far_mm: Some(11.),
            ..good
        },
        Options {
            windows_mm: [Some([2., 1.]), None],
            ..good
        },
    ] {
        assert_eq!(project(&[], options), Err(ProjectionError::Invalid));
    }
    assert_eq!(
        project(&[panel(1, [1.; 3], [1.; 3])], good),
        Err(ProjectionError::Invalid)
    );
    assert_eq!(
        project(
            &[panel(1, [0.; 3], [2.; 3])],
            Options {
                segment_limit: 3,
                ..good
            }
        ),
        Err(ProjectionError::Limit)
    );
    let mut solid = panel(1, [0.; 3], [2.; 3]);
    solid.cuts.push(disk([2., 1.], [0., 0.], 1.));
    assert_eq!(
        project(std::slice::from_ref(&solid), good),
        Err(ProjectionError::Invalid)
    );
    solid.cuts[0] = disk([1., 2.], [0., 0.], 0.);
    assert_eq!(project(&[solid], good), Err(ProjectionError::Invalid));
    let mut crowded = panel(0, [0.; 3], [2.; 3]);
    crowded
        .cuts
        .extend((0..1500).map(|_| disk([0., 1.], [1., 1.], 0.5)));
    assert_eq!(project(&[crowded], good), Err(ProjectionError::Limit));
}
