use multicad_core::{
    drawings::{self, VectorCurve, VectorRole},
    project_file,
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GeometryErrorKind, PlaneFrame, ProjectedVector, Solid,
    project_drawing,
};
use multicad_model::{Compiler, GeometryBackend, drawing_preview_sources};

fn fixture() -> (multicad_core::document::Document, drawings::Page) {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/front-panel.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let page = drawings::books(&document)
        .flat_map(|id| drawings::read(&document, id).unwrap().pages)
        .find(|page| page.number == "FP-101")
        .unwrap();
    (document, page)
}

fn with_channel<T>(
    backend: GeometryBackend,
    inspect: impl FnOnce(&Solid, [DrawingProjection; 3]) -> T,
) -> T {
    let (document, page) = fixture();
    let roots = drawing_preview_sources(&document, page.id).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_sources_with_backend(&document, 0, &roots, backend, None)
    .unwrap();
    let leaf = model
        .snapshot()
        .leaf_occurrences
        .iter()
        .find(|(_, leaf)| document.get(leaf.producer).unwrap().name == "Front panel sheet feature")
        .unwrap();
    let options = ["Front", "Top", "Left end"].map(|name| {
        let view = page.views.iter().find(|view| view.name == name).unwrap();
        DrawingProjection {
            frame: PlaneFrame {
                origin: view.frame.origin_mm,
                u: view.frame.u,
                v: view.frame.v,
            },
            hidden_lines: view.hidden_lines,
            tangent_edges: view.tangent_edges,
            ..Default::default()
        }
    });
    inspect(model.leaf_solid(leaf.0).unwrap(), options)
}

fn project(solid: &Solid, options: DrawingProjection) -> Vec<ProjectedVector> {
    project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap()
}

#[test]
fn saved_pierced_channel_end_view_preserves_the_hidden_cuts() {
    with_channel(GeometryBackend::Rust, |solid, [_, _, options]| {
        let vectors = project(solid, options);
        assert_eq!(vectors.len(), 88);
        let visible: Vec<_> = vectors
            .iter()
            .filter(|vector| vector.role == VectorRole::Visible)
            .map(|vector| &vector.curve)
            .collect();
        assert_eq!(visible.len(), 12);
        assert_eq!(
            visible
                .iter()
                .filter(|curve| matches!(curve, VectorCurve::Ellipse { .. }))
                .count(),
            4,
        );
        assert!(vectors.iter().any(|vector| {
            vector.role == VectorRole::Hidden
                && matches!(vector.curve, VectorCurve::Line { a, b }
                    if (a[0]-b[0]).abs() > 1. && (a[1]-31.25).abs() < 1e-8 && (b[1]-31.25).abs() < 1e-8)
        }));
        let visible_only = project(
            solid,
            DrawingProjection {
                hidden_lines: false,
                ..options
            },
        );
        assert_eq!(
            visible,
            visible_only.iter().map(|v| &v.curve).collect::<Vec<_>>()
        );

        let tiny_budget = DrawingProjection {
            maximum_curves: 12,
            ..options
        };
        let input = DrawingSolid { solid, hatch: None };
        assert_eq!(
            project_drawing(&[input], tiny_budget).unwrap_err().kind,
            GeometryErrorKind::Limit,
        );
    });
}

#[test]
fn saved_pierced_channel_projects_both_transverse_views_with_hidden_lines() {
    with_channel(GeometryBackend::Rust, |solid, [front, top, _]| {
        for (options, [visible_count, hidden_count], circular_count) in
            [(front, [18, 38], 10), (top, [6, 62], 2)]
        {
            let vectors = project(solid, options);
            let (visible, hidden): (Vec<_>, Vec<_>) = vectors
                .into_iter()
                .partition(|vector| vector.role == VectorRole::Visible);
            assert_eq!([visible.len(), hidden.len()], [visible_count, hidden_count]);
            assert!(
                hidden
                    .iter()
                    .all(|vector| vector.role == VectorRole::Hidden)
            );
            assert_eq!(
                visible
                    .iter()
                    .filter(|vector| matches!(vector.curve, VectorCurve::Ellipse { .. }))
                    .count(),
                circular_count
            );
            assert_eq!(
                visible,
                project(
                    solid,
                    DrawingProjection {
                        hidden_lines: false,
                        ..options
                    }
                )
            );
        }

        let a = std::f64::consts::FRAC_1_SQRT_2;
        let oblique_openings = DrawingProjection {
            frame: PlaneFrame {
                v: [0., a, a],
                ..front.frame
            },
            ..front
        };
        let error =
            project_drawing(&[DrawingSolid { solid, hatch: None }], oblique_openings).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("face-on or edge-on piercings"));
    });
}

#[cfg(feature = "occt")]
mod native {
    use super::*;
    use multicad_core::manufacturing::PlanarEdge;
    use std::{
        collections::{BTreeMap, BTreeSet},
        f64::consts::TAU,
    };

    type Point = [i64; 2];
    type Segment = [Point; 2];
    type SpanKey = (bool, [i64; 4]);
    type Spans = BTreeMap<SpanKey, Vec<Point>>;

    fn q(value: f64) -> i64 {
        (value * 1e6).round() as i64
    }

    fn line_key(a: [f64; 2], b: [f64; 2]) -> Segment {
        let mut points = [a.map(q), b.map(q)];
        points.sort_unstable();
        points
    }

    fn construction_seams(solid: &Solid, end_view: DrawingProjection) -> [Vec<Segment>; 3] {
        let mut cap_lines = BTreeMap::new();
        let mut cap_edges = BTreeMap::<_, Vec<PlanarEdge>>::new();
        for face in solid.faces().unwrap() {
            let Some((point, normal)) = face.plane else {
                continue;
            };
            if (normal[0].abs() - 1.).abs() > 1e-10 {
                continue;
            }
            let mut frame = end_view.frame;
            frame.origin[0] = point[0];
            let sourced = solid
                .planar_face_contours_with_sources(face.index, frame)
                .unwrap();
            for (ring, keys) in sourced.contours.loops.iter().zip(sourced.edge_keys) {
                for (edge, key) in ring.edges.iter().zip(keys) {
                    cap_edges.entry(q(point[0])).or_default().push(edge.clone());
                    if let PlanarEdge::Line { a, b } = *edge {
                        let count = cap_lines
                            .entry((q(point[0]), key, line_key(a, b)))
                            .or_insert(0);
                        *count += 1;
                    }
                }
            }
        }
        let seams: BTreeSet<_> = cap_lines
            .into_iter()
            .filter_map(|((depth, _, line), count)| (count == 2).then_some((depth, line)))
            .collect();
        let web_joints = [
            line_key([0., 0.], [1.5, 0.]),
            line_key([0., 80.], [1.5, 80.]),
        ];
        assert_eq!(
            seams,
            [q(0.), q(260.)]
                .into_iter()
                .flat_map(|depth| web_joints.map(|line| (depth, line)))
                .collect(),
        );
        // Each seam endpoint joins a straight web edge to a tangential
        // bend arc. Extruding this joint leaves no physical crease.
        for (depth, [a, b]) in seams {
            let edges = &cap_edges[&depth];
            for end in [a, b] {
                let (tangent, point) = edges
                    .iter()
                    .find_map(|edge| {
                        let PlanarEdge::Line { a, b } = *edge else {
                            return None;
                        };
                        if web_joints.contains(&line_key(a, b)) {
                            return None;
                        }
                        let direction = [b[0] - a[0], b[1] - a[1]];
                        [a, b]
                            .into_iter()
                            .find(|p| p.map(q) == end)
                            .map(|p| (direction, p))
                    })
                    .unwrap();
                let tangent_bend = edges.iter().any(|edge| {
                    let PlanarEdge::Arc { a, b, center, .. } = *edge else {
                        return false;
                    };
                    [a, b].into_iter().any(|p| p.map(q) == end)
                        && (tangent[0] * (point[0] - center[0])
                            + tangent[1] * (point[1] - center[1]))
                            .abs()
                            < 1e-8
                });
                assert!(tangent_bend);
            }
        }
        let bend_rails = [0., 80.].map(|z| line_key([0., z], [260., z]));
        [bend_rails.as_slice(), &[], web_joints.as_slice()].map(<[_]>::to_vec)
    }

    fn intervals<'a>(
        vectors: impl IntoIterator<Item = (VectorRole, &'a VectorCurve)>,
        skip: impl Fn(bool, Segment) -> bool,
    ) -> (Spans, usize) {
        let mut spans: Spans = BTreeMap::new();
        let mut removed = 0;
        for (role, curve) in vectors {
            let hidden = match role {
                VectorRole::Hidden => true,
                VectorRole::Visible => false,
                other => panic!("unexpected projection role {other:?}"),
            };
            let line;
            let points = match *curve {
                VectorCurve::Ellipse {
                    center,
                    u,
                    v,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                } => {
                    assert!((major_mm - minor_mm).abs() < 1e-9);
                    let direct = u[0] * v[1] > u[1] * v[0];
                    let angle = if direct { start_rad } else { -end_rad };
                    let start = (u[1].atan2(u[0]) + angle).rem_euclid(TAU);
                    let start = if start > TAU - 1e-9 { 0. } else { start };
                    let end = start + end_rad - start_rad;
                    let key = [2, q(center[0]), q(center[1]), q(major_mm)];
                    let entry = spans.entry((hidden, key)).or_default();
                    entry.push([q(start), q(end.min(TAU))]);
                    if end > TAU + 1e-9 {
                        entry.push([0, q(end - TAU)]);
                    }
                    continue;
                }
                VectorCurve::Line { a, b } => {
                    if skip(hidden, line_key(a, b)) {
                        removed += 1;
                        continue;
                    }
                    line = [a, b];
                    &line[..]
                }
                VectorCurve::BSpline {
                    degree: 1,
                    periodic: false,
                    ref poles,
                    ref knots,
                    first,
                    last,
                    ..
                } => {
                    assert_eq!(knots.first().copied(), Some(first));
                    assert_eq!(knots.last().copied(), Some(last));
                    poles
                }
                ref other => panic!("unexpected projection curve {other:?}"),
            };
            let a = points[0];
            let length = |b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
            let b = points
                .iter()
                .copied()
                .max_by(|a, b| length(*a).total_cmp(&length(*b)))
                .unwrap();
            let length = length(b);
            if length <= 1e-7 {
                continue;
            }
            let [mut x, mut y] = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            if x < -1e-10 || (x.abs() <= 1e-10 && y < 0.) {
                [x, y] = [-x, -y];
            }
            let offset = a[0] * y - a[1] * x;
            let along = points.iter().map(|p| {
                assert!((p[0] * y - p[1] * x - offset).abs() < 1e-7);
                q(p[0] * x + p[1] * y)
            });
            let range = [along.clone().min().unwrap(), along.max().unwrap()];
            spans
                .entry((hidden, [1, q(x), q(y), q(offset)]))
                .or_default()
                .push(range);
        }
        for values in spans.values_mut() {
            values.sort_unstable();
            let mut merged: Vec<Point> = Vec::new();
            for [a, b] in values.drain(..) {
                if let Some(last) = merged.last_mut()
                    && a <= last[1] + 1
                {
                    last[1] = last[1].max(b);
                } else {
                    merged.push([a, b]);
                }
            }
            *values = merged;
        }
        (spans, removed)
    }

    #[test]
    fn saved_pierced_channel_views_match_native_physical_intervals() {
        let rust = with_channel(GeometryBackend::Rust, |solid, views| {
            views.map(|options| project(solid, options))
        });
        let (native, seams) = with_channel(GeometryBackend::OpenCascade, |solid, views| {
            (
                views.map(|options| project(solid, options)),
                construction_seams(solid, views[2]),
            )
        });

        let mut totals = [0; 2];
        for ((rust, native), seams) in rust.into_iter().zip(native).zip(seams) {
            let source = native[0].source_index;
            assert!(
                rust.iter()
                    .chain(&native)
                    .all(|vector| vector.source_index == source)
            );
            let (rust, _) = intervals(rust.iter().map(|v| (v.role, &v.curve)), |_, _| false);
            let (native, removed) =
                intervals(native.iter().map(|v| (v.role, &v.curve)), |_, line| {
                    seams.contains(&line)
                });
            assert_eq!(removed, seams.len() * 2);
            assert_eq!(rust, native);
            totals[0] += rust.len();
            totals[1] += rust.values().map(Vec::len).sum::<usize>();
        }
        assert_eq!(totals, [101, 109]);
    }

    #[test]
    fn saved_front_panel_assembly_matches_native_physical_intervals() {
        let rust = assembly_views(GeometryBackend::Rust);
        let native = assembly_views(GeometryBackend::OpenCascade);
        let seams = with_channel(GeometryBackend::OpenCascade, |solid, views| {
            construction_seams(solid, views[2])
        });
        let mut totals = [0; 2];
        for (index, ((rust, native), seams)) in rust.into_iter().zip(native).zip(seams).enumerate()
        {
            let source = native[0].source;
            assert!(
                rust.iter()
                    .chain(&native)
                    .all(|vector| vector.source == source)
            );
            let (rust, _) = intervals(rust.iter().map(|v| (v.role, &v.curve)), |_, _| false);
            let (native, removed) =
                intervals(native.iter().map(|v| (v.role, &v.curve)), |hidden, line| {
                    (index != 2 || hidden) && seams.contains(&line)
                });
            assert_eq!(removed, [4, 0, 4][index]);
            assert_eq!(rust, native, "physical interval union in view {index}");
            totals[0] += rust.len();
            totals[1] += rust.values().map(Vec::len).sum::<usize>();
        }
        assert_eq!(totals, [310, 374]);
    }
}

fn assembly_views(backend: GeometryBackend) -> [Vec<drawings::DrawingVector>; 3] {
    let (document, page) = fixture();
    let roots = drawing_preview_sources(&document, page.id).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_sources_with_backend(&document, 0, &roots, backend, None)
    .unwrap();
    let result = model
        .drawing_page_preview(&document, 0, &Default::default(), page.id)
        .unwrap();
    ["Front", "Top", "Left end"].map(|name| {
        let view = page.views.iter().find(|view| view.name == name).unwrap();
        let evaluation = &result.views[&view.id];
        let drawings::ViewEvaluation::Ready { vectors, .. } = evaluation else {
            panic!("saved {name} view: {evaluation:?}");
        };
        vectors.clone()
    })
}

#[test]
fn saved_front_panel_assembly_draws_the_near_window_and_six_buttons() {
    let views = assembly_views(GeometryBackend::Rust);
    for vectors in &views {
        assert!(vectors.iter().any(|v| v.role == VectorRole::Visible));
        assert!(vectors.iter().any(|v| v.role == VectorRole::Hidden));
    }
    let front = &views[0];
    assert!(front.iter().any(|v| v.role == VectorRole::Visible
        && matches!(
            v.curve,
            VectorCurve::Line {
                a: [34., 27.5],
                b: [106., 27.5]
            }
        )));
    assert_eq!(
        front
            .iter()
            .filter(|v| v.role == VectorRole::Visible
                && matches!(
                    v.curve,
                    VectorCurve::Ellipse {
                        major_mm: 7.5,
                        minor_mm: 7.5,
                        ..
                    }
                ))
            .count(),
        6
    );
}

#[test]
fn assembled_bent_channel_only_reveals_foreign_edges_through_material_openings() {
    use multicad_kernel::{GeometryPolicy, rust_kernel::Rational};

    with_channel(GeometryBackend::Rust, |channel, [front, _, _]| {
        let frame = front.frame;
        let n = frame.normal();
        let behind = |u| PlaneFrame {
            origin: std::array::from_fn(|i| {
                frame.origin[i] + u * frame.u[i] + 40. * frame.v[i] - 300. * n[i]
            }),
            ..frame
        };
        let make_box = |id, u| {
            Solid::rust_box(
                id,
                [2; 3].map(|value| Rational::new(value, 1).unwrap()),
                behind(u),
                GeometryPolicy::default(),
            )
            .unwrap()
        };
        let [through_window, behind_web] = [make_box(1, 50.), make_box(2, 20.)];
        let scene = [channel, &through_window, &behind_web]
            .map(|solid| DrawingSolid { solid, hatch: None });
        let vectors = project_drawing(&scene, front).unwrap();
        for (source, visible) in [(1, true), (2, false)] {
            let vectors: Vec<_> = vectors
                .iter()
                .filter(|v| v.source_index == source)
                .collect();
            assert!(!vectors.is_empty());
            assert_eq!(
                vectors.iter().any(|v| v.role == VectorRole::Visible),
                visible
            );
        }
    });
}

#[test]
fn assembled_bent_channel_refuses_an_uncertified_oblique_view() {
    use multicad_kernel::{GeometryPolicy, rust_kernel::Rational};

    with_channel(GeometryBackend::Rust, |channel, [front, _, _]| {
        let box_solid = Solid::rust_box(
            1,
            [2; 3].map(|value| Rational::new(value, 1).unwrap()),
            PlaneFrame {
                origin: [50., 5., 40.],
                ..Default::default()
            },
            GeometryPolicy::default(),
        )
        .unwrap();
        let scene = [channel, &box_solid].map(|solid| DrawingSolid { solid, hatch: None });
        let a = std::f64::consts::FRAC_1_SQRT_2;
        let oblique = DrawingProjection {
            frame: PlaneFrame {
                v: [0., a, a],
                ..front.frame
            },
            ..front
        };
        let error = project_drawing(&scene, oblique).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("cardinal quarter bend spokes"));
    });
}

#[test]
fn transverse_projection_requires_coincident_or_separate_openings_in_two_layers() {
    use multicad_core::{document::EntityId, references::Origin};
    use multicad_kernel::{CurvedEdge2d, CurvedProfile, GeometryPolicy, SweepOpening};

    let origin = |role: &str| Origin {
        source: EntityId::new(),
        role: role.into(),
    };
    let vertices = [
        [0., 0.],
        [20., 0.],
        [20., 2.],
        [4., 2.],
        [4., 8.],
        [20., 8.],
        [20., 10.],
        [0., 10.],
    ];
    let sides = vertices.map(|_| origin("stock side"));
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: vertices
            .into_iter()
            .zip(vertices.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let policy = GeometryPolicy::default();
    let stock =
        Solid::rust_extrude_normal_profile_named(&profile, 25., EntityId::new(), &sides, policy)
            .unwrap();
    let make_opening = |wall: [usize; 2], y, travel, x| SweepOpening {
        walls: wall.map(|i| sides[i].clone()),
        frame: PlaneFrame {
            origin: [0., y, 0.],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        through_mm: [0., travel, 0.],
        outline: vec![CurvedEdge2d::Circle {
            center: [x, 12.],
            radius_mm: 1.,
        }],
        origins: vec![origin("round opening")],
    };
    let pierced = |x| {
        stock
            .rust_pierce_normal_profile(
                &[
                    make_opening([0, 2], 0., 2., 10.),
                    make_opening([6, 4], 10., -2., x),
                ],
                policy,
            )
            .unwrap()
    };
    let options = DrawingProjection {
        frame: PlaneFrame {
            origin: [0.; 3],
            u: [0., 0., 1.],
            v: [1., 0., 0.],
        },
        hidden_lines: true,
        ..Default::default()
    };
    for x in [10., 14.] {
        let solid = pierced(x);
        let vectors = project(&solid, options);
        let rims: Vec<_> = vectors
            .iter()
            .filter(|v| matches!(v.curve, VectorCurve::Ellipse { .. }))
            .collect();
        assert_eq!(rims.len(), 4);
        assert_eq!(
            rims.iter()
                .filter(|v| v.role == VectorRole::Visible)
                .count(),
            1
        );
    }
    let offset = pierced(10.5);
    let error = project_drawing(
        &[DrawingSolid {
            solid: &offset,
            hatch: None,
        }],
        options,
    )
    .unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("overlapping projected piercings"));
}
