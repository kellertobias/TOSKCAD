use multicad_core::{
    document::Document,
    drawings::{self, Page, VectorCurve, VectorRole},
    project_file,
};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, PlaneFrame, ProjectedVector, project_drawing,
};
use multicad_model::{Compiler, GeometryBackend, drawing_preview_sources};
use std::f64::consts::TAU;

fn fixture() -> (Document, Page) {
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

fn project_members(
    document: &Document,
    page: &Page,
    backend: GeometryBackend,
    selection: (&str, usize),
) -> Vec<Vec<ProjectedVector>> {
    let roots = drawing_preview_sources(document, page.id).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_sources_with_backend(document, 0, &roots, backend, None)
    .unwrap();
    page.views
        .iter()
        .map(|view| {
            let roots = view.source.entities();
            let input: Vec<_> = model
                .snapshot()
                .leaf_occurrences
                .iter()
                .filter(|(path, leaf)| {
                    roots.contains(&path.root)
                        && document
                            .get(leaf.producer)
                            .unwrap()
                            .name
                            .ends_with(selection.0)
                })
                .map(|(path, _)| DrawingSolid {
                    solid: model.leaf_solid(path).unwrap(),
                    hatch: None,
                })
                .collect();
            assert_eq!(input.len(), selection.1);
            project_drawing(
                &input,
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
            .unwrap()
        })
        .collect()
}

#[test]
fn saved_front_panel_buttons_project_in_all_three_authored_directions() {
    let (document, page) = fixture();
    let views = project_members(&document, &page, GeometryBackend::Rust, ("Button union", 6));
    let counts: Vec<_> = views
        .iter()
        .map(|view| {
            let visible = view
                .iter()
                .filter(|edge| edge.role == VectorRole::Visible)
                .count();
            [visible, view.len() - visible]
        })
        .collect();
    assert_eq!(counts, [[12, 36], [48, 96], [32, 112]]);
    assert!(
        views[0]
            .iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Ellipse {
        major_mm, minor_mm, start_rad, end_rad, ..
    } if major_mm == minor_mm && (end_rad - start_rad - TAU).abs() < 1e-10))
    );
    assert!(
        views[1..]
            .iter()
            .flatten()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );
}

#[test]
fn saved_socket_cap_envelopes_publish_authored_views_without_rear_rim_overdraw() {
    let (document, page) = fixture();
    let views = project_members(
        &document,
        &page,
        GeometryBackend::Rust,
        ("ISO 4762 M3 × 8 feature", 2),
    );
    assert_eq!(views.iter().map(Vec::len).collect::<Vec<_>>(), [48, 16, 48]);
    let top = &views[1];
    assert!(
        top.iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Ellipse { .. }))
    );
    assert_eq!(
        top.iter()
            .filter(|edge| edge.role == VectorRole::Visible)
            .count(),
        2
    );
}

#[cfg(feature = "occt")]
#[test]
fn saved_front_panel_button_and_screw_intervals_match_open_cascade() {
    use std::collections::BTreeMap;
    type Intervals = BTreeMap<(usize, bool, [i64; 4]), Vec<[i64; 2]>>;

    fn intervals(vectors: Vec<ProjectedVector>) -> Intervals {
        let q = |v: f64| (v * 1e6).round() as i64;
        let mut spans = Intervals::new();
        for vector in vectors {
            let hidden = match vector.role {
                VectorRole::Hidden => true,
                VectorRole::Visible => false,
                other => panic!("unexpected projection role {other:?}"),
            };
            let points = match vector.curve {
                VectorCurve::Ellipse {
                    center,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                    ..
                } => {
                    assert_eq!(major_mm, minor_mm);
                    assert!((end_rad - start_rad - TAU).abs() < 1e-10);
                    let key = [2, q(center[0]), q(center[1]), q(major_mm)];
                    spans
                        .entry((vector.source_index, hidden, key))
                        .or_default()
                        .push([0, 0]);
                    continue;
                }
                VectorCurve::Line { a, b } => vec![a, b],
                VectorCurve::BSpline {
                    degree: 1,
                    periodic: false,
                    poles,
                    knots,
                    first,
                    last,
                    ..
                } => {
                    // A circle projected edge-on may be emitted by OCCT as
                    // a chain of collinear degree-one spans.
                    assert_eq!(knots.first().copied(), Some(first));
                    assert_eq!(knots.last().copied(), Some(last));
                    poles
                }
                other => panic!("unexpected projection curve {other:?}"),
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
            spans
                .entry((vector.source_index, hidden, key))
                .or_default()
                .push(range);
        }
        for spans in spans.values_mut() {
            spans.sort_unstable();
            let mut merged: Vec<[i64; 2]> = Vec::new();
            for [a, b] in spans.drain(..) {
                if let Some(last) = merged.last_mut()
                    && a <= last[1] + 1
                {
                    last[1] = last[1].max(b);
                } else {
                    merged.push([a, b]);
                }
            }
            *spans = merged;
        }
        spans
    }

    let (document, page) = fixture();
    for selection in [("Button union", 6), ("ISO 4762 M3 × 8 feature", 2)] {
        let rust = project_members(&document, &page, GeometryBackend::Rust, selection);
        let native = project_members(&document, &page, GeometryBackend::OpenCascade, selection);
        for (rust, native) in rust.into_iter().zip(native) {
            assert_eq!(intervals(rust), intervals(native));
        }
    }
}
