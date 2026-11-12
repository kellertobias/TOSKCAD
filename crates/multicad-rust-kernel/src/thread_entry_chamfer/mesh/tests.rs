use super::*;
fn body() -> Chamfer {
    let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
        length_mm: 2.6,
        ..super::super::tests::tool(false, 1, true).spec()
    })
    .unwrap();
    Chamfer::new(source, source.spec().depth_mm * 1.2, None).unwrap()
}
fn options() -> Tessellation {
    Tessellation {
        deflection: 0.04,
        angle: 0.4,
        triangle_limit: 500_000,
    }
}
fn coarse(body: &Chamfer, options: Tessellation) -> Plan {
    let leaves = mesh_segments(body, options)
        .unwrap()
        .into_iter()
        .map(|segment| {
            let ends = segment.ends.map(Sample::from);
            let q = I::new(ends[0].phase.lo, ends[1].phase.hi);
            Leaf {
                segment,
                ends,
                dq: (ends[1].phase - ends[0].phase).positive(),
                thread: body.patch_bounds(segment, q, false).unwrap(),
                cone: segment
                    .active
                    .then(|| body.patch_bounds(segment, q, true).unwrap()),
                trim: body
                    .trim_error(segment, q, (ends[1].phase - ends[0].phase).positive())
                    .unwrap(),
                normals: [
                    geometry::normal_field(
                        body,
                        segment,
                        (ends[0].phase + ends[1].phase) / I::point(2.),
                        false,
                    )
                    .unwrap(),
                    geometry::normal_field(
                        body,
                        segment,
                        (ends[0].phase + ends[1].phase) / I::point(2.),
                        true,
                    )
                    .unwrap(),
                ],
            }
        })
        .collect();
    let cycle = body.tool.spec().pitch_mm * f64::from(body.tool.spec().starts)
        / body.tool.helical_rate().abs();
    Plan {
        leaves,
        angular_step: cycle / 48.,
        angular_count: 48,
    }
}
#[test]
fn an_uncertified_coarse_sampler_is_refined_before_manufacturing_publication() {
    let body = body();
    let original = body;
    let options = options();
    let mut work = Work {
        remaining: 16_000_000,
        interrupt: None,
    };
    assert!(matches!(
        builder::build(&body, &coarse(&body, options), options, &mut work),
        Err(builder::Failure::Refine(_))
    ));
    assert_eq!(
        build_adaptive(&body, coarse(&body, options), options, &mut work, 1),
        Err(Error::Unresolved)
    );
    let mesh = build_adaptive(&body, coarse(&body, options), options, &mut work, 8).unwrap();
    let proof = crate::manufacturing::certify(&mesh).unwrap();
    let volume = body.volume_mm3(QueryOptions::default()).unwrap();
    assert!((proof.signed_volume_mm3 - volume.lower).abs() < 0.01 * volume.lower);
    assert_eq!(body, original);
}
#[test]
fn phase_sampling_detects_loss_of_representable_progress_and_real_grid_limits() {
    let body = body();
    let mut segment = mesh_segments(&body, options()).unwrap()[0];
    let q = 0.01_f64;
    segment.ends = [
        Knot {
            phase: I::point(q),
            nominal: q,
            threshold: false,
            glued: false,
        },
        Knot {
            phase: I::point(q.next_up()),
            nominal: q.next_up(),
            threshold: false,
            glued: false,
        },
    ];
    let mut work = Work {
        remaining: 1000,
        interrupt: None,
    };
    let tight = Tessellation {
        deflection: 1e-200,
        ..options()
    };
    assert!(matches!(
        make_plan(&body, &[segment], tight, 1., &mut work),
        Err(Error::Unresolved)
    ));
    segment.ends[1] = segment.ends[0];
    assert!(matches!(
        make_plan(&body, &[segment], options(), 1., &mut work),
        Err(Error::Unresolved)
    ));
    let source = super::super::tests::tool(false, 8, false);
    let many = Chamfer::new(source, 0.3, None).unwrap();
    assert_eq!(
        tessellate(
            &many,
            Tessellation {
                deflection: 100.,
                angle: 1.,
                triangle_limit: 1
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert!(matches!(
        make_plan(
            &body,
            &[],
            Tessellation {
                deflection: f64::from_bits(1),
                ..options()
            },
            1.,
            &mut work
        ),
        Err(Error::Unresolved)
    ));
}
#[test]
fn shared_flat_root_contact_carries_its_real_coefficient_residual() {
    let source = super::super::tests::tool(false, 1, false);
    let contact = Chamfer::new(source, source.spec().depth_mm, None).unwrap();
    let segments = contact.phase_segments().unwrap();
    let error = segments
        .iter()
        .map(|&s| contact.glue_residual(s).unwrap()[0])
        .fold(0., f64::max);
    assert!(error > 0.);
    let mesh = tessellate(&contact, options()).unwrap();
    assert!(crate::manufacturing::certify(&mesh).is_ok());
    let spec = crate::helical_thread::Spec {
        major_radius_mm: 1e6,
        pitch_mm: 0.01,
        length_mm: 0.066,
        depth_mm: 0.005,
        crest_width_mm: 0.00125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts: 1,
        left_hand: false,
        root_radius_mm: None,
    };
    let source = crate::helical_thread::Thread::new(spec).unwrap();
    let body = Chamfer::new(source, spec.depth_mm, None).unwrap();
    assert_eq!(
        tessellate(
            &body,
            Tessellation {
                deflection: 1e-5,
                ..options()
            }
        ),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(body.source_thread(), &source);
    assert!(
        body.volume_mm3(QueryOptions {
            tolerance: 100.,
            maximum_cells: 10000
        })
        .unwrap()
        .lower
            > 0.
    );
}
#[test]
fn a_circle_turn_at_the_existing_face_boundary_does_not_create_a_spurious_sliver() {
    for rounded in [false, true] {
        let radius = 0.25;
        let depth = if rounded {
            0.875 / (2. * 45_f64.to_radians().tan())
                - radius * (1. / 45_f64.to_radians().sin() - 1.)
        } else {
            0.2
        };
        let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 2.6,
            depth_mm: depth,
            crest_width_mm: 0.125,
            lower_flank_deg: 45.,
            upper_flank_deg: 45.,
            starts: 1,
            left_hand: false,
            root_radius_mm: rounded.then_some(radius),
        })
        .unwrap();
        let body = Chamfer::new(source, 0.4, None).unwrap();
        assert!(tessellate(&body, options()).is_ok());
    }
}

#[test]
fn an_empty_surface_proposal_cannot_become_a_successful_manufacturing_solid() {
    let body = body();
    let mut work = Work {
        remaining: 1000,
        interrupt: None,
    };
    let plan = Plan {
        leaves: Vec::new(),
        angular_step: 0.1,
        angular_count: 63,
    };
    assert_eq!(
        build_adaptive(&body, plan, options(), &mut work, 8),
        Err(Error::Unresolved)
    );
}

#[test]
fn clockwise_terminal_mesh_and_cancellation_during_the_proof_keep_source_immutable() {
    let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
        length_mm: 2.6,
        ..super::super::tests::tool(false, 1, true).spec()
    })
    .unwrap();
    let body = Chamfer::new(
        source,
        0.3,
        Some(TerminalIntent {
            width_mm: 0.6,
            radial_depth_mm: 0.1,
        }),
    )
    .unwrap();
    let original = body;
    let options = Tessellation {
        deflection: 0.06,
        angle: 0.5,
        triangle_limit: 500_000,
    };
    let calls = std::cell::Cell::new(0);
    let mesh = tessellate_with_interrupt(
        &body,
        options,
        Some(&|| {
            calls.set(calls.get() + 1);
            false
        }),
    )
    .unwrap();
    assert!(crate::manufacturing::certify(&mesh).is_ok());
    let total = calls.get();
    assert!(total > 4000);
    for cutoff in [500, 1500, 4000, total - 2] {
        calls.set(0);
        assert_eq!(
            tessellate_with_interrupt(
                &body,
                options,
                Some(&|| {
                    calls.set(calls.get() + 1);
                    calls.get() > cutoff
                })
            ),
            Err(Error::Cancelled)
        );
        assert_eq!(body, original);
    }
}
#[test]
fn unresolved_subprecision_root_contact_cannot_be_promoted_to_a_trimmed_boundary() {
    let mut spec = super::super::tests::tool(false, 1, true).spec();
    spec.major_radius_mm = 1e7;
    let radius = 2_f64.powi(-40);
    spec.root_radius_mm = Some(radius);
    spec.depth_mm = 0.875 / (2. * 30_f64.to_radians().tan()) - radius;
    let source = crate::helical_thread::Thread::new(spec).unwrap();
    let root = source.root_circle().unwrap();
    let body = Chamfer::from_material(source, root.center_radius, 0.3, None).unwrap();
    let segments = body.phase_segments().unwrap();
    assert!(segments.iter().any(|s| s.ends.iter().any(|k| k.glued)));
    assert!(
        segments
            .iter()
            .any(|&s| body.glue_residual(s) == Err(Error::Unresolved))
    );
    assert_eq!(
        tessellate(&body, options()),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(body.source_thread(), &source);
}

#[test]
fn unrepresentable_extra_phase_rails_fail_instead_of_publishing_degenerate_facets() {
    let body = body();
    let options = options();
    let mut work = Work {
        remaining: 16_000_000,
        interrupt: None,
    };
    let mut plan = make_plan(
        &body,
        &mesh_segments(&body, options).unwrap(),
        options,
        1.,
        &mut work,
    )
    .unwrap();
    let original = plan.leaves.remove(0);
    let q = 0.001_f64;
    let point = |q| Sample {
        phase: I::point(q),
        nominal: q,
        threshold: false,
        glued: false,
    };
    let cuts = [
        original.ends[0],
        point(q),
        point(q.next_up()),
        original.ends[1],
    ];
    let mut replacement = Vec::new();
    for pair in cuts.windows(2) {
        let ends = [pair[0], pair[1]];
        let range = I::new(ends[0].phase.lo, ends[1].phase.hi);
        replacement.push(Leaf {
            segment: original.segment,
            ends,
            dq: (ends[1].phase - ends[0].phase).positive(),
            thread: body.patch_bounds(original.segment, range, false).unwrap(),
            cone: Some(body.patch_bounds(original.segment, range, true).unwrap()),
            trim: body
                .trim_error(
                    original.segment,
                    range,
                    (ends[1].phase - ends[0].phase).positive(),
                )
                .unwrap(),
            normals: [
                geometry::normal_field(
                    &body,
                    original.segment,
                    (ends[0].phase + ends[1].phase) / I::point(2.),
                    false,
                )
                .unwrap(),
                geometry::normal_field(
                    &body,
                    original.segment,
                    (ends[0].phase + ends[1].phase) / I::point(2.),
                    true,
                )
                .unwrap(),
            ],
        });
    }
    replacement.extend(plan.leaves);
    plan.leaves = replacement;
    assert_eq!(
        build_adaptive(&body, plan, options, &mut work, 8),
        Err(Error::Unresolved)
    );
}
