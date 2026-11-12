use super::*;
use crate::helical_thread::Spec;
pub(super) fn tool(left: bool, starts: u32, rounded: bool) -> Thread {
    let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
    Thread::new(Spec {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 6.6,
        depth_mm: root.map_or(5. * 3_f64.sqrt() / 16., |r| 0.875 * 3_f64.sqrt() / 2. - r),
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts,
        left_hand: left,
        root_radius_mm: root,
    })
    .unwrap()
}
#[test]
fn clipping_preserves_literal_source_law_and_material_for_every_phase_regime() {
    for left in [false, true] {
        for starts in [1, 2, 8] {
            for rounded in [false, true] {
                let source = tool(left, starts, rounded);
                for factor in [0.1, 0.9, 1., 1.2] {
                    for terminal in [
                        None,
                        Some(TerminalIntent {
                            width_mm: 0.6,
                            radial_depth_mm: 0.1,
                        }),
                    ] {
                        let c = source.spec().depth_mm * factor;
                        let body = Chamfer::new(source, c, terminal).unwrap();
                        assert_eq!(body.source_thread(), &source);
                        assert_eq!(
                            body.intent(),
                            Some(Intent {
                                radial_depth_mm: c,
                                terminal
                            })
                        );
                        assert_eq!(body.entry_radius_mm(), source.spec().major_radius_mm - c);
                        assert_eq!(body.entry_length_mm(), c);
                        assert_eq!(body.cone_radius(0.), body.entry_radius_mm());
                        assert_eq!(body.cone_radius(c), source.spec().major_radius_mm);
                        for i in 0..64 {
                            let phase = source.spec().pitch_mm * i as f64 / 64.;
                            let r = source.radius_at_phase(phase);
                            let seam = body.seam_station(phase);
                            assert!(seam >= 0. && seam <= c);
                            if r > body.entry_radius_mm() {
                                assert!((body.cone_radius(seam) - r).abs() < 1e-14);
                            } else {
                                assert_eq!(seam, 0.);
                            }
                            for z in [0., 0.25 * c, 0.75 * c, c, body.material_length_mm()] {
                                let theta = (z - phase) / source.helical_rate();
                                let original = source.radius(theta, z);
                                let fraction = z.min(c) / c;
                                let cone = (1. - fraction) * body.entry_radius_mm()
                                    + fraction * source.spec().major_radius_mm;
                                assert!((body.radius(theta, z) - original.min(cone)).abs() < 1e-14);
                            }
                        }
                        if let Some(tail) = body.terminal_relief() {
                            assert_eq!(
                                body.radius(
                                    0.,
                                    body.material_length_mm().midpoint(body.full_length_mm())
                                ),
                                tail.neck_radius_mm()
                            );
                        }
                        let imported = Chamfer::from_material(
                            source,
                            body.entry_radius_mm(),
                            body.entry_length_mm(),
                            body.terminal,
                        )
                        .unwrap();
                        assert_eq!(imported.intent(), None);
                        assert_eq!(imported.face_count(), body.face_count());
                        let roles: (Vec<_>, Vec<_>) = (0..body.face_count())
                            .map(|i| (i, body.face(i).unwrap()))
                            .unzip();
                        assert_eq!(roles.0.len(), roles.1.len());
                        assert_eq!(roles.1[0], Face::EntryCap);
                        assert_eq!(roles.1[1], Face::EndCap);
                        for i in 2..2 + 4 * starts {
                            assert_eq!(
                                body.face(i),
                                Some(Face::ThreadBand {
                                    start: (i - 2) / 4,
                                    band: (i - 2) % 4
                                })
                            );
                        }
                        assert!(body.face(body.face_count()).is_none());
                    }
                }
            }
        }
    }
}
#[test]
fn invalid_clipping_never_mutates_source_or_fabricates_material_intent() {
    let source = tool(false, 1, true);
    let before = source;
    for c in [
        f64::NAN,
        f64::INFINITY,
        -1.,
        0.,
        0.001_f64.next_down(),
        3.,
        6.6,
    ] {
        assert_eq!(Chamfer::new(source, c, None), Err(Error::Invalid));
    }
    assert_eq!(
        Chamfer::new(
            source,
            0.2,
            Some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 3.
            })
        ),
        Err(Error::Invalid)
    );
    for (r, c) in [
        (0., 0.1),
        (3., 0.1),
        (f64::NAN, 0.1),
        (2.9, 0.),
        (2.9, 6.6),
        (2.9, f64::INFINITY),
    ] {
        assert_eq!(
            Chamfer::from_material(source, r, c, None),
            Err(Error::Invalid)
        );
    }
    let foreign = Relief::new(tool(true, 1, true), 0.6, 0.1).unwrap();
    assert_eq!(
        Chamfer::from_material(source, 2.9, 0.1, Some(foreign)),
        Err(Error::Invalid)
    );
    let body = Chamfer::new(source, 0.2, None).unwrap();
    for (theta, z) in [(f64::NAN, 1.), (0., f64::NAN), (0., -1.), (0., 7.)] {
        assert!(body.radius(theta, z).is_nan());
    }
    assert_eq!(source, before);
    assert_eq!(
        thread_error(crate::helical_thread::ThreadError::Invalid),
        Error::Invalid
    );
    assert_eq!(
        thread_error(crate::helical_thread::ThreadError::Limit),
        Error::Limit
    );
}

#[test]
fn bounded_positive_column_volume_matches_independent_flat_profile_integrals_and_symmetry() {
    let options = QueryOptions {
        tolerance: 1e-7,
        ..Default::default()
    };
    for rounded in [false, true] {
        for factor in [0.1, 0.9, 1., 1.2] {
            for terminal in [
                None,
                Some(TerminalIntent {
                    width_mm: 0.6,
                    radial_depth_mm: 0.1,
                }),
            ] {
                let body = Chamfer::new(
                    tool(false, 1, rounded),
                    tool(false, 1, rounded).spec().depth_mm * factor,
                    terminal,
                )
                .unwrap();
                let proof = body.volume_mm3(options).unwrap();
                assert!(proof.lower > 0.);
                assert!(proof.upper - proof.lower <= options.tolerance);
                for (left, starts) in [(true, 1), (false, 8), (true, 8)] {
                    let other = Chamfer::new(
                        tool(left, starts, rounded),
                        body.entry_length_mm(),
                        terminal,
                    )
                    .unwrap();
                    assert_eq!(other.volume_mm3(options).unwrap(), proof);
                }
                if !rounded {
                    let source = body.source_thread();
                    let a = body.entry_radius_mm();
                    let delta = source.spec().major_radius_mm - a;
                    let k = delta / body.entry_length_mm();
                    let mut loss = 0.;
                    for (band, span) in source.profile_breaks().windows(2).enumerate() {
                        let [x, y] = [
                            source.radius_at_phase(span[0]) - a,
                            source.radius_at_phase(span[1]) - a,
                        ];
                        let x = x.max(0.);
                        let y = y.max(0.);
                        let active = if x == 0. && y == 0. {
                            0.
                        } else if (x == 0.) != (y == 0.) {
                            (x + y) / source.slope_at_phase(span[0], band).abs()
                        } else {
                            span[1] - span[0]
                        };
                        let i2 = active * (x * x + x * y + y * y) / 3.;
                        let i3 = active * (x + y) * (x * x + y * y) / 4.;
                        loss += (2. * i3 + 3. * a * i2) / (3. * k);
                    }
                    let base = body
                        .terminal_relief()
                        .map_or_else(|| source.volume_mm3(), Relief::volume_mm3);
                    let expected = base - std::f64::consts::PI / source.spec().pitch_mm * loss;
                    assert!(
                        proof.lower <= expected + 1e-12 && proof.upper >= expected - 1e-12,
                        "{proof:?} expected{expected}"
                    );
                }
            }
        }
    }
}
#[test]
fn controlled_volume_refuses_bad_budgets_cancels_and_recovers_without_source_changes() {
    let body = Chamfer::new(tool(false, 1, true), 0.4, None).unwrap();
    let before = body;
    for tolerance in [0., -1., f64::NAN, f64::INFINITY] {
        assert_eq!(
            body.volume_mm3(QueryOptions {
                tolerance,
                ..Default::default()
            }),
            Err(Error::Invalid)
        );
    }
    for maximum_cells in [0, 1, 1_000_001] {
        assert_eq!(
            body.volume_mm3(QueryOptions {
                maximum_cells,
                ..Default::default()
            }),
            Err(Error::Limit)
        );
    }
    let count = std::cell::Cell::new(0);
    let cancelled = || {
        count.set(count.get() + 1);
        count.get() >= 4
    };
    assert_eq!(
        body.volume_with_interrupt(QueryOptions::default(), Some(&cancelled)),
        Err(Error::Cancelled)
    );
    assert_eq!(body, before);
    assert!(body.volume_mm3(QueryOptions::default()).is_ok());
}

#[test]
fn controlled_support_encloses_true_caps_neck_and_oblique_material_without_a_fake_entry_disk() {
    let options = QueryOptions {
        tolerance: 1e-6,
        ..Default::default()
    };
    for left in [false, true] {
        let body = Chamfer::new(
            tool(left, 1, true),
            0.4,
            Some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap();
        for direction in [
            [1., 0., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
            [0.3, 0.4, 0.5],
            [0.3, -0.4, -0.5],
        ] {
            let result = body.support(direction, options).unwrap();
            assert!(result.upper - result.lower <= options.tolerance);
            let mut sample = f64::NEG_INFINITY;
            for iz in 0..=64 {
                let z = body.full_length_mm() * iz as f64 / 64.;
                for i in 0..1024 {
                    let theta = std::f64::consts::TAU * i as f64 / 1024.;
                    let r = body.radius(theta, z);
                    let value = r
                        * (direction[0] * theta.cos() + direction[1] * theta.sin()).max(0.)
                        + direction[2] * z;
                    sample = sample.max(value);
                }
            }
            assert!(
                sample <= result.upper + 1e-12,
                "{direction:?} {result:?} sample{sample}"
            );
            assert!(result.lower - sample < 0.1);
        }
    }
    let source = tool(false, 1, true);
    let shallow = Chamfer::new(source, 0.1 * source.spec().depth_mm, None).unwrap();
    let direction = [0., 1., -100.];
    let proof = shallow.support(direction, options).unwrap();
    let mut sample = 0_f64;
    for i in 0..8192 {
        let theta = std::f64::consts::TAU * i as f64 / 8192.;
        sample = sample.max(shallow.radius(theta, 0.) * theta.sin());
    }
    // Along each fixed phase, the downward objective is strictly decreasing:
    // 100 > cone slope + R/|rate|. The cap's angular Lipschitz bound gives
    // an independent enclosure from the dense analytic cap samples.
    let lipschitz = source.maximum_profile_slope() * source.helical_rate().abs()
        + source.spec().major_radius_mm;
    let upper = sample + lipschitz * std::f64::consts::TAU / (2. * 8192.);
    assert!(
        proof.upper >= sample && proof.lower <= upper,
        "{proof:?} [{sample},{upper}]"
    );
    assert!(
        proof.upper < shallow.entry_radius_mm() - 0.01,
        "a clipped valley is not a full entry disk: {proof:?}"
    );
}
#[test]
fn support_budgets_and_cancellation_are_typed_and_leave_material_unchanged() {
    let body = Chamfer::new(tool(false, 1, true), 0.4, None).unwrap();
    let before = body;
    assert_eq!(
        body.support([0.; 3], QueryOptions::default()).unwrap(),
        Enclosure {
            lower: 0.,
            upper: 0.
        }
    );
    for direction in [[f64::NAN, 0., 0.], [0., f64::INFINITY, 0.]] {
        assert_eq!(
            body.support(direction, QueryOptions::default()),
            Err(Error::Invalid)
        );
    }
    assert_eq!(
        body.support(
            [0.3, 0.4, -0.5],
            QueryOptions {
                maximum_cells: 1,
                ..Default::default()
            }
        ),
        Err(Error::Limit)
    );
    let checks = std::cell::Cell::new(0);
    let cancel = || {
        checks.set(checks.get() + 1);
        checks.get() >= 10
    };
    assert_eq!(
        body.support_with_interrupt([0.3, 0.4, -0.5], QueryOptions::default(), Some(&cancel)),
        Err(Error::Cancelled)
    );
    assert_eq!(body, before);
    assert!(
        body.support([0.3, 0.4, -0.5], QueryOptions::default())
            .is_ok()
    );
}

#[test]
fn scale_aware_support_and_certified_full_cone_handle_nonunit_objectives_and_precision_limits() {
    let body = Chamfer::new(
        tool(false, 1, true),
        1.2 * tool(false, 1, true).spec().depth_mm,
        None,
    )
    .unwrap();
    for d in [
        [0., 0., 2.],
        [3., 4., 0.],
        [0., f64::from_bits(1), 0.],
        [0.3, -0.4, -0.5],
    ] {
        let proof = body.support(d, QueryOptions::default()).unwrap();
        assert!(proof.lower.is_finite() && proof.upper.is_finite());
    }
    assert_eq!(
        body.support(
            [3., 4., 0.],
            QueryOptions {
                tolerance: 1e-30,
                ..Default::default()
            }
        ),
        Err(Error::Unresolved)
    );
    assert_eq!(
        body.support(
            [f64::MAX, 1., 1.],
            QueryOptions {
                tolerance: f64::from_bits(1),
                ..Default::default()
            }
        ),
        Err(Error::Unresolved)
    );
    let thin = Chamfer::from_material(*body.source_thread(), 2.9, f64::from_bits(1), None).unwrap();
    assert_eq!(
        thin.volume_mm3(QueryOptions::default()),
        Err(Error::Unresolved)
    );
    assert_eq!(
        thin.support([0.3, 0.4, -0.5], QueryOptions::default()),
        Err(Error::Unresolved)
    );
}

#[test]
fn unproved_central_crest_clipping_refuses_and_large_phase_reduction_is_preserved() {
    let source = Thread::new(Spec {
        major_radius_mm: 1e7,
        pitch_mm: 0.0032990690049907053,
        length_mm: 0.03,
        depth_mm: 0.0007390051778989907,
        crest_width_mm: 0.0007390799746356486,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts: 1,
        left_hand: false,
        root_radius_mm: Some(0.0014780103557979804),
    })
    .unwrap();
    assert!(source.radius_at_phase(source.profile_breaks()[2]) > source.spec().major_radius_mm);
    assert_eq!(Chamfer::new(source, 0.001, None), Err(Error::Unresolved));
    assert_eq!(
        Chamfer::from_material(source, source.spec().major_radius_mm - 0.001, 0.001, None),
        Err(Error::Unresolved)
    );
    let mut spec = tool(false, 1, true).spec();
    spec.major_radius_mm *= 10.;
    spec.pitch_mm *= 10.;
    spec.length_mm *= 10.;
    spec.depth_mm *= 10.;
    spec.crest_width_mm *= 10.;
    spec.root_radius_mm = spec.root_radius_mm.map(|r| 10. * r);
    let source = Thread::new(spec).unwrap();
    let body = Chamfer::new(source, 0.4, None).unwrap();
    assert!(source.radius(f64::MAX, 0.).is_finite());
    assert_eq!(
        body.radius(f64::MAX, 0.),
        source.radius(f64::MAX, 0.).min(body.entry_radius_mm())
    );
}

#[test]
fn shared_ring_mesh_is_closed_and_tracks_analytic_material() {
    use crate::display::Tessellation;
    for (left, starts, rounded, factor, terminal) in [
        (false, 1, false, 0.1, false),
        (true, 2, true, 0.9, true),
        (false, 1, true, 1.2, false),
    ] {
        let source = tool(left, starts, rounded);
        let body = Chamfer::new(
            source,
            source.spec().depth_mm * factor,
            terminal.then_some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap();
        let options = Tessellation {
            deflection: 0.03,
            angle: 0.3,
            triangle_limit: 5_000_000,
        };
        let now = std::time::Instant::now();
        eprintln!("mesh case {left} {starts} {rounded} {factor} {terminal}");
        let mesh = tessellate(&body, options).unwrap();
        eprintln!(
            "mesh {} triangles in {:?}",
            mesh.face_indices.len(),
            now.elapsed()
        );
        let proof = crate::manufacturing::certify(&mesh).unwrap();
        assert!(proof.signed_volume_mm3 > 0.);
        assert!(mesh.face_indices.iter().all(|&f| f < body.face_count()));
    }
}

#[test]
fn manufacturing_mesh_refuses_invalid_precision_and_resource_requests_atomically() {
    use crate::display::{MeshError, Tessellation};
    let source = tool(false, 1, true);
    let body = Chamfer::new(source, 0.3, None).unwrap();
    let original = body;
    let options = Tessellation {
        deflection: 0.04,
        angle: 0.4,
        triangle_limit: 500_000,
    };
    for bad in [
        Tessellation {
            deflection: 0.,
            ..options
        },
        Tessellation {
            angle: 0.001,
            ..options
        },
        Tessellation {
            triangle_limit: 0,
            ..options
        },
        Tessellation {
            triangle_limit: 5_000_001,
            ..options
        },
    ] {
        assert_eq!(tessellate(&body, bad), Err(MeshError::InvalidOptions));
    }
    assert_eq!(
        tessellate_with_interrupt(&body, options, Some(&|| true)),
        Err(Error::Cancelled)
    );
    let calls = std::cell::Cell::new(0);
    assert_eq!(
        tessellate_with_interrupt(
            &body,
            options,
            Some(&|| {
                calls.set(calls.get() + 1);
                calls.get() > 8
            })
        ),
        Err(Error::Cancelled)
    );
    assert!(calls.get() > 8);
    assert_eq!(
        tessellate(
            &body,
            Tessellation {
                triangle_limit: 1,
                ..options
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        tessellate(
            &body,
            Tessellation {
                deflection: 1e-30,
                ..options
            }
        ),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(body, original);
    assert_eq!(body.source_thread(), &source);
    let mesh = tessellate(&body, options).unwrap();
    assert!(crate::manufacturing::certify(&mesh).is_ok());
    assert_eq!(body, original);
}
