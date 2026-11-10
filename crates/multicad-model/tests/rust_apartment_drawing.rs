use multicad_core::{
    document::Document,
    drawings::{self, DrawingVector, Page, VectorCurve, VectorRole, ViewEvaluation},
    project_file,
};
use multicad_model::{Compiler, GeometryBackend, drawing_preview_sources};
use std::f64::consts::{PI, TAU};

fn fixture(number: &str) -> (Document, Page) {
    let bytes = include_bytes!("../../../examples/apartment.tcd");
    let document = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let page = drawings::books(&document)
        .flat_map(|id| drawings::read(&document, id).unwrap().pages)
        .find(|page| page.number == number)
        .unwrap();
    (document, page)
}

fn preview(document: &Document, page: &Page, backend: GeometryBackend) -> Vec<DrawingVector> {
    let [vectors] = preview_views(document, page, backend)
        .try_into()
        .expect("expected one saved drawing view");
    vectors
}

fn preview_views(
    document: &Document,
    page: &Page,
    backend: GeometryBackend,
) -> Vec<Vec<DrawingVector>> {
    let roots = drawing_preview_sources(document, page.id).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_sources_with_backend(document, 0, &roots, backend, None)
    .unwrap();
    let result = model
        .drawing_page_preview(document, 0, &Default::default(), page.id)
        .unwrap();
    page.views
        .iter()
        .map(|view| {
            let ViewEvaluation::Ready { vectors, .. } = &result.views[&view.id] else {
                panic!(
                    "{} / {}: {:?}",
                    page.number, view.name, result.views[&view.id]
                );
            };
            vectors.clone()
        })
        .collect()
}

#[test]
fn apartment_floor_plan_publishes_the_real_worktop_apertures() {
    let (document, page) = fixture("AP-101");
    let vectors = preview(&document, &page, GeometryBackend::Rust);
    assert_eq!(
        vectors
            .iter()
            .filter(|v| v.role == VectorRole::Visible)
            .count(),
        235
    );
    assert_eq!(
        vectors
            .iter()
            .filter(|v| v.role == VectorRole::Section)
            .count(),
        60
    );
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));

    let workbench = page.views[0]
        .source
        .entities()
        .into_iter()
        .find(|&id| document.get(id).unwrap().name == "Workbench")
        .unwrap();
    let worktop: Vec<_> = vectors.iter().filter(|v| v.source == workbench).collect();
    assert_eq!(worktop.len(), 104);
    assert!(worktop.iter().all(|v| v.role == VectorRole::Visible));
    let mut centers = worktop
        .iter()
        .filter_map(|v| {
            let VectorCurve::Ellipse {
                center,
                major_mm,
                minor_mm,
                start_rad,
                end_rad,
                ..
            } = v.curve
            else {
                assert!(matches!(v.curve, VectorCurve::Line { .. }));
                return None;
            };
            assert_eq!([major_mm, minor_mm], [10.; 2]);
            assert!((end_rad - start_rad - TAU).abs() < 1e-12);
            Some(center.map(|v| v.round() as i64))
        })
        .collect::<Vec<_>>();
    centers.sort_unstable();
    let expected: Vec<_> = (0..10)
        .flat_map(|x| (0..10).map(move |y| [540 + 80 * x, 2940 + 80 * y]))
        .collect();
    assert_eq!(centers, expected);
}

#[cfg(feature = "occt")]
#[test]
fn apartment_floor_plan_structural_vectors_match_open_cascade() {
    let (document, page) = fixture("AP-101");
    let canonical = |vectors: Vec<DrawingVector>| {
        let q = |v: f64| (1e6 * v).round() as i64;
        let mut output = Vec::new();
        for vector in vectors {
            // The engines anchor hatch stroke patterns independently.
            let role = match vector.role {
                VectorRole::Visible => 0,
                VectorRole::Section => 1,
                VectorRole::Hatch => continue,
                VectorRole::Hidden => panic!("hidden vector in visible-only plan"),
            };
            let geometry = match vector.curve {
                VectorCurve::Line { a, b } => {
                    let mut points = [a.map(q), b.map(q)];
                    points.sort_unstable();
                    vec![0, points[0][0], points[0][1], points[1][0], points[1][1]]
                }
                VectorCurve::Ellipse {
                    center,
                    u,
                    v,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                } => {
                    let sweep = end_rad - start_rad;
                    let mut key = vec![
                        1,
                        q(center[0]),
                        q(center[1]),
                        q(major_mm),
                        q(minor_mm),
                        q(sweep),
                    ];
                    if (sweep - TAU).abs() > 1e-10 {
                        let at = |t: f64| {
                            let (sin, cos) = t.sin_cos();
                            std::array::from_fn::<_, 2, _>(|i| {
                                q(center[i] + major_mm * u[i] * cos + minor_mm * v[i] * sin)
                            })
                        };
                        let mut ends = [at(start_rad), at(end_rad)];
                        ends.sort_unstable();
                        key.extend(ends.into_iter().flatten());
                        key.extend(at(start_rad.midpoint(end_rad)));
                    }
                    key
                }
                other => panic!("unexpected structural curve {other:?}"),
            };
            output.push((vector.source, role, geometry));
        }
        output.sort_unstable();
        output
    };
    let rust = canonical(preview(&document, &page, GeometryBackend::Rust));
    let native = canonical(preview(&document, &page, GeometryBackend::OpenCascade));
    assert_eq!(rust.len(), 295);
    assert_eq!(rust, native);
}

#[test]
fn apartment_isometric_projects_workbench_apertures_with_room_occlusion() {
    let (document, page) = fixture("AP-102");
    let source = |name| {
        page.views[0]
            .source
            .entities()
            .into_iter()
            .find(|&id| document.get(id).unwrap().name == name)
            .unwrap()
    };
    let workbench = source("Workbench");
    let cabinet = source("Cabinet");
    let vectors = preview(&document, &page, GeometryBackend::Rust);
    assert_eq!(vectors.len(), 212);
    assert!(vectors.iter().all(|v| v.role == VectorRole::Visible));
    let cabinet_vectors: Vec<_> = vectors.iter().filter(|v| v.source == cabinet).collect();
    assert_eq!(cabinet_vectors.len(), 52);
    assert_eq!(
        cabinet_vectors
            .iter()
            .filter(|v| matches!(v.curve, VectorCurve::Ellipse { major_mm: 2.5, .. }))
            .count(),
        13
    );

    let worktop: Vec<_> = vectors.iter().filter(|v| v.source == workbench).collect();
    assert_eq!(worktop.len(), 20);
    assert!(worktop.iter().all(|v| v.role == VectorRole::Visible));
    let mut centers: Vec<_> = worktop
        .iter()
        .filter_map(|v| {
            let VectorCurve::Ellipse {
                center,
                major_mm,
                minor_mm,
                start_rad,
                end_rad,
                ..
            } = v.curve
            else {
                assert!(matches!(v.curve, VectorCurve::Line { .. }));
                return None;
            };
            assert_eq!(major_mm, 10.);
            assert!((minor_mm - 10. / 3.0_f64.sqrt()).abs() < 1e-10);
            assert!((end_rad - start_rad - PI).abs() < 1e-10);
            Some(center[0])
        })
        .collect();
    centers.sort_by(f64::total_cmp);
    let expected = (0..10)
        .rev()
        .map(|index| -1187.9393923933994 - 80. * f64::from(index) / 2.0_f64.sqrt());
    assert_eq!(centers.len(), 10);
    assert!(
        centers
            .into_iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 1e-9)
    );
}

#[cfg(feature = "occt")]
#[test]
fn apartment_isometric_and_workshop_visible_intervals_match_open_cascade() {
    use multicad_core::document::EntityId;
    use std::collections::BTreeMap;

    fn intervals(vectors: Vec<DrawingVector>) -> BTreeMap<(EntityId, Vec<i64>), Vec<[i64; 2]>> {
        let quantize = |v: f64| (v * 1e6).round() as i64;
        let first_quadrant = |[u, v]: [f64; 2]| u > 1e-10 || (u.abs() <= 1e-10 && v > 0.);
        let mut spans = BTreeMap::<_, Vec<[f64; 2]>>::new();
        for vector in vectors {
            assert_eq!(vector.role, VectorRole::Visible);
            match vector.curve {
                VectorCurve::Line { a, b } => {
                    let [x, y] = [b[0] - a[0], b[1] - a[1]];
                    let length = x.hypot(y);
                    let mut axis = [x / length, y / length];
                    if !first_quadrant(axis) {
                        axis = axis.map(|v| -v);
                    }
                    let [x, y] = axis;
                    let key = vec![0, quantize(x), quantize(y), quantize(a[0] * y - a[1] * x)];
                    let mut range = [a, b].map(|p| p[0] * x + p[1] * y);
                    range.sort_by(f64::total_cmp);
                    spans.entry((vector.source, key)).or_default().push(range);
                }
                VectorCurve::Ellipse {
                    center,
                    u,
                    v,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                } => {
                    if (major_mm - minor_mm).abs() < 1e-9
                        && (end_rad - start_rad - TAU).abs() < 1e-9
                    {
                        let key = vec![
                            2,
                            quantize(center[0]),
                            quantize(center[1]),
                            quantize(major_mm),
                        ];
                        spans
                            .entry((vector.source, key))
                            .or_default()
                            .push([0., TAU]);
                        continue;
                    }
                    let sign = u[0] * v[1] - u[1] * v[0];
                    let shift = if first_quadrant(u) { 0. } else { PI };
                    let u = if shift == 0. { u } else { u.map(|x| -x) };
                    let key = [center[0], center[1], u[0], u[1], major_mm, minor_mm].map(quantize);
                    let entry = spans
                        .entry((vector.source, [vec![1], key.to_vec()].concat()))
                        .or_default();
                    let directed_start = if sign > 0. { start_rad } else { -end_rad };
                    let start = (directed_start + shift).rem_euclid(TAU);
                    let start = if start >= TAU - 1e-9 { 0. } else { start };
                    let end = start + end_rad - start_rad;
                    entry.push([start, end.min(TAU)]);
                    if end > TAU + 1e-9 {
                        entry.push([0., end - TAU]);
                    }
                }
                other => panic!("unexpected isometric curve {other:?}"),
            }
        }
        spans
            .into_iter()
            .map(|(key, mut ranges)| {
                ranges.sort_by(|a, b| a[0].total_cmp(&b[0]));
                let mut merged: Vec<[f64; 2]> = Vec::new();
                for [a, b] in ranges {
                    if let Some(last) = merged.last_mut()
                        && a <= last[1] + 1e-6
                    {
                        last[1] = last[1].max(b);
                    } else {
                        merged.push([a, b]);
                    }
                }
                (
                    key,
                    merged.into_iter().map(|span| span.map(quantize)).collect(),
                )
            })
            .collect()
    }

    let (document, page) = fixture("AP-102");
    let rust = intervals(preview(&document, &page, GeometryBackend::Rust));
    let native = intervals(preview(&document, &page, GeometryBackend::OpenCascade));
    assert_eq!(rust.len(), 181);
    assert_eq!(rust, native);

    let (document, page) = fixture("AP-103");
    let rust = preview_views(&document, &page, GeometryBackend::Rust);
    let native = preview_views(&document, &page, GeometryBackend::OpenCascade);
    assert_eq!(rust.len(), 2);
    assert_eq!(rust.len(), native.len());
    for (rust, native) in rust.into_iter().zip(native) {
        assert_eq!(intervals(rust), intervals(native));
    }
}

#[test]
fn apartment_workshop_references_publish_both_saved_views() {
    let (document, page) = fixture("AP-103");
    let views = preview_views(&document, &page, GeometryBackend::Rust);
    assert_eq!(views.iter().map(Vec::len).collect::<Vec<_>>(), [119, 44]);
    assert!(
        views
            .iter()
            .flatten()
            .all(|vector| vector.role == VectorRole::Visible)
    );
    assert_eq!(
        views[0]
            .iter()
            .filter(|vector| matches!(
                vector.curve,
                VectorCurve::Ellipse { major_mm: 10., minor_mm: 10., start_rad, end_rad, .. }
                    if (end_rad - start_rad - TAU).abs() < 1e-10
            ))
            .count(),
        100
    );
}
