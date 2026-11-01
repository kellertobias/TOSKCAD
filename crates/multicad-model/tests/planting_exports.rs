#[allow(dead_code)]
#[path = "../../multicad-app/examples/planting_review.rs"]
mod garden;
use multicad_core::drawings::{self, VectorCurve, ViewEvaluation};
fn extent(points: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in points {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}
fn pdf_paths(bytes: &[u8]) -> Vec<Vec<[f64; 2]>> {
    let text = String::from_utf8_lossy(bytes);
    let begin = text.find("stream\n").unwrap() + 7;
    let stream = &text[begin..];
    let stream = &stream[..stream.find("endstream").unwrap()];
    stream
        .lines()
        .filter(|line| line.ends_with(" S"))
        .filter_map(|line| {
            let tokens: Vec<_> = line.split_whitespace().collect();
            let mut points = vec![];
            for (i, token) in tokens.iter().enumerate() {
                if *token == "m" || *token == "l" {
                    points.push([
                        tokens[i - 2].parse::<f64>().unwrap() / drawings::pdf::PT_PER_MM,
                        tokens[i - 1].parse::<f64>().unwrap() / drawings::pdf::PT_PER_MM,
                    ]);
                }
            }
            (points.len() > 16).then_some(points)
        })
        .collect()
}
fn dxf_paths(text: &str) -> Vec<Vec<[f64; 2]>> {
    // Independent file parser, rather than reading the shared PageRender back.
    let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(text.as_bytes())).unwrap();
    let mut paths = vec![];
    let mut current: Vec<[f64; 2]> = vec![];
    for entity in drawing.entities() {
        if entity.common.layer != "VIEW_VISIBLE" {
            continue;
        }
        let dxf::entities::EntityType::Line(line) = &entity.specific else {
            continue;
        };
        let (a, b) = ([line.p1.x, line.p1.y], [line.p2.x, line.p2.y]);
        if current
            .last()
            .is_none_or(|p| (p[0] - a[0]).hypot(p[1] - a[1]) > 1e-5)
        {
            current = vec![a];
        }
        current.push(b);
        if (current[0][0] - b[0]).hypot(current[0][1] - b[1]) < 1e-5 {
            if current.len() > 16 {
                paths.push(current.clone());
            }
            current.clear();
        }
    }
    paths
}
fn check_sizes(paths: Vec<Vec<[f64; 2]>>, denominator: u32) {
    let mut sizes: Vec<_> = paths
        .iter()
        .map(|p| {
            let b = extent(p);
            assert!((b[2] - b[0] - (b[3] - b[1])).abs() < 0.025);
            b[2] - b[0]
        })
        .collect();
    sizes.sort_by(f64::total_cmp);
    assert_eq!(sizes.len(), 6);
    let expected = [1000., 1000., 3200., 3200., 4000., 4000.];
    for (actual, expected) in sizes.into_iter().zip(expected) {
        assert!(
            (actual - expected / denominator as f64).abs() < 0.025,
            "{actual} vs {} at 1:{denominator}",
            expected / denominator as f64
        );
    }
}
#[test]
fn native_garden_pdf_and_dxf_measure_actual_paper_spreads_at_both_scales() {
    let garden = garden::build().unwrap();
    let model = garden::compile(&garden).unwrap();
    let session = &garden.session;
    let evaluation = model
        .drawings(session.document(), session.revision(), &Default::default())
        .unwrap();
    for (&page, denominator) in garden.pages.iter().zip([100, 50]) {
        let book = drawings::read(session.document(), garden.book).unwrap();
        let v = &book.pages.iter().find(|p| p.id == page).unwrap().views[0];
        let ViewEvaluation::Ready {
            vectors, labels, ..
        } = &evaluation.views[&v.id]
        else {
            panic!()
        };
        assert_eq!(labels.len(), 4);
        assert_eq!(
            vectors
                .iter()
                .filter(|v| matches!(v.curve, VectorCurve::Ellipse { .. }))
                .count(),
            6
        );
        // Context survives under transparent canopies, and all planting linework is
        // appended after context. Nothing is faked as section hatch or canopy mesh.
        let first = vectors
            .iter()
            .position(|v| matches!(v.curve, VectorCurve::Ellipse { .. }))
            .unwrap();
        assert!(first >= 8);
        assert!(
            vectors[first..]
                .iter()
                .all(|v| matches!(v.curve, VectorCurve::Ellipse { .. }) && v.line_width_mm == 0.18)
        );
        let render = drawings::render::render_page(
            session.document(),
            session.revision(),
            garden.book,
            page,
            &evaluation,
        )
        .unwrap();
        assert!(
            render
                .texts
                .iter()
                .any(|t| t.text == "T1" && t.height_mm == drawings::SOURCE_LABEL_MM)
        );
        let pdf = drawings::pdf::book_pdf(
            session.document(),
            session.revision(),
            &[(garden.book, page)],
            &evaluation,
        )
        .unwrap();
        check_sizes(pdf_paths(&pdf), denominator);
        let dxf = drawings::dxf::render_dxf(&render, 1., 4);
        check_sizes(dxf_paths(&dxf), denominator);
    }
}
