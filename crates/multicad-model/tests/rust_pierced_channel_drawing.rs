use multicad_core::{
    drawings::{self, VectorCurve, VectorRole},
    project_file,
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GeometryErrorKind, PlaneFrame, ProjectedVector, Solid,
    project_drawing,
};
use multicad_model::{Compiler, GeometryBackend, drawing_preview_sources};

fn with_channel<T>(
    backend: GeometryBackend,
    inspect: impl FnOnce(&Solid, DrawingProjection) -> T,
) -> T {
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
    let roots = drawing_preview_sources(&document, page.id).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_sources_with_backend(&document, 0, &roots, backend, None)
    .unwrap();
    let view = page
        .views
        .iter()
        .find(|view| view.name == "Left end")
        .unwrap();
    let leaf = model
        .snapshot()
        .leaf_occurrences
        .iter()
        .find(|(_, leaf)| document.get(leaf.producer).unwrap().name == "Front panel sheet feature")
        .unwrap();
    inspect(
        model.leaf_solid(leaf.0).unwrap(),
        DrawingProjection {
            frame: PlaneFrame {
                origin: view.frame.origin_mm,
                u: view.frame.u,
                v: view.frame.v,
            },
            hidden_lines: view.hidden_lines,
            tangent_edges: view.tangent_edges,
            ..Default::default()
        },
    )
}

fn project(solid: &Solid, options: DrawingProjection) -> Vec<ProjectedVector> {
    project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap()
}

#[test]
fn saved_pierced_channel_end_view_preserves_the_hidden_cuts() {
    with_channel(GeometryBackend::Rust, |solid, options| {
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

#[cfg(feature = "occt")]
#[test]
fn saved_pierced_channel_end_view_matches_native_physical_intervals() {
    use multicad_core::manufacturing::PlanarEdge;
    use std::{
        collections::{BTreeMap, BTreeSet},
        f64::consts::TAU,
    };

    type Point = [i64; 2];
    type Key = (bool, [i64; 4]);
    fn q(value: f64) -> i64 {
        (value * 1e6).round() as i64
    }
    fn line_key(a: [f64; 2], b: [f64; 2]) -> [Point; 2] {
        let mut points = [a.map(q), b.map(q)];
        points.sort_unstable();
        points
    }

    let rust = with_channel(GeometryBackend::Rust, project);
    let (native, seams) = with_channel(GeometryBackend::OpenCascade, |solid, options| {
        let mut cap_lines = BTreeMap::new();
        for face in solid.faces().unwrap() {
            let Some((point, normal)) = face.plane else {
                continue;
            };
            if (normal[0].abs() - 1.).abs() > 1e-10 {
                continue;
            }
            let mut frame = options.frame;
            frame.origin[0] = point[0];
            let sourced = solid
                .planar_face_contours_with_sources(face.index, frame)
                .unwrap();
            for (ring, keys) in sourced.contours.loops.iter().zip(sourced.edge_keys) {
                for (edge, key) in ring.edges.iter().zip(keys) {
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
        (project(solid, options), web_joints)
    });

    let intervals = |vectors: Vec<ProjectedVector>, remove_seams: bool| {
        let mut spans: BTreeMap<Key, Vec<Point>> = BTreeMap::new();
        let mut removed = 0;
        for vector in vectors {
            let hidden = match vector.role {
                VectorRole::Hidden => true,
                VectorRole::Visible => false,
                other => panic!("unexpected end projection role {other:?}"),
            };
            let points = match vector.curve {
                VectorCurve::Ellipse {
                    center,
                    u,
                    v,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                } => {
                    assert_eq!(major_mm, minor_mm);
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
                    if remove_seams && seams.contains(&line_key(a, b)) {
                        removed += 1;
                        continue;
                    }
                    vec![a, b]
                }
                VectorCurve::BSpline {
                    degree: 1,
                    periodic: false,
                    poles,
                    knots,
                    first,
                    last,
                    ..
                } => {
                    assert_eq!(knots.first().copied(), Some(first));
                    assert_eq!(knots.last().copied(), Some(last));
                    poles
                }
                other => panic!("unexpected end projection curve {other:?}"),
            };
            let bounds = |axis: usize| {
                let values = points.iter().map(|p| q(p[axis]));
                [values.clone().min().unwrap(), values.max().unwrap()]
            };
            let [x, y] = [bounds(0), bounds(1)];
            if x[0] == x[1] && y[0] == y[1] {
                continue;
            }
            let (key, range) = if x[0] == x[1] {
                ([0, x[0], 0, 0], y)
            } else {
                assert_eq!(y[0], y[1]);
                ([1, y[0], 0, 0], x)
            };
            spans.entry((hidden, key)).or_default().push(range);
        }
        assert_eq!(removed, if remove_seams { 4 } else { 0 });
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
        spans
    };
    let rust = intervals(rust, false);
    assert_eq!(rust.len(), 34);
    assert_eq!(rust.values().map(Vec::len).sum::<usize>(), 38);
    assert_eq!(rust, intervals(native, true));
}
