use super::*;
fn work() -> Work<'static> {
    Work {
        remaining: 100_000,
        interrupt: None,
    }
}
#[test]
fn polar_curve_proof_refines_positive_derivatives_without_accepting_axis_crossings() {
    let p = (0..=9)
        .map(|i| {
            let t = i as f64;
            let cubic = if i >= 3 {
                t * (t - 1.) * (t - 2.) / (9. * 8. * 7. * 3.)
            } else {
                0.
            };
            let quadratic = if i >= 2 {
                0.5 * t * (t - 1.) / (9. * 8.)
            } else {
                0.
            };
            [
                I::point(1.),
                I::point(cubic - quadratic + 0.26 * t / 9.),
                I::point(0.),
            ]
        })
        .collect::<Vec<_>>();
    assert!(positive_curve(&p, &mut work()).unwrap().is_none());
    let angle = positive_spline(&p, 1, &mut work()).unwrap();
    let expected = (1_f64 / 3. - 0.5 + 0.26).atan();
    assert!(angle.lo <= expected && angle.hi >= expected);
    let mut reversed = p.clone();
    reversed.reverse();
    assert!(positive_spline(&reversed, 1, &mut work()).is_err());
    let crossing = (0..=9)
        .map(|i| {
            [
                I::point(2. * i as f64 / 9. - 1.),
                I::point(0.),
                I::point(0.),
            ]
        })
        .collect::<Vec<_>>();
    assert_eq!(
        positive_spline(&crossing, 1, &mut work()).err(),
        Some(Error::Unresolved)
    );
    assert_eq!(
        positive_spline(
            &p,
            1,
            &mut Work {
                remaining: 1,
                interrupt: None
            }
        )
        .err(),
        Some(Error::Limit)
    );
    assert_eq!(
        positive_spline(
            &p,
            1,
            &mut Work {
                remaining: 100_000,
                interrupt: Some(&|| true)
            }
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert!(positive_spline(&p, 1, &mut work()).is_ok());
}
#[test]
fn independent_cap_partitions_require_unique_complete_corresponding_strips() {
    let tensors = BTreeSet::from([0, 1, 2]);
    let cones = BTreeSet::from([3]);
    let groups = vec![
        (0, vec![(0, true)]),
        (1, vec![(1, true)]),
        (2, vec![(2, true)]),
        (3, vec![(3, true), (4, true)]),
    ];
    check_groups(&groups, &tensors, &cones, &mut work()).unwrap();
    let mut duplicated = groups.clone();
    duplicated[2].0 = 0;
    assert_eq!(
        check_groups(&duplicated, &tensors, &cones, &mut work()),
        Err(Error::Invalid)
    );
    let mut fragmented = groups.clone();
    fragmented[0].1.push((5, true));
    assert_eq!(
        check_groups(&fragmented, &tensors, &cones, &mut work()),
        Err(Error::Invalid)
    );
    let mut missing = groups.clone();
    missing.pop();
    assert_eq!(
        check_groups(&missing, &tensors, &cones, &mut work()),
        Err(Error::Invalid)
    );
    assert_eq!(
        check_groups(
            &groups,
            &tensors,
            &cones,
            &mut Work {
                remaining: 1,
                interrupt: None
            }
        ),
        Err(Error::Limit)
    );
    assert_eq!(align_cycles(&[0, 1, 2, 3], &[2, 3, 0, 1]).unwrap(), 2);
    for wrong in [
        &[0, 2, 1, 3][..],
        &[0, 1, 2][..],
        &[4, 5, 6, 7][..],
        &[][..],
    ] {
        assert_eq!(align_cycles(&[0, 1, 2, 3], wrong), Err(Error::Invalid));
    }
    assert_eq!(align_cycles(&[], &[]), Err(Error::Invalid));
    let wire = vec![
        (0, true),
        (1, false),
        (2, true),
        (3, false),
        (4, true),
        (5, false),
    ];
    let expected = (0..6).collect();
    check_cone_partition(&wire, &expected, 3).unwrap();
    assert_eq!(
        check_cone_partition(&wire[..5], &expected, 3),
        Err(Error::Invalid)
    );
    for run in [2, 4] {
        assert_eq!(
            check_cone_partition(&wire, &expected, run),
            Err(Error::Unsupported)
        );
    }
}

#[test]
fn raw_cap_incidence_refuses_reused_rims_wrong_owners_and_nonunique_long_rails() {
    let source = crate::radial_tensor::tests::fixture();
    let input = source.input();
    let tensor = input
        .faces
        .iter()
        .position(|f| matches!(input.surfaces[f.surface], Surface::Tensor(_)))
        .unwrap();
    let cone = input
        .faces
        .iter()
        .position(|f| matches!(input.surfaces[f.surface], Surface::Cone { .. }))
        .unwrap();
    let plane = input
        .faces
        .iter()
        .position(|f| matches!(input.surfaces[f.surface], Surface::Plane(_)))
        .unwrap();
    let spline = input
        .edges
        .iter()
        .position(|e| matches!(input.curves[e.curve], Curve::Spline { .. }))
        .unwrap();
    let circle = input
        .edges
        .iter()
        .position(|e| matches!(input.curves[e.curve], Curve::Circle { .. }))
        .unwrap();
    let mut rims = BTreeMap::new();
    insert_rim(&mut rims, spline, tensor, true).unwrap();
    assert_eq!(
        insert_rim(&mut rims, spline, tensor, true),
        Err(Error::Invalid)
    );
    check_cap_owner(input, &rims, tensor, spline, true).unwrap();
    assert_eq!(
        check_cap_owner(input, &rims, tensor, spline, false),
        Err(Error::Invalid)
    );
    assert_eq!(
        check_cap_owner(input, &rims, tensor, circle, true),
        Err(Error::Invalid)
    );
    assert_eq!(
        check_cap_owner(input, &rims, cone, spline, true),
        Err(Error::Invalid)
    );
    check_cap_owner(input, &rims, cone, circle, true).unwrap();
    assert_eq!(
        check_cap_owner(input, &rims, plane, circle, true),
        Err(Error::Invalid)
    );
    let owners = &source.topology.edge_faces;
    connect(input, owners, input.edges[spline].vertices, owners[spline]).unwrap();
    assert_eq!(
        connect(input, owners, [usize::MAX, 0], owners[spline]),
        Err(Error::Unresolved)
    );
    assert_eq!(
        connect(input, owners, input.edges[circle].vertices, owners[circle]),
        Err(Error::Invalid)
    );
    let mut duplicated = input.clone();
    duplicated.edges.push(input.edges[spline]);
    let mut duplicate_owners = owners.clone();
    duplicate_owners.push(owners[spline]);
    assert_eq!(
        connect(
            &duplicated,
            &duplicate_owners,
            input.edges[spline].vertices,
            owners[spline]
        ),
        Err(Error::Unresolved)
    );
}

#[test]
fn finite_cap_circle_angles_require_matching_plane_sense_and_an_enclosed_polar_origin() {
    let body = crate::radial_tensor::tests::material();
    let chart = &body.ordered.caps.axial.chart;
    let axis = Axis {
        origin: [0.; 3],
        normal: [0., 0., 1.],
        reference: [1., 0., 0.],
    };
    let points = [[4., 0., 0.], [0., 4., 0.]];
    let angle = circle_angle(chart, 1e-9, axis, 4., points, true).unwrap();
    assert!(angle.lo < std::f64::consts::FRAC_PI_2 && angle.hi > std::f64::consts::FRAC_PI_2);
    assert_eq!(
        circle_angle(
            chart,
            1e-9,
            Axis {
                normal: [0., 1e-10, 1.],
                ..axis
            },
            4.,
            points,
            true
        )
        .err(),
        Some(Error::Unsupported)
    );
    for (error, a) in [
        (1., axis),
        (
            1e-9,
            Axis {
                origin: [1., 0., 0.],
                ..axis
            },
        ),
    ] {
        assert_eq!(
            circle_angle(chart, error, a, 4., points, true).err(),
            Some(Error::Unresolved)
        );
    }
    assert!(circle_angle(chart, 1e-9, axis, 4., points, false).is_err());
    let mut input = crate::radial_tensor::tests::fixture().input().clone();
    for curve in &mut input.curves {
        if let Curve::Circle { axis, .. } = curve {
            axis.normal[1] = 1e-10;
        }
    }
    let axial = BoundaryOwnership::new(input, 1e-7, None)
        .unwrap()
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap();
    assert_eq!(axial.certify_caps(None).err(), Some(Error::Unsupported));
}

fn connect<'a>(
    input: &'a BoundaryInput,
    owners: &[[usize; 2]],
    vertices: [usize; 2],
    sides: [usize; 2],
) -> Result<(usize, &'a [Point]), Error> {
    connecting_rail(input, owners, vertices, sides, &mut work())
}
#[test]
fn cap_rail_association_and_source_projection_share_cooperative_cancellation() {
    let source = crate::radial_tensor::tests::fixture();
    let axial = source
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap();
    let calls = std::cell::Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    let caps = axial.clone().certify_caps(Some(&count)).unwrap();
    let total = calls.get();
    // This one-ridge fixture finishes with the four charged cone-side
    // incidence checks. The preceding rail projection has bounded chunks.
    assert_eq!(caps.axial.cones.len(), 1);
    assert_eq!(caps.sides.len(), 4);
    let chunks = caps.rails.last().unwrap().1.len().div_ceil(128);
    for stop in [total - 4 - chunks, total - 4] {
        calls.set(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() == stop
        };
        assert_eq!(
            axial.clone().certify_caps(Some(&cancel)).err(),
            Some(Error::Cancelled)
        );
        assert_eq!(calls.get(), stop);
    }
    assert_eq!(axial.certify_caps(None).unwrap().input(), caps.input());
}
