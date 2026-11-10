use multicad_core::{
    document::Document,
    drawings::{self, DrawingVector, Page, VectorCurve, VectorRole, ViewEvaluation, ViewSource},
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
    let [view] = page.views.as_slice() else {
        panic!("expected one AP-101 view");
    };
    let ViewEvaluation::Ready { vectors, .. } = &result.views[&view.id] else {
        panic!("{:?}", result.views[&view.id]);
    };
    vectors.clone()
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
    let [saved] = page.views.as_slice() else {
        panic!("expected one AP-102 view");
    };
    let mut view = saved.clone();
    let ViewSource::Model { entities } = &mut view.source else {
        panic!("expected model sources");
    };
    let workbench = *entities
        .iter()
        .find(|&&id| document.get(id).unwrap().name == "Workbench")
        .unwrap();
    // Keep the room occluders and all sources except the cabinet, whose
    // oblique mortises require separate visibility support.
    entities.retain(|&id| document.get(id).unwrap().name != "Cabinet");

    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(&document, 0, GeometryBackend::Rust, None)
    .unwrap();
    let evaluation = model
        .drawing_view(&document, 0, &view, &Default::default())
        .unwrap();
    let ViewEvaluation::Ready { vectors, .. } = evaluation else {
        panic!("{evaluation:?}");
    };
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
