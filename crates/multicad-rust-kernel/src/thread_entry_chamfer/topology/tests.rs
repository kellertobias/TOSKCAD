use super::*;
#[test]
fn direct_source_topology_has_all_semantic_faces_closed_wires_and_real_neck_holes() {
    for (left, starts, rounded, c, neck) in [
        (false, 1, false, 0.3, false),
        (true, 2, true, 0.3, true),
        (true, 2, true, 0.5, false),
        (false, 3, true, 0.8, true),
    ] {
        let body = Chamfer::new(
            super::super::tests::tool(left, starts, rounded),
            c,
            neck.then_some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap();
        let topology = body.boundary_topology(QueryOptions::default()).unwrap();
        assert_eq!(topology.source(), &body);
        assert_eq!(topology.faces().len(), body.face_count() as usize);
        assert!(topology.maximum_vertex_error_mm() <= 1e-7);
        for (i, face) in topology.faces().iter().enumerate() {
            assert_eq!(Some(face.role()), body.face(i as u32));
            assert!(!face.loops().is_empty());
        }
        assert_eq!(topology.faces()[1].loops().len(), if neck { 2 } else { 1 });
        for (id, edge) in topology.edges().iter().enumerate() {
            for (end, vertex) in edge.vertices().into_iter().enumerate() {
                let value = topology.curve_sample(id, end as f64).unwrap();
                let actual = topology.vertices()[vertex];
                let d = value
                    .nominal_mm
                    .into_iter()
                    .zip(actual.nominal_mm)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>();
                assert!(d <= value.maximum_error_mm + actual.maximum_error_mm + 1e-13);
            }
            let bounds = topology.curve_bounds(id, [0., 1.]).unwrap();
            for t in [0., 0.2, 0.5, 0.8, 1.] {
                let p = topology.curve_sample(id, t).unwrap();
                for (i, x) in p.nominal_mm.into_iter().enumerate() {
                    assert!(
                        bounds[i].lower <= x + p.maximum_error_mm
                            && x - p.maximum_error_mm <= bounds[i].upper
                    );
                }
            }
        }
    }
}
#[test]
fn nonbinary_pitch_all_starts_keep_closed_semantic_source_incidence() {
    for left in [false, true] {
        for starts in [4, 8] {
            let mut spec = super::super::tests::tool(left, starts, true).spec();
            spec.pitch_mm *= 0.7;
            spec.depth_mm *= 0.7;
            spec.crest_width_mm *= 0.7;
            spec.root_radius_mm = spec.root_radius_mm.map(|r| r * 0.7);
            let body = Chamfer::new(Thread::new(spec).unwrap(), 0.3, None).unwrap();
            let topology = body.boundary_topology(QueryOptions::default()).unwrap();
            let euler = topology.vertices().len() as isize - topology.edges().len() as isize
                + topology
                    .faces()
                    .iter()
                    .map(|f| 2 - f.loops().len() as isize)
                    .sum::<isize>();
            assert_eq!(euler, 2);
            assert!(topology.maximum_source_error_mm() <= 1e-7);
        }
    }
}
#[test]
fn semantic_topology_is_independent_of_adaptive_tolerance_and_retains_periodic_branches() {
    for left in [false, true] {
        let source = super::super::tests::tool(left, 4, false);
        let body = Chamfer::new(
            source,
            source.spec().depth_mm,
            Some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap();
        let tight = body.boundary_topology(QueryOptions::default()).unwrap();
        let loose = body
            .boundary_topology(QueryOptions {
                tolerance: 1e-5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            tight
                .edges()
                .iter()
                .map(BoundaryEdge::source)
                .collect::<Vec<_>>(),
            loose
                .edges()
                .iter()
                .map(BoundaryEdge::source)
                .collect::<Vec<_>>()
        );
        assert_eq!(tight.vertices(), loose.vertices());
        assert!(tight.maximum_source_error_mm() <= 1e-7);
        assert!(tight.maximum_curve_binding_error_mm() <= tight.maximum_source_error_mm());
        let mut seam_count = 0;
        for (id, edge) in tight.edges().iter().enumerate() {
            let uses = tight
                .faces()
                .iter()
                .flat_map(|f| f.loops())
                .flatten()
                .filter(|u| u.edge == id)
                .collect::<Vec<_>>();
            assert_eq!(uses.len(), 2);
            assert_ne!(uses[0].forward, uses[1].forward);
            if edge.is_periodic_seam() {
                seam_count += 1;
                assert_ne!(uses[0].angular_turns, uses[1].angular_turns);
            }
            let endpoints = match edge.source() {
                SourceCurve::Intersection { phase, .. }
                | SourceCurve::EntryArc { phase, .. }
                | SourceCurve::EndProfile { phase, .. } => Some(phase),
                SourceCurve::WallRail { phase, .. } | SourceCurve::ConeRail { phase, .. } => {
                    Some([phase; 2])
                }
                _ => None,
            };
            if let Some(endpoints) = endpoints {
                for endpoint in endpoints {
                    let interval = endpoint.interval();
                    assert!(
                        interval.lower <= endpoint.nominal()
                            && endpoint.nominal() <= interval.upper
                    );
                    match endpoint.constraint() {
                        PhaseConstraint::ProfileKnot(i) => {
                            assert_eq!(endpoint.nominal(), source.profile_breaks()[i])
                        }
                        PhaseConstraint::Valley => assert!(endpoint.nominal() > 0.),
                        PhaseConstraint::EntryIntersection { band, .. } => assert!(band > 0),
                    };
                }
            }
        }
        assert_eq!(seam_count, 2);
    }
}
#[test]
fn invalid_ranges_budgets_and_every_cancelled_construction_leave_source_unchanged() {
    let body = Chamfer::new(super::super::tests::tool(false, 1, true), 0.3, None).unwrap();
    let before = body;
    for tolerance in [0., -1., f64::NAN] {
        assert_eq!(
            body.boundary_topology(QueryOptions {
                tolerance,
                ..Default::default()
            })
            .unwrap_err(),
            Error::Invalid
        );
    }
    for maximum_cells in [0, 1, 1_000_001] {
        assert_eq!(
            body.boundary_topology(QueryOptions {
                maximum_cells,
                ..Default::default()
            })
            .unwrap_err(),
            Error::Limit
        );
    }
    assert_eq!(
        body.boundary_topology(QueryOptions {
            tolerance: 1e-30,
            ..Default::default()
        })
        .unwrap_err(),
        Error::Unresolved
    );
    let count = std::cell::Cell::new(0);
    let good = body
        .boundary_topology_with_interrupt(
            QueryOptions::default(),
            Some(&|| {
                count.set(count.get() + 1);
                false
            }),
        )
        .unwrap();
    for stop in [1, 2, count.get() / 2, count.get() - 1, count.get()] {
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            body.boundary_topology_with_interrupt(
                QueryOptions::default(),
                Some(&|| {
                    calls.set(calls.get() + 1);
                    calls.get() >= stop
                })
            )
            .unwrap_err(),
            Error::Cancelled
        );
        assert_eq!(body, before);
    }
    for range in [
        [-0.1, 0.],
        [0., 1.1],
        [1., 0.],
        [f64::NAN, 1.],
        [0., f64::INFINITY],
    ] {
        assert_eq!(good.curve_bounds(0, range), Err(Error::Invalid));
    }
    assert_eq!(good.curve_bounds(usize::MAX, [0., 1.]), Err(Error::Invalid));
    assert_eq!(good.curve_sample(usize::MAX, 0.), Err(Error::Invalid));
    assert_eq!(good.curve_sample(0, f64::NAN), Err(Error::Invalid));
    let retry = body.boundary_topology(QueryOptions::default()).unwrap();
    assert_eq!(good.vertices(), retry.vertices());
}
#[test]
fn entire_shared_rails_are_certified_instead_of_only_their_endpoints() {
    let body = Chamfer::new(super::super::tests::tool(false, 1, false), 1., None).unwrap();
    let segment = body.phase_segments().unwrap()[0];
    let end = builder::endpoint(&body, segment, segment.ends[0]);
    let a = SourceCurve::WallRail {
        start: 0,
        band: 0,
        phase: end,
        active: true,
    };
    let b = SourceCurve::WallRail {
        start: 0,
        band: 0,
        phase: end,
        active: false,
    };
    let distance = |t| {
        let a = sample(curve::bounds(&body, a, [t; 2]).unwrap()).unwrap();
        let b = sample(curve::bounds(&body, b, [t; 2]).unwrap()).unwrap();
        a.nominal_mm
            .into_iter()
            .zip(b.nominal_mm)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    assert!(distance(0.) < 1.01);
    assert!(distance(1.) < 1e-12);
    assert!(distance(0.5) > 6.);
    let bound = curve::rail_binding_error(&body, a, b, 0).unwrap();
    assert!(bound > distance(0.5));
    assert_eq!(curve::rail_binding_error(&body, a, a, 0).unwrap(), 0.);
}
fn raw(neck: bool) -> Candidate {
    let body = Chamfer::new(
        super::super::tests::tool(false, 1, true),
        0.3,
        neck.then_some(TerminalIntent {
            width_mm: 0.6,
            radial_depth_mm: 0.1,
        }),
    )
    .unwrap();
    builder::candidate(
        body,
        1e-7,
        &mut Work::new(QueryOptions::default(), None).unwrap(),
    )
    .unwrap()
}
fn rejected(edit: impl FnOnce(&mut Candidate)) {
    let mut candidate = raw(true);
    edit(&mut candidate);
    assert_eq!(
        validate::check(
            &candidate,
            &mut Work::new(QueryOptions::default(), None).unwrap()
        ),
        Err(Error::Unresolved)
    );
}
#[test]
fn uncertified_producer_records_cannot_bypass_ownership_source_binding_or_point_receipts() {
    rejected(|c| c.faces.clear());
    rejected(|c| {
        c.faces.pop();
    });
    rejected(|c| c.faces[0].loops.clear());
    rejected(|c| c.faces[0].loops[0].clear());
    rejected(|c| c.faces[0].loops[0][0].edge = usize::MAX);
    rejected(|c| c.edges[0].vertices[0] = usize::MAX);
    rejected(|c| c.vertices.push(c.vertices[0]));
    rejected(|c| c.faces[0].role = Face::FarCap);
    rejected(|c| {
        let u = c.faces[1].loops[1][0];
        c.faces[1].loops[1].push(u);
    });
    rejected(|c| {
        for face in &mut c.faces {
            for wire in &mut face.loops {
                wire.reverse();
                for u in wire {
                    u.forward = !u.forward;
                }
            }
        }
    });
    rejected(|c| c.faces[0].loops[0].swap(0, 1));
    rejected(|c| c.vertices[0].coordinates_mm[0].lower = f64::NAN);
    rejected(|c| c.vertices[0].coordinates_mm[0].lower += 1.);
    rejected(|c| {
        c.vertices[0].coordinates_mm[0].lower += 1.;
        c.vertices[0].coordinates_mm[0].upper += 1.;
    });
    rejected(|c| c.vertices[0].nominal_mm[0] += 1.);
    rejected(|c| c.vertices[0].maximum_error_mm = f64::INFINITY);
    rejected(|c| c.maximum_vertex_error_mm = 0.);
    rejected(|c| {
        let edge = c
            .edges
            .iter_mut()
            .find(|e| matches!(e.source, SourceCurve::Circle { .. }))
            .unwrap();
        if let SourceCurve::Circle { radius_mm, z_mm } = edge.source {
            edge.source = SourceCurve::Circle {
                radius_mm: radius_mm + 1.,
                z_mm,
            };
        }
    });
    rejected(|c| {
        let edge = c
            .edges
            .iter_mut()
            .find(|e| matches!(e.source, SourceCurve::NeckSeam { .. }))
            .unwrap();
        if let SourceCurve::NeckSeam {
            radius_mm,
            stations_mm,
        } = edge.source
        {
            edge.source = SourceCurve::NeckSeam {
                radius_mm,
                stations_mm: [stations_mm[0], stations_mm[1] + 1.],
            };
        }
    });
    rejected(|c| {
        let segment = c.source.phase_segments().unwrap()[0];
        c.edges[0].source = SourceCurve::Intersection {
            start: u32::MAX,
            band: segment.band,
            phase: segment
                .ends
                .map(|k| builder::endpoint(&c.source, segment, k)),
            active: segment.active,
        };
    });
    // A malformed self-use must not masquerade as an allowed periodic seam.
    rejected(|c| {
        let seam = c
            .edges
            .iter_mut()
            .find(|e| matches!(e.source, SourceCurve::NeckSeam { .. }))
            .unwrap();
        seam.periodic_seam = false;
    });
    let mut no_neck = raw(false);
    no_neck.edges[0].source = SourceCurve::Circle {
        radius_mm: 1.,
        z_mm: 0.,
    };
    assert_eq!(
        validate::check(
            &no_neck,
            &mut Work::new(QueryOptions::default(), None).unwrap()
        ),
        Err(Error::Unresolved)
    );
}
fn polyhedron(points: Vec<Point>, triangles: &[[usize; 3]]) -> Candidate {
    let mut raw = raw(false);
    raw.vertices = points
        .into_iter()
        .map(|p| sample(p.map(I::point)).unwrap())
        .collect();
    let source = raw.edges[0].source;
    raw.edges.clear();
    raw.faces.clear();
    let mut edges = BTreeMap::new();
    for (index, triangle) in triangles.iter().enumerate() {
        let mut wire = Vec::new();
        for (i, &a) in triangle.iter().enumerate() {
            let b = triangle[(i + 1) % 3];
            let key = (a.min(b), a.max(b));
            let edge = *edges.entry(key).or_insert_with(|| {
                let id = raw.edges.len();
                raw.edges.push(BoundaryEdge {
                    vertices: [key.0, key.1],
                    source,
                    periodic_seam: false,
                });
                id
            });
            wire.push(EdgeUse {
                edge,
                forward: a < b,
                angular_turns: 0,
            });
        }
        raw.faces.push(BoundaryFace {
            role: raw.source.face(index as u32).unwrap(),
            loops: vec![wire],
        });
    }
    raw
}
#[test]
fn structural_preflight_rejects_disconnected_closed_shells_and_pinched_vertex_links() {
    let triangles = [
        [0, 2, 1],
        [0, 1, 3],
        [1, 2, 3],
        [2, 0, 3],
        [4, 6, 5],
        [4, 5, 7],
        [5, 6, 7],
        [6, 4, 7],
    ];
    let points = vec![
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [4., 0., 0.],
        [5., 0., 0.],
        [4., 1., 0.],
        [4., 0., 1.],
    ];
    let disconnected = polyhedron(points, &triangles);
    assert_eq!(
        validate::check(
            &disconnected,
            &mut Work::new(QueryOptions::default(), None).unwrap()
        ),
        Err(Error::Unresolved)
    );
    let triangles = [
        [0, 1, 2],
        [0, 2, 3],
        [0, 3, 4],
        [0, 4, 1],
        [5, 2, 1],
        [5, 3, 2],
        [5, 4, 3],
        [5, 1, 4],
    ];
    for merge in [0, 1] {
        let points = vec![
            [0., 0., 1.],
            [1., 0., 0.],
            [0., 1., 0.],
            [-1., 0., 0.],
            [0., -1., 0.],
            [0., 0., -1.],
        ];
        let mut pinched = polyhedron(points, &triangles);
        for edge in &mut pinched.edges {
            edge.vertices = edge.vertices.map(|i| if i == 5 { merge } else { i });
        }
        pinched.vertices.pop();
        assert_eq!(
            validate::check(
                &pinched,
                &mut Work::new(QueryOptions::default(), None).unwrap()
            ),
            Err(Error::Unresolved)
        );
    }
}
#[test]
fn every_actual_construction_checkpoint_can_cancel_without_publishing_a_partial_topology() {
    let source = Chamfer::new(
        super::super::tests::tool(true, 1, true),
        0.5,
        Some(TerminalIntent {
            width_mm: 0.6,
            radial_depth_mm: 0.1,
        }),
    )
    .unwrap();
    let count = std::cell::Cell::new(0usize);
    source
        .boundary_topology_with_interrupt(
            QueryOptions::default(),
            Some(&|| {
                count.set(count.get() + 1);
                false
            }),
        )
        .unwrap();
    for stop in 1..=count.get() {
        let step = std::cell::Cell::new(0usize);
        assert_eq!(
            source
                .boundary_topology_with_interrupt(
                    QueryOptions::default(),
                    Some(&|| {
                        step.set(step.get() + 1);
                        step.get() == stop
                    })
                )
                .unwrap_err(),
            Error::Cancelled
        );
    }
    assert!(source.boundary_topology(QueryOptions::default()).is_ok());
}
#[test]
fn source_error_budget_covers_whole_binding_and_endpoint_reconstruction_together() {
    let source = Chamfer::new(super::super::tests::tool(false, 1, true), 0.3, None).unwrap();
    let good = source.boundary_topology(QueryOptions::default()).unwrap();
    let maximum = good
        .maximum_vertex_error_mm()
        .max(good.maximum_curve_binding_error_mm());
    let tolerance = maximum.midpoint(good.maximum_source_error_mm());
    assert!(maximum < tolerance && tolerance < good.maximum_source_error_mm());
    assert_eq!(
        source
            .boundary_topology(QueryOptions {
                tolerance,
                ..Default::default()
            })
            .unwrap_err(),
        Error::Unresolved
    );
}
