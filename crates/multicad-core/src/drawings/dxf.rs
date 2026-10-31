//! Drawing DXF from a shared page render: the whole sheet at paper scale,
//! y up, in the recipe's unit. Views keep their independent scales because
//! their vectors are already placed on paper. This is distinct from 1:1
//! part-contour DXF. ASCII DXF R12 with one layer per role.
use super::render::{Layer, PageRender};
use std::fmt::Write;

/// DXF layer, linetype and AutoCAD color index for each role.
pub fn layer(layer: Layer) -> (&'static str, &'static str, u8) {
    match layer {
        Layer::Border => ("SHEET_BORDER", "CONTINUOUS", 7),
        Layer::Title => ("SHEET_TITLE", "CONTINUOUS", 7),
        Layer::Visible => ("VIEW_VISIBLE", "CONTINUOUS", 7),
        Layer::Hidden => ("VIEW_HIDDEN", "DASHED", 8),
        Layer::Section => ("VIEW_SECTION", "CONTINUOUS", 1),
        Layer::Hatch => ("VIEW_HATCH", "CONTINUOUS", 9),
        Layer::Dimension => ("DIMENSIONS", "CONTINUOUS", 3),
        Layer::Note => ("NOTES", "CONTINUOUS", 2),
        Layer::Leader => ("LEADERS", "CONTINUOUS", 2),
        Layer::Weld => ("WELD_SYMBOLS", "CONTINUOUS", 4),
        Layer::Table => ("TABLES", "CONTINUOUS", 5),
        Layer::Draft => ("SHEET_DRAFTING", "CONTINUOUS", 6),
        Layer::ViewLabel => ("VIEW_LABELS", "CONTINUOUS", 7),
    }
}
const ALL: [Layer; 13] = [
    Layer::Border,
    Layer::Title,
    Layer::Visible,
    Layer::Hidden,
    Layer::Section,
    Layer::Hatch,
    Layer::Dimension,
    Layer::Note,
    Layer::Leader,
    Layer::Weld,
    Layer::Table,
    Layer::Draft,
    Layer::ViewLabel,
];

/// DXF text with control codes for degree, diameter and plus-minus; other
/// characters outside ASCII become '?'.
fn text(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '°' => "%%d".to_string(),
            'Ø' | 'ø' => "%%c".to_string(),
            '±' => "%%p".to_string(),
            '·' => ".".to_string(),
            '—' | '–' => "-".to_string(),
            c if c.is_ascii() && !c.is_control() => c.to_string(),
            _ => "?".to_string(),
        })
        .collect()
}
fn n(v: f64) -> String {
    let r = (v * 1e6).round() / 1e6;
    if r == 0. { "0".into() } else { format!("{r}") }
}

/// One page as DXF. `scale` converts millimeters to the file unit and
/// `insunits` is that unit's `$INSUNITS` code.
pub fn render_dxf(page: &PageRender, scale: f64, insunits: u16) -> String {
    let h = page.size_mm[1];
    let p = |q: [f64; 2]| [q[0] * scale, (h - q[1]) * scale];
    let mut out = String::new();
    let mut pair = |code: i32, value: &str| {
        let _ = writeln!(out, "{code}\n{value}");
    };
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    pair(9, "$INSUNITS");
    pair(70, &insunits.to_string());
    pair(9, "$EXTMIN");
    pair(10, "0");
    pair(20, "0");
    pair(9, "$EXTMAX");
    pair(10, &n(page.size_mm[0] * scale));
    pair(20, &n(h * scale));
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "TABLES");
    pair(0, "TABLE");
    pair(2, "LTYPE");
    pair(70, "2");
    for (name, pattern) in [
        ("CONTINUOUS", vec![]),
        ("DASHED", vec![3. * scale, -1.5 * scale]),
    ] {
        pair(0, "LTYPE");
        pair(2, name);
        pair(70, "0");
        pair(
            3,
            if pattern.is_empty() {
                "Solid line"
            } else {
                "Dashed"
            },
        );
        pair(72, "65");
        pair(73, &pattern.len().to_string());
        pair(40, &n(pattern.iter().map(|v: &f64| v.abs()).sum()));
        for v in pattern {
            pair(49, &n(v));
        }
    }
    pair(0, "ENDTAB");
    pair(0, "TABLE");
    pair(2, "LAYER");
    pair(70, &ALL.len().to_string());
    for role in ALL {
        let (name, linetype, color) = layer(role);
        pair(0, "LAYER");
        pair(2, name);
        pair(70, "0");
        pair(62, &color.to_string());
        pair(6, linetype);
    }
    pair(0, "ENDTAB");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for poly in &page.polylines {
        let (name, _, _) = layer(poly.layer);
        let mut points: Vec<_> = poly.points.iter().map(|q| p(*q)).collect();
        if poly.closed && points.len() > 2 {
            points.push(points[0]);
        }
        for w in points.windows(2) {
            pair(0, "LINE");
            pair(8, name);
            pair(10, &n(w[0][0]));
            pair(20, &n(w[0][1]));
            pair(11, &n(w[1][0]));
            pair(21, &n(w[1][1]));
        }
    }
    for arc in &page.arcs {
        let (name, _, _) = layer(arc.layer);
        let c = p(arc.center);
        let r = arc.radius_mm * scale;
        if (arc.end_rad - arc.start_rad).abs() >= std::f64::consts::TAU - 1e-9 {
            pair(0, "CIRCLE");
            pair(8, name);
            pair(10, &n(c[0]));
            pair(20, &n(c[1]));
            pair(40, &n(r));
        } else {
            // Paper y down: angles negate, and direction reverses.
            let (a, b) = (-arc.start_rad.to_degrees(), -arc.end_rad.to_degrees());
            let (start, end) = if arc.end_rad > arc.start_rad {
                (b, a)
            } else {
                (a, b)
            };
            pair(0, "ARC");
            pair(8, name);
            pair(10, &n(c[0]));
            pair(20, &n(c[1]));
            pair(40, &n(r));
            pair(50, &n(start.rem_euclid(360.)));
            pair(51, &n(end.rem_euclid(360.)));
        }
    }
    for fill in &page.fills {
        let (name, _, _) = layer(fill.layer);
        let q: Vec<_> = fill.points.iter().map(|x| p(*x)).collect();
        pair(0, "SOLID");
        pair(8, name);
        for (i, point) in [q[0], q[1], q[2], q[2]].iter().enumerate() {
            pair(10 + i as i32, &n(point[0]));
            pair(20 + i as i32, &n(point[1]));
        }
    }
    for t in &page.texts {
        let (name, _, _) = layer(t.layer);
        let at = p(t.at);
        pair(0, "TEXT");
        pair(8, name);
        pair(10, &n(at[0]));
        pair(20, &n(at[1]));
        pair(40, &n(t.height_mm * scale));
        pair(1, &text(&t.text));
        if t.angle_deg.abs() > 1e-9 {
            pair(50, &n(-t.angle_deg));
        }
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    out
}
