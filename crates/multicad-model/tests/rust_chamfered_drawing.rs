use multicad_core::{
    document::Document,
    drawings::{self, DrawingView, Page, VectorCurve, VectorRole, ViewEvaluation, ViewKind},
    project_file,
};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::{f64::consts::TAU, path::Path};

fn fixture() -> (Document, Page) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/toolcase.tcd");
    let mut file = std::fs::File::open(path).unwrap();
    let document = project_file::read(
        &mut file,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let page = drawings::books(&document)
        .flat_map(|book| drawings::read(&document, book).unwrap().pages)
        .find(|page| page.number == "WK-101")
        .unwrap();
    (document, page)
}

fn preview(
    document: &Document,
    page: &Page,
    backend: GeometryBackend,
) -> drawings::DrawingEvaluation {
    let roots = multicad_model::drawing_preview_sources(document, page.id).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_sources_with_backend(document, 0, &roots, backend, None)
    .unwrap();
    model
        .drawing_page_preview(document, 0, &Default::default(), page.id)
        .unwrap()
}

fn rings(view: &ViewEvaluation, role: VectorRole) -> Vec<[i64; 3]> {
    let ViewEvaluation::Ready { vectors, .. } = view else {
        panic!("{view:?}");
    };
    let mut rings = Vec::new();
    for vector in vectors.iter().filter(|v| v.role == role) {
        if let VectorCurve::Ellipse {
            center,
            major_mm,
            minor_mm,
            start_rad,
            end_rad,
            ..
        } = vector.curve
        {
            assert_eq!(major_mm, minor_mm);
            assert!((end_rad - start_rad - TAU).abs() < 1e-12);
            rings.push([center[0], center[1], major_mm].map(quantize));
        }
    }
    rings.sort_unstable();
    rings
}

fn quantize(value: f64) -> i64 {
    (value * 1e7).round() as i64
}

#[test]
fn actual_toolcase_exterior_views_show_every_chamfer_and_throat() {
    let (document, page) = fixture();
    let projected = preview(&document, &page, GeometryBackend::Rust);
    for view in &page.views {
        let evaluation = &projected.views[&view.id];
        let ViewEvaluation::Ready { vectors, .. } = evaluation else {
            panic!("{evaluation:?}");
        };
        assert_eq!(vectors.len(), 96);
        assert!(vectors.iter().all(|v| v.role == VectorRole::Visible));
        let line_count = vectors
            .iter()
            .filter(|v| matches!(v.curve, VectorCurve::Line { .. }))
            .count();
        assert_eq!(line_count, 76);
        let circles = rings(evaluation, VectorRole::Visible);
        let x_sign = view.frame.u[1];
        let mut expected = Vec::new();
        for (centers, radii) in [
            (
                &[[22., 157.5], [22., 244.5], [95., 157.5], [95., 244.5]][..],
                [1.7, 3.2],
            ),
            (
                &[
                    [30.25, 35.5],
                    [30.25, 84.5],
                    [76.6, 35.5],
                    [76.6, 84.5],
                    [171.5, 35.5],
                    [171.5, 84.5],
                ][..],
                [2.25, 4.2],
            ),
        ] {
            for &[y, z] in centers {
                for radius in radii {
                    expected.push([(y - 117.5) * x_sign, z - 155., radius].map(quantize));
                }
            }
        }
        expected.sort_unstable();
        assert_eq!(circles, expected);
    }
}

fn view(
    model: &Model,
    document: &Document,
    view: &DrawingView,
) -> Result<ViewEvaluation, multicad_model::ModelError> {
    model.drawing_view(document, 0, view, &Default::default())
}

#[test]
fn reverse_view_occludes_the_far_chamfer_and_rejects_uncertified_projection() {
    let (document, page) = fixture();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(&document, 0, GeometryBackend::Rust, None)
    .unwrap();
    let mut inside = page.views[0].clone();
    inside.frame.u = inside.frame.u.map(|x| -x);
    let from_inside = view(&model, &document, &inside).unwrap();
    assert_eq!(rings(&from_inside, VectorRole::Visible).len(), 10);
    assert!(rings(&from_inside, VectorRole::Hidden).is_empty());

    inside.hidden_lines = true;
    let with_hidden = view(&model, &document, &inside).unwrap();
    assert_eq!(rings(&with_hidden, VectorRole::Visible).len(), 10);
    assert_eq!(rings(&with_hidden, VectorRole::Hidden).len(), 10);
    assert!(
        rings(&with_hidden, VectorRole::Visible)
            .iter()
            .all(|r| r[2] <= quantize(2.25))
    );
    assert!(
        rings(&with_hidden, VectorRole::Hidden)
            .iter()
            .all(|r| r[2] >= quantize(3.2))
    );

    inside.kind = ViewKind::Detail {
        center_mm: [0.; 2],
        radius_mm: 100.,
    };
    assert!(
        view(&model, &document, &inside)
            .unwrap_err()
            .to_string()
            .contains("uncropped")
    );
    inside.kind = ViewKind::Projection;
    let angle = std::f64::consts::FRAC_1_SQRT_2;
    inside.frame.u = [angle, angle, 0.];
    assert!(view(&model, &document, &inside).is_err());
}

#[cfg(feature = "occt")]
#[test]
fn actual_toolcase_axial_vectors_match_open_cascade() {
    let (document, page) = fixture();
    let rust = preview(&document, &page, GeometryBackend::Rust);
    let occt = preview(&document, &page, GeometryBackend::OpenCascade);
    for view in &page.views {
        let compare = |source: &drawings::DrawingEvaluation| {
            let result = &source.views[&view.id];
            let ViewEvaluation::Ready { vectors, .. } = result else {
                panic!("{result:?}");
            };
            let mut lines: Vec<_> = vectors
                .iter()
                .filter_map(|v| {
                    let VectorCurve::Line { a, b } = v.curve else {
                        return None;
                    };
                    let mut points = [a.map(quantize), b.map(quantize)];
                    points.sort_unstable();
                    Some(points)
                })
                .collect();
            lines.sort_unstable();
            (lines, rings(result, VectorRole::Visible))
        };
        assert_eq!(compare(&rust), compare(&occt), "{}", view.name);
    }
}
