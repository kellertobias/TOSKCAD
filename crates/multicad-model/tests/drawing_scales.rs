//! Displayed view scales against actual PDF geometry. Native views of a small
//! knob, a 200 mm sheet, a 720 mm cabinet and a 4 m room are fitted through
//! the shared fitting contract, exported to PDF, and the PDF content stream is
//! parsed independently: every caption's printed scale must equal the paper
//! extent of that view's vectors divided by the model size.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::{pdf::PT_PER_MM, *},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, Model};
use std::process::Command;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn add(s: &mut Session, name: &str, p: PrimitiveParameters) -> EntityId {
    let request = PrimitiveRequest::new(name, p);
    let r = model::prepare_primitive(s, s.revision(), actor(), &request).unwrap();
    s.commit(r.prepared).unwrap();
    request.body
}
fn boxed(size: [f64; 3]) -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size: size.map(length),
        frame: Default::default(),
    }
}
fn view(name: &str, part: EntityId, u: [f64; 3], v: [f64; 3]) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model {
            entities: vec![part],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u,
            v,
        },
        kind: ViewKind::Projection,
        position_mm: [140., 90.],
        scale: Scale {
            numerator: 1,
            denominator: 1,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
    }
}
/// Fit one view of a page with the same steps as the native Fit command.
fn fit(page: &mut Page, evaluation: &DrawingEvaluation, mode: FitMode) {
    let layout = layout_page(page, 1, 1).unwrap();
    let view = &mut page.views[0];
    let Some(ViewEvaluation::Ready { vectors, .. }) = evaluation.views.get(&view.id) else {
        panic!("view not ready")
    };
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in vectors.iter().flat_map(|v| &v.preview_mm) {
        for i in 0..2 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let placed = fit_view(view, &layout, min, max, mode).unwrap();
    for i in 0..3 {
        view.frame.origin_mm[i] +=
            view.frame.u[i] * placed.center_mm[0] + view.frame.v[i] * placed.center_mm[1];
    }
    view.scale = placed.scale;
    view.position_mm = placed.position_mm;
}

/// Open undashed strokes (view vectors) and text strings, per PDF page, in mm.
struct PdfPage {
    strokes: Vec<Vec<[f64; 2]>>,
    texts: Vec<(String, [f64; 2])>,
    height_mm: f64,
}
fn parse_pdf(bytes: &[u8]) -> Vec<PdfPage> {
    let text = String::from_utf8_lossy(bytes);
    let mut heights = text.match_indices("/MediaBox [0 0 ").map(|(i, _)| {
        let rest = &text[i + 15..];
        let end = rest.find(']').unwrap();
        let v: Vec<f64> = rest[..end]
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        v[1] / PT_PER_MM
    });
    let mut pages = vec![];
    for (i, _) in text.match_indices("stream\n") {
        if text[..i].ends_with("end") {
            continue;
        }
        let body = &text[i + 7..];
        let body = &body[..body.find("endstream").unwrap()];
        // Content streams precede the shared logo image object.
        let Some(height_mm) = heights.next() else {
            break;
        };
        let mut page = PdfPage {
            strokes: vec![],
            texts: vec![],
            height_mm,
        };
        for line in body.lines() {
            if line.starts_with("BT ") {
                let tm: Vec<&str> = line.split_whitespace().collect();
                let at = tm.iter().position(|t| *t == "Tm").unwrap();
                let x: f64 = tm[at - 2].parse().unwrap();
                let y: f64 = tm[at - 1].parse().unwrap();
                let s = &line[line.find('(').unwrap() + 1..line.rfind(')').unwrap()];
                page.texts
                    .push((s.to_owned(), [x / PT_PER_MM, height_mm - y / PT_PER_MM]));
            } else if line.ends_with(" S") && line.contains("[] 0 d") {
                let tokens: Vec<&str> = line.split_whitespace().collect();
                let mut points = vec![];
                for (k, t) in tokens.iter().enumerate() {
                    if *t == "m" || *t == "l" {
                        let x: f64 = tokens[k - 2].parse().unwrap();
                        let y: f64 = tokens[k - 1].parse().unwrap();
                        points.push([x / PT_PER_MM, height_mm - y / PT_PER_MM]);
                    }
                }
                page.strokes.push(points);
            }
        }
        pages.push(page);
    }
    pages
}
fn extent(strokes: &[Vec<[f64; 2]>]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in strokes.iter().flatten() {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}
fn printed_scale(text: &str) -> f64 {
    let ratio = text.rsplit("Scale ").next().unwrap();
    let (n, d) = ratio.split_once(':').unwrap();
    n.trim().parse::<f64>().unwrap() / d.trim().parse::<f64>().unwrap()
}

struct Case {
    name: &'static str,
    model_mm: [f64; 2],
    mode: FitMode,
    expected: Scale,
}

#[test]
fn fitted_view_captions_and_title_block_agree_with_pdf_geometry() {
    let mut s = Session::new(Document::new("Scale labels").unwrap()).unwrap();
    let knob = add(
        &mut s,
        "Knob",
        PrimitiveParameters::Cylinder {
            radius: length(8.),
            height: length(15.),
            frame: PrimitiveFrame::default(),
        },
    );
    let sheet = add(&mut s, "Sheet", boxed([200., 150., 2.]));
    let cabinet = add(&mut s, "Cabinet", boxed([720., 400., 560.]));
    let room = add(&mut s, "Room slab", boxed([4000., 3000., 200.]));
    let top = ([1., 0., 0.], [0., 1., 0.]);
    let front = ([1., 0., 0.], [0., 0., 1.]);
    let cases = [
        (knob, top, "Knob", [16., 16.], FitMode::Standard),
        (sheet, top, "Sheet", [200., 150.], FitMode::Standard),
        (cabinet, front, "Cabinet", [720., 560.], FitMode::Standard),
        (room, top, "Room", [4000., 3000.], FitMode::Exact),
        (knob, top, "Knob 1:1", [16., 16.], FitMode::KeepScale),
    ];
    let mut pages: Vec<Page> = cases
        .iter()
        .enumerate()
        .map(|(i, (part, (u, v), name, _, _))| {
            let mut page = Page::new(format!("S-{}", i + 1));
            page.info.title = format!("{name} drawing");
            page.views.push(view(name, *part, *u, *v));
            page
        })
        .collect();
    let (book, p) =
        prepare_create(&s, s.revision(), actor(), "Sheets", Some(pages[0].clone())).unwrap();
    s.commit(p).unwrap();
    for page in &pages[1..] {
        s.commit(
            prepare(
                &s,
                s.revision(),
                actor(),
                book,
                PageAction::Add(page.clone()),
            )
            .unwrap(),
        )
        .unwrap();
    }
    let evaluation = compile(&s)
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    for (page, case) in pages.iter_mut().zip(&cases) {
        fit(page, &evaluation, case.4);
        s.commit(
            prepare(
                &s,
                s.revision(),
                actor(),
                book,
                PageAction::Replace(page.clone()),
            )
            .unwrap(),
        )
        .unwrap();
    }
    let expected = [
        Case {
            name: "Knob",
            model_mm: cases[0].3,
            mode: FitMode::Standard,
            // A 16 mm knob is enlarged to a readable standard detail scale.
            expected: Scale {
                numerator: 5,
                denominator: 1,
            },
        },
        Case {
            name: "Sheet",
            model_mm: cases[1].3,
            mode: FitMode::Standard,
            expected: Scale {
                numerator: 1,
                denominator: 2,
            },
        },
        Case {
            name: "Cabinet",
            model_mm: cases[2].3,
            mode: FitMode::Standard,
            expected: Scale {
                numerator: 1,
                denominator: 5,
            },
        },
        Case {
            name: "Room",
            model_mm: cases[3].3,
            mode: FitMode::Exact,
            expected: pages[3].views[0].scale,
        },
        Case {
            name: "Knob 1:1",
            model_mm: cases[4].3,
            mode: FitMode::KeepScale,
            expected: Scale {
                numerator: 1,
                denominator: 1,
            },
        },
    ];
    for (page, case) in pages.iter().zip(&expected) {
        assert_eq!(
            page.views[0].scale, case.expected,
            "{} ({:?})",
            case.name, case.mode
        );
    }
    // The exact room ratio is a reduction no standard series would pick.
    assert!(pages[3].views[0].scale.denominator > 10);

    let model = compile(&s);
    let evaluation = model
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    let renders: Vec<_> = pages
        .iter()
        .map(|p| render::render_page(s.document(), s.revision(), book, p.id, &evaluation).unwrap())
        .collect();
    let bytes = pdf::render_pdf(&renders, "Scale labels");
    // Optional visual evidence: MULTICAD_DRAWING_REVIEW=<dir> keeps the PDF.
    if let Some(dir) = std::env::var_os("MULTICAD_DRAWING_REVIEW") {
        std::fs::write(std::path::Path::new(&dir).join("scales.pdf"), &bytes).unwrap();
    }
    let parsed = parse_pdf(&bytes);
    logo_checks(&bytes, pages.len(), true);
    assert_eq!(parsed.len(), pages.len());
    for ((pdf, page), case) in parsed.iter().zip(&pages).zip(&expected) {
        let caption = pdf
            .texts
            .iter()
            .find(|(t, _)| t.starts_with(&format!("{} ", case.name)) && t.contains("Scale "))
            .unwrap_or_else(|| panic!("{} has no caption: {:?}", case.name, pdf.texts));
        let shown = printed_scale(&caption.0);
        let b = extent(&pdf.strokes);
        let measured = [
            (b[2] - b[0]) / case.model_mm[0],
            (b[3] - b[1]) / case.model_mm[1],
        ];
        // Paper chord sampling (0.01 mm) and PDF point rounding only.
        for (m, size) in measured.iter().zip(case.model_mm) {
            assert!(
                ((m - shown) * size).abs() <= 0.03,
                "{}: caption {} but PDF geometry is {m}",
                case.name,
                caption.0
            );
        }
        // The caption sits on paper, next to (below or above) its view.
        assert!(caption.1[1] > 10. && caption.1[1] < pdf.height_mm - 10.);
        assert!(
            caption.1[1] >= b[3] || caption.1[1] <= b[1],
            "{}: caption overlaps view",
            case.name
        );
        // The title block's single scale matches the only view on the page.
        let title_scale = scale_text(page.views[0].scale);
        let tokens = |t: &str| t.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert!(
            pdf.texts
                .iter()
                .any(|(t, _)| *t != caption.0 && tokens(t).contains(&title_scale)),
            "{}: title block lacks {title_scale}: {:?}",
            case.name,
            pdf.texts
        );
        let stale_one = page.views[0].scale
            != Scale {
                numerator: 1,
                denominator: 1,
            };
        if stale_one {
            assert!(
                !pdf.texts
                    .iter()
                    .any(|(t, _)| tokens(t).iter().any(|w| w == "1:1")),
                "{}: misleading 1:1 remains",
                case.name
            );
        }
    }

    // The same sheets with the app logo switched off: no image, no hole.
    let mut plain = vec![];
    for page in &pages {
        let mut p = page.clone();
        p.settings.app_logo = false;
        s.commit(prepare(&s, s.revision(), actor(), book, PageAction::Replace(p)).unwrap())
            .unwrap();
    }
    let evaluation = compile(&s)
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    for page in &pages {
        plain.push(
            render::render_page(s.document(), s.revision(), book, page.id, &evaluation).unwrap(),
        );
    }
    let without = pdf::render_pdf(&plain, "Scale labels");
    logo_checks(&without, pages.len(), false);
    if let Some(dir) = std::env::var_os("MULTICAD_DRAWING_REVIEW") {
        std::fs::write(
            std::path::Path::new(&dir).join("scales-no-logo.pdf"),
            &without,
        )
        .unwrap();
    }

    // Mixed scales on one sheet: each view keeps its own caption and the
    // title block no longer claims a single scale.
    let mut mixed = pages[1].clone();
    let mut detail = mixed.views[0].clone();
    detail.id = EntityId::new();
    detail.name = "Detail".into();
    detail.scale = Scale {
        numerator: 1,
        denominator: 4,
    };
    detail.position_mm = [60., 60.];
    mixed.views[0].scale = Scale {
        numerator: 1,
        denominator: 2,
    };
    mixed.views.push(detail);
    s.commit(
        prepare(
            &s,
            s.revision(),
            actor(),
            book,
            PageAction::Replace(mixed.clone()),
        )
        .unwrap(),
    )
    .unwrap();
    let evaluation = compile(&s)
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    let render =
        render::render_page(s.document(), s.revision(), book, mixed.id, &evaluation).unwrap();
    let texts: Vec<_> = render.texts.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"Sheet — Scale 1:2"), "{texts:?}");
    assert!(texts.contains(&"Detail — Scale 1:4"), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains(MIXED_SCALE)), "{texts:?}");
    assert!(
        !texts
            .iter()
            .any(|t| t.split_whitespace().any(|w| w == "1:1")),
        "{texts:?}"
    );
    let path = tempfile::tempdir().unwrap();
    let file = path.path().join("scales.pdf");
    std::fs::write(&file, pdf::render_pdf(&[render], "Mixed")).unwrap();
    if let Ok(out) = Command::new("pdftotext").arg(&file).arg("-").output()
        && out.status.success()
    {
        let words = String::from_utf8_lossy(&out.stdout);
        assert!(
            words.contains("Scale 1:2") && words.contains("Scale 1:4"),
            "{words}"
        );
        assert!(words.contains(MIXED_SCALE), "{words}");
    }
}

/// The logo is one shared RGB image drawn once per page when enabled, and
/// independent consumers decode it; disabled pages carry no image at all.
fn logo_checks(bytes: &[u8], pages: usize, shown: bool) {
    let text = String::from_utf8_lossy(bytes);
    let draws = text.matches("/Logo Do").count();
    let images = text.matches("/Subtype /Image").count();
    if !shown {
        assert_eq!((draws, images), (0, 0));
        assert!(!text.contains("/XObject"));
        return;
    }
    assert_eq!(draws, pages);
    assert_eq!(images, 1, "one shared image object");
    let [w, h] = logo_size();
    assert!(text.contains(&format!("/Width {w} /Height {h} /ColorSpace /DeviceRGB")));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("logo.pdf");
    std::fs::write(&file, bytes).unwrap();
    if let Ok(out) = Command::new("pdfimages").arg("-list").arg(&file).output()
        && out.status.success()
    {
        let list = String::from_utf8_lossy(&out.stdout);
        let rows: Vec<_> = list.lines().skip(2).collect();
        assert_eq!(rows.len(), pages, "{list}");
        for row in rows {
            let cols: Vec<_> = row.split_whitespace().collect();
            assert_eq!(
                (cols[3], cols[4], cols[5]),
                (&*w.to_string(), &*h.to_string(), "rgb"),
                "{row}"
            );
            // Printed at more than 300 dpi in both directions.
            assert!(cols[12].parse::<f64>().unwrap() >= 300., "{row}");
            assert!(cols[13].parse::<f64>().unwrap() >= 300., "{row}");
        }
    }
    // Rasterized, the logo area is the blueprint blue, not paper white.
    let png = dir.path().join("page");
    if Command::new("pdftoppm")
        .args(["-r", "50", "-f", "1", "-l", "1"])
        .arg(&file)
        .arg(&png)
        .status()
        .is_ok_and(|s| s.success())
    {
        let path = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|e| e == "ppm"))
            .unwrap();
        // Binary PPM: "P6\n<w> <h>\n255\n" then RGB bytes.
        let ppm = std::fs::read(path).unwrap();
        let mut fields = vec![];
        let mut start = 0;
        for (i, b) in ppm.iter().enumerate() {
            if b.is_ascii_whitespace() {
                if i > start {
                    fields.push(String::from_utf8_lossy(&ppm[start..i]).into_owned());
                }
                start = i + 1;
                if fields.len() == 4 {
                    break;
                }
            }
        }
        assert_eq!(fields[0], "P6");
        let width: usize = fields[1].parse().unwrap();
        let buf = &ppm[start..];
        let page = Page::new("x");
        let layout = layout_page(&page, 1, 1).unwrap();
        let logo = layout.logo_mm.unwrap();
        let px = 50. / 25.4;
        let at = |x: f64, y: f64| {
            let i = ((y * px) as usize * width + (x * px) as usize) * 3;
            [buf[i], buf[i + 1], buf[i + 2]]
        };
        let [r, g, b] = at(logo[0] + 1., (logo[1] + logo[3]) * 0.5);
        assert!(b > r + 40 && b > 150, "logo pixel {r},{g},{b}");
    }
}
