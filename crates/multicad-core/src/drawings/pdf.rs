//! Multi-page vector PDF from shared page renders. Each drawing page becomes
//! one PDF page at its paper size and orientation; geometry stays vector and
//! text uses the standard Courier font, matching the monospaced layout.
//! Output is deterministic: no timestamps or random identifiers.
use super::DrawingEvaluation;
use super::render::{PageRender, render_page};
use crate::document::{Document, EntityId};
use std::fmt::Write;

/// PDF points per millimeter.
pub const PT_PER_MM: f64 = 72. / 25.4;
/// Arc segments per full turn when arcs are written as polylines.
const ARC_SEGMENTS: f64 = 96.;

fn n(v: f64) -> String {
    let r = (v * 1000.).round() / 1000.;
    if r == 0. { "0".into() } else { format!("{r}") }
}
/// WinAnsi bytes for a PDF literal string, escaping delimiters. Characters
/// outside WinAnsi become '?'.
fn literal(text: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in text.chars() {
        let byte = match c {
            '(' | ')' | '\\' => {
                out.push(b'\\');
                c as u8
            }
            ' '..='~' => c as u8,
            '°' => 0xB0,
            'Ø' => 0xD8,
            'ø' => 0xF8,
            '±' => 0xB1,
            '·' => 0xB7,
            '×' => 0xD7,
            '—' => 0x97,
            '–' => 0x96,
            'µ' => 0xB5,
            '²' => 0xB2,
            '³' => 0xB3,
            c if (0xA0..=0xFF).contains(&(c as u32)) => c as u32 as u8,
            _ => b'?',
        };
        out.push(byte);
    }
    out.push(b')');
    out
}

fn content(page: &PageRender) -> Vec<u8> {
    let h = page.size_mm[1];
    let p = |q: [f64; 2]| format!("{} {}", n(q[0] * PT_PER_MM), n((h - q[1]) * PT_PER_MM));
    let mut out = Vec::new();
    let mut line = |s: String| {
        out.extend(s.as_bytes());
        out.push(b'\n');
    };
    line("1 J 1 j".into());
    for poly in &page.polylines {
        if poly.points.len() < 2 {
            continue;
        }
        let dash = if poly.dashed {
            format!("[{} {}] 0 d", n(3. * PT_PER_MM), n(1.5 * PT_PER_MM))
        } else {
            "[] 0 d".into()
        };
        let mut path = format!(
            "{} w {dash} {} m",
            n(poly.width_mm * PT_PER_MM),
            p(poly.points[0])
        );
        for q in &poly.points[1..] {
            let _ = write!(path, " {} l", p(*q));
        }
        path.push_str(if poly.closed { " s" } else { " S" });
        line(path);
    }
    for arc in &page.arcs {
        let sweep = arc.end_rad - arc.start_rad;
        let steps = ((sweep.abs() / std::f64::consts::TAU) * ARC_SEGMENTS)
            .ceil()
            .max(4.) as usize;
        let at = |t: f64| {
            let a = arc.start_rad + sweep * t;
            [
                arc.center[0] + arc.radius_mm * a.cos(),
                arc.center[1] + arc.radius_mm * a.sin(),
            ]
        };
        let mut path = format!("{} w [] 0 d {} m", n(arc.width_mm * PT_PER_MM), p(at(0.)));
        for i in 1..=steps {
            let _ = write!(path, " {} l", p(at(i as f64 / steps as f64)));
        }
        path.push_str(" S");
        line(path);
    }
    for fill in &page.fills {
        let mut path = format!("{} m", p(fill.points[0]));
        for q in &fill.points[1..] {
            let _ = write!(path, " {} l", p(*q));
        }
        path.push_str(" h f");
        line(path);
    }
    for r in &page.logos {
        line(format!(
            "q {} 0 0 {} {} cm /Logo Do Q",
            n((r[2] - r[0]) * PT_PER_MM),
            n((r[3] - r[1]) * PT_PER_MM),
            p([r[0], r[3]])
        ));
    }
    for text in &page.texts {
        // Clockwise on paper is clockwise on the page: negate for y up.
        let (s, c) = (-text.angle_deg).to_radians().sin_cos();
        let mut op = format!(
            "BT /F1 {} Tf {} {} {} {} {} Tm ",
            n(text.height_mm * PT_PER_MM),
            n(c),
            n(s),
            n(-s),
            n(c),
            p(text.at)
        )
        .into_bytes();
        op.extend(literal(&text.text));
        op.extend(b" Tj ET");
        out.extend(op);
        out.push(b'\n');
    }
    out
}

/// The bundled logo as an RGB image XObject. The PNG's zlib data is reused
/// unchanged with PNG predictors, so no pixels are resampled or recompressed.
fn logo_image() -> Vec<u8> {
    let [w, h] = super::logo_size();
    let data = super::logo_idat();
    let mut object = format!(
        "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 3 /BitsPerComponent 8 /Columns {w} >> /Length {} >>\nstream\n",
        data.len()
    )
    .into_bytes();
    object.extend(data);
    object.extend(b"\nendstream");
    object
}

/// A PDF document from page renders, in the given order.
pub fn render_pdf(pages: &[PageRender], title: &str) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![];
    // 1 catalog, 2 page tree, 3 font, 4 info, then a page and content per page.
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", 5 + 2 * i))
        .collect();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec(),
    );
    let mut info = b"<< /Producer (MultiCAD) /Title ".to_vec();
    info.extend(literal(title));
    info.extend(b" >>");
    objects.push(info);
    // One shared logo image after the pages, when any page shows it.
    let logo = 5 + 2 * pages.len();
    for (i, page) in pages.iter().enumerate() {
        let [w, h] = page.size_mm.map(|v| v * PT_PER_MM);
        let xobject = if page.logos.is_empty() {
            String::new()
        } else {
            format!(" /XObject << /Logo {logo} 0 R >>")
        };
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R >>{xobject} >> /Contents {} 0 R >>",
                n(w),
                n(h),
                6 + 2 * i
            )
            .into_bytes(),
        );
        let stream = content(page);
        let mut object = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        object.extend(stream);
        object.extend(b"endstream");
        objects.push(object);
    }
    if pages.iter().any(|p| !p.logos.is_empty()) {
        objects.push(logo_image());
    }
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(object);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Render the given pages of one drawing book to PDF. Every page must have
/// a current, checked evaluation.
pub fn book_pdf(
    document: &Document,
    revision: u64,
    pages: &[(EntityId, EntityId)],
    evaluation: &DrawingEvaluation,
) -> Result<Vec<u8>, String> {
    let renders = pages
        .iter()
        .map(|(book, page)| render_page(document, revision, *book, *page, evaluation))
        .collect::<Result<Vec<_>, _>>()?;
    if renders.is_empty() {
        return Err("Choose at least one drawing page".into());
    }
    Ok(render_pdf(&renders, document.name()))
}
