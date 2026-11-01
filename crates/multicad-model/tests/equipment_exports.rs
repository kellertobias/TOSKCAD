#[allow(dead_code)]
#[path = "../../multicad-app/examples/equipment_review.rs"]
mod apartment;
use multicad_core::{
    drawings::{self, ViewEvaluation},
    equipment::Kind,
};
fn pdf_points(bytes: &[u8]) -> Vec<[f64; 2]> {
    let text = String::from_utf8_lossy(bytes);
    let begin = text.find("stream\n").unwrap() + 7;
    let stream = &text[begin..];
    let stream = &stream[..stream.find("endstream").unwrap()];
    let mut points = vec![];
    for line in stream.lines().filter(|l| l.ends_with(" S")) {
        let tokens: Vec<_> = line.split_whitespace().collect();
        for (i, t) in tokens.iter().enumerate() {
            if *t == "m" || *t == "l" {
                points.push([
                    tokens[i - 2].parse::<f64>().unwrap() / drawings::pdf::PT_PER_MM,
                    tokens[i - 1].parse::<f64>().unwrap() / drawings::pdf::PT_PER_MM,
                ]);
            }
        }
    }
    points
}
fn bounds(points: impl Iterator<Item = [f64; 2]>) -> [f64; 4] {
    points.fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}
fn check_mark(points: &[[f64; 2]], denominator: f64) {
    let center = [
        200. + (1000. - 6000.) / denominator,
        167. + (145. - 4000.) / denominator + 2.25,
    ];
    let b = bounds(
        points
            .iter()
            .copied()
            .filter(|p| (p[0] - center[0]).hypot(p[1] - center[1]) < 1.80),
    );
    assert!(
        (b[2] - b[0] - 3.5).abs() < 0.025,
        "TypeF mark width at1:{denominator}: {b:?}"
    );
    assert!(
        (b[3] - b[1] - 3.5).abs() < 0.025,
        "TypeF mark height at1:{denominator}: {b:?}"
    );
}
#[test]
fn all_families_export_scaled_footprints_fixed_paper_marks_and_category_visibility() {
    let apartment = apartment::build().unwrap();
    let model = apartment::compile(&apartment).unwrap();
    let s = &apartment.session;
    let evaluation = model
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    let book = drawings::read(s.document(), apartment.book).unwrap();
    for (index, denominator) in [100., 50., 50.].into_iter().enumerate() {
        let page = &book.pages[index];
        let view = &page.views[0];
        let ViewEvaluation::Ready {
            vectors, labels, ..
        } = &evaluation.views[&view.id]
        else {
            panic!("not ready")
        };
        assert_eq!(labels.len(), if index == 2 { 9 } else { 21 });
        let tub = apartment
            .equipment
            .iter()
            .find(|r| r.definition.name == Kind::Bathtub.preset().name)
            .unwrap();
        assert_eq!(
            vectors.iter().any(|v| v.source == tub.placement.instance),
            index != 2
        );
        let render = drawings::render::render_page(
            s.document(),
            s.revision(),
            apartment.book,
            page.id,
            &evaluation,
        )
        .unwrap();
        assert!(
            render
                .texts
                .iter()
                .any(|t| t.text == "P1" && (t.height_mm - 3.5).abs() < 1e-9)
        );
        let pdf = drawings::pdf::book_pdf(
            s.document(),
            s.revision(),
            &[(apartment.book, page.id)],
            &evaluation,
        )
        .unwrap();
        check_mark(&pdf_points(&pdf), denominator);
        let text = drawings::dxf::render_dxf(&render, 1., 4);
        let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(text.as_bytes())).unwrap();
        let mut points = vec![];
        let mut lines = vec![];
        for entity in drawing.entities() {
            if entity.common.layer != "VIEW_VISIBLE" {
                continue;
            }
            if let dxf::entities::EntityType::Line(line) = &entity.specific {
                let a = [line.p1.x, line.p1.y];
                let b = [line.p2.x, line.p2.y];
                points.extend([a, b]);
                lines.push((a, b));
            }
        }
        check_mark(&points, denominator);
        if index != 2 {
            // Independent DXF measurements at the known bathtub footprint.
            let y = 167. + (6500. - 4000. - 375.) / denominator;
            assert!(lines.iter().any(|(a, b)| (a[1] - y).abs() < 1e-5
                && (b[1] - y).abs() < 1e-5
                && ((a[0] - b[0]).abs() - 1700. / denominator).abs() < 1e-5));
        }
    }
}
