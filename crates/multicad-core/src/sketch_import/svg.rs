//! Static SVG geometry reader. Only element geometry and transforms are read;
//! scripts, styles, text, images, references and DTD entities are never
//! evaluated or fetched. Output is in millimeters, Y up, with the origin at
//! the bottom-left corner of the root viewport.
use super::{
    Artwork, Collector, Diagnostics, Format, ImportError, UnitChoice, fail,
    geometry::{Affine, P, Segment, bezier, ellipse_arc, transform},
};
use quick_xml::{Reader, XmlVersion, events::BytesStart, events::Event};
use std::{
    collections::BTreeMap,
    f64::consts::{FRAC_PI_2, PI, TAU},
};

const MAX_ELEMENTS: usize = 200_000;
const MAX_DEPTH: usize = 256;
const PX_MM: f64 = 25.4 / 96.;

#[derive(Clone)]
struct State {
    transform: Affine,
    layer: String,
}

pub fn parse(bytes: &[u8], units: UnitChoice) -> Result<Artwork, ImportError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| ImportError("SVG must be UTF-8 encoded".into()))?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut out = Collector::default();
    let mut diagnostics = Diagnostics::default();
    let mut notes = Vec::new();
    // `None` entries mark skipped subtrees so End events stay balanced.
    let mut stack: Vec<Option<State>> = Vec::new();
    let mut elements = 0usize;
    let mut groups = 0usize;
    let mut root_seen = false;
    let mut millimeters = PX_MM;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| ImportError(format!("Malformed SVG XML: {e}")))?;
        let (element, empty) = match event {
            Event::Eof => break,
            Event::DocType(d) => {
                let raw = String::from_utf8_lossy(&d);
                if raw.contains("ENTITY") {
                    return fail("SVG DTD entity declarations are not supported");
                }
                continue;
            }
            Event::End(_) => {
                stack.pop();
                continue;
            }
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            _ => continue,
        };
        elements += 1;
        if elements > MAX_ELEMENTS || stack.len() >= MAX_DEPTH {
            return fail("SVG exceeds the element count or nesting budget");
        }
        let attrs = attributes(&element)?;
        let qname = String::from_utf8_lossy(element.name().as_ref()).into_owned();
        let name = match qname.split_once(':') {
            Some(("svg", local)) => local.to_string(),
            Some(_) => {
                // Editor metadata such as sodipodi:namedview is not geometry.
                if !empty {
                    stack.push(None);
                }
                continue;
            }
            None => qname,
        };
        let parent = match stack.last() {
            Some(Some(state)) => Some(state.clone()),
            Some(None) => {
                if !empty {
                    stack.push(None);
                }
                continue;
            }
            None => None,
        };
        let Some(parent) = parent else {
            if name != "svg" || root_seen {
                return fail("SVG root element must be <svg>");
            }
            root_seen = true;
            let (root, mm, note) = root_transform(&attrs, units, &mut notes)?;
            millimeters = mm;
            notes.push(note);
            if !empty {
                stack.push(Some(State {
                    transform: root,
                    layer: "(root)".into(),
                }));
            }
            continue;
        };
        if hidden(&attrs) {
            diagnostics.add("svg.hidden", "Elements with display:none were skipped");
            if !empty {
                stack.push(None);
            }
            continue;
        }
        let local = match attrs.get("transform") {
            Some(t) => parse_transform(t)?,
            None => Affine::IDENTITY,
        };
        let state = State {
            transform: parent.transform.then_after(local),
            layer: parent.layer.clone(),
        };
        if !state.transform.finite() {
            return fail("SVG transform is singular or nonfinite");
        }
        let mut child = None;
        match name.as_str() {
            "g" | "a" | "switch" => {
                let mut state = state;
                if stack.len() == 1 && name == "g" {
                    groups += 1;
                    state.layer = attrs
                        .get("inkscape:label")
                        .or_else(|| attrs.get("id"))
                        .cloned()
                        .unwrap_or_else(|| format!("group {groups}"));
                    out.layer(&state.layer);
                }
                child = Some(state);
            }
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => {
                match shape(&name, &attrs) {
                    Ok(paths) => {
                        for (segments, closed) in paths {
                            let segments: Vec<_> = segments
                                .iter()
                                .map(|s| transform(s, &state.transform))
                                .collect();
                            out.push(&state.layer, segments, closed)?;
                        }
                    }
                    Err(Skip(kind, message)) => diagnostics.add(kind, message),
                }
            }
            "defs" | "symbol" | "linearGradient" | "radialGradient" | "pattern" | "filter"
            | "marker" | "title" | "desc" | "metadata" => {}
            "clipPath" | "mask" => diagnostics.add(
                "svg.clip",
                "Clip paths and masks are not applied; unclipped geometry is imported",
            ),
            "use" => diagnostics.add("svg.use", "<use> references are not expanded"),
            "text" => diagnostics.add(
                "svg.text",
                "Text is not converted to outlines; convert text to paths first",
            ),
            "image" => diagnostics.add("svg.image", "Raster images are not imported or fetched"),
            "script" => diagnostics.add("svg.script", "Scripts are ignored and never executed"),
            "style" => diagnostics.add(
                "svg.style",
                "Stylesheets are ignored; geometry is imported regardless of styling",
            ),
            "foreignObject" => diagnostics.add("svg.foreign", "Foreign objects are not imported"),
            "svg" => diagnostics.add("svg.nested", "Nested <svg> viewports are not imported"),
            other => diagnostics.add(
                &format!("svg.{other}"),
                format!("<{other}> elements are not imported"),
            ),
        }
        if !empty {
            stack.push(child);
        }
    }
    if !root_seen {
        return fail("File has no SVG root element");
    }
    Ok(Artwork {
        format: Format::Svg,
        paths: out.paths,
        layers: out.layers,
        unsupported: diagnostics.finish(),
        notes,
        millimeters_per_unit: millimeters,
    })
}

fn attributes(e: &BytesStart) -> Result<BTreeMap<String, String>, ImportError> {
    let mut map = BTreeMap::new();
    for a in e.attributes() {
        let a = a.map_err(|e| ImportError(format!("Malformed SVG attribute: {e}")))?;
        let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let value = a
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| ImportError(format!("Unsupported SVG attribute value: {e}")))?;
        map.insert(key, value.into_owned());
    }
    Ok(map)
}

fn hidden(attrs: &BTreeMap<String, String>) -> bool {
    attrs.get("display").is_some_and(|d| d.trim() == "none")
        || attrs.get("style").is_some_and(|s| {
            s.split(';').any(|decl| {
                decl.split_once(':')
                    .is_some_and(|(k, v)| k.trim() == "display" && v.trim() == "none")
            })
        })
}

/// Absolute root length in millimeters; `None` for relative or missing values.
fn absolute_length(value: Option<&String>) -> Result<Option<(f64, bool)>, ImportError> {
    let Some(value) = value.map(|v| v.trim()) else {
        return Ok(None);
    };
    let split = value
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let n: f64 = number
        .trim()
        .parse()
        .map_err(|_| ImportError(format!("Invalid SVG root length {value:?}")))?;
    let (mm, physical) = match unit.trim() {
        "" | "px" => (PX_MM, false),
        "mm" => (1., true),
        "cm" => (10., true),
        "in" => (25.4, true),
        "pt" => (25.4 / 72., true),
        "pc" => (25.4 / 6., true),
        "Q" | "q" => (0.25, true),
        _ => return Ok(None),
    };
    if !(n.is_finite() && n > 0. && n < 1e9) {
        return fail("SVG root size must be positive and finite");
    }
    Ok(Some((n * mm, physical)))
}

fn root_transform(
    attrs: &BTreeMap<String, String>,
    units: UnitChoice,
    notes: &mut Vec<String>,
) -> Result<(Affine, f64, String), ImportError> {
    let view_box = match attrs.get("viewBox") {
        Some(v) => {
            let n = numbers(v)?;
            if n.len() != 4 || n[2] <= 0. || n[3] <= 0. {
                return fail("SVG viewBox must contain four numbers with positive size");
            }
            Some([n[0], n[1], n[2], n[3]])
        }
        None => None,
    };
    let width = absolute_length(attrs.get("width"))?;
    let height = absolute_length(attrs.get("height"))?;
    let [minx, miny, vbw, vbh] = view_box.unwrap_or([0., 0., 0., 0.]);
    let (sx, sy, vw, vh, note) = match units {
        UnitChoice::Explicit(unit) => {
            let s = unit.millimeters();
            let h = match view_box {
                Some(_) => vbh * s,
                None => height.map_or(0., |(mm, _)| mm / PX_MM * s),
            };
            let w = match view_box {
                Some(_) => vbw * s,
                None => width.map_or(0., |(mm, _)| mm / PX_MM * s),
            };
            (
                s,
                s,
                w,
                h,
                format!(
                    "One SVG user unit explicitly interpreted as 1 {}",
                    unit.label()
                ),
            )
        }
        UnitChoice::FromFile => match view_box {
            None => {
                let h = height.map_or(0., |(mm, _)| mm);
                let w = width.map_or(0., |(mm, _)| mm);
                (
                    PX_MM,
                    PX_MM,
                    w,
                    h,
                    "No viewBox: one user unit is one CSS pixel (1/96 in = 0.2645833 mm)".into(),
                )
            }
            Some(_) => match (width, height) {
                (Some((w, _)), Some((h, _))) => {
                    let (sx, sy) = (w / vbw, h / vbh);
                    (sx, sy, w, h, format!("viewBox maps to {w:.6} × {h:.6} mm"))
                }
                (Some((w, _)), None) => {
                    let s = w / vbw;
                    (
                        s,
                        s,
                        w,
                        vbh * s,
                        format!("Width {w:.6} mm with viewBox aspect ratio"),
                    )
                }
                (None, Some((h, _))) => {
                    let s = h / vbh;
                    (
                        s,
                        s,
                        vbw * s,
                        h,
                        format!("Height {h:.6} mm with viewBox aspect ratio"),
                    )
                }
                (None, None) => (
                    PX_MM,
                    PX_MM,
                    vbw * PX_MM,
                    vbh * PX_MM,
                    "No absolute size: one viewBox unit is one CSS pixel (1/96 in)".into(),
                ),
            },
        },
    };
    if vh == 0. {
        notes.push("No viewport height: the origin is the SVG user-space origin".into());
    }
    let (mut sx, mut sy) = (sx, sy);
    let (mut ax, mut ay) = (0., 0.);
    if view_box.is_some() && (sx - sy).abs() > 1e-12 * sx.max(sy) {
        let par = attrs
            .get("preserveAspectRatio")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "xMidYMid meet".into());
        let mut tokens = par.split_whitespace();
        let align = tokens.next().unwrap_or("xMidYMid");
        if align != "none" {
            let s = if tokens.next() == Some("slice") {
                sx.max(sy)
            } else {
                sx.min(sy)
            };
            let extra = [vw - vbw * s, vh - vbh * s];
            let factor = |token: &str| match token {
                "Min" => 0.,
                "Max" => 1.,
                _ => 0.5,
            };
            if align.len() == 8 {
                ax = extra[0] * factor(&align[1..4]);
                ay = extra[1] * factor(&align[5..8]);
            }
            sx = s;
            sy = s;
            notes.push(format!("preserveAspectRatio {par} keeps a uniform scale"));
        } else {
            notes.push("preserveAspectRatio none: user space is scaled non-uniformly; circles become exact ellipses".into());
        }
    }
    let root = Affine([sx, 0., 0., -sy, ax - minx * sx, vh + miny * sy - ay]);
    Ok((root, sx.max(sy), note))
}

pub(crate) fn parse_transform(value: &str) -> Result<Affine, ImportError> {
    let mut result = Affine::IDENTITY;
    let mut rest = value.trim();
    while !rest.is_empty() {
        let open = rest
            .find('(')
            .ok_or_else(|| ImportError(format!("Malformed SVG transform {value:?}")))?;
        let close = rest[open..]
            .find(')')
            .map(|c| c + open)
            .ok_or_else(|| ImportError(format!("Malformed SVG transform {value:?}")))?;
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let n = numbers(&rest[open + 1..close])?;
        let m = match (name, n.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => Affine::translate(*x, 0.),
            ("translate", [x, y]) => Affine::translate(*x, *y),
            ("scale", [s]) => Affine::scale(*s, *s),
            ("scale", [x, y]) => Affine::scale(*x, *y),
            ("rotate", [a]) => Affine::rotate(a.to_radians()),
            ("rotate", [a, x, y]) => Affine::translate(*x, *y)
                .then_after(Affine::rotate(a.to_radians()))
                .then_after(Affine::translate(-x, -y)),
            ("skewX", [a]) => Affine([1., 0., a.to_radians().tan(), 1., 0., 0.]),
            ("skewY", [a]) => Affine([1., a.to_radians().tan(), 0., 1., 0., 0.]),
            _ => return fail(format!("Unsupported SVG transform {name}")),
        };
        result = result.then_after(m);
        rest = rest[close + 1..].trim_start_matches([' ', ',', '\t', '\n', '\r']);
    }
    if !result.finite() {
        return fail("SVG transform is singular or nonfinite");
    }
    Ok(result)
}

/// SVG number list tokenizer, including `1.5.5` and `1-2` concatenations.
struct Numbers<'a> {
    s: &'a [u8],
    i: usize,
}
impl<'a> Numbers<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            i: 0,
        }
    }
    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r' | b',')
        {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
    fn number(&mut self) -> Result<f64, ImportError> {
        self.skip();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let mut digits = false;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
            digits = true;
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
                digits = true;
            }
        }
        if digits && matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            } else {
                self.i = save;
            }
        }
        if !digits {
            return fail("Malformed SVG number");
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).expect("ascii");
        let v: f64 = text
            .parse()
            .map_err(|_| ImportError("Malformed SVG number".into()))?;
        if !v.is_finite() || v.abs() > 1e12 {
            return fail("SVG number is nonfinite or out of range");
        }
        Ok(v)
    }
    fn flag(&mut self) -> Result<bool, ImportError> {
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => fail("Malformed SVG arc flag"),
        }
    }
    fn at_number(&mut self) -> bool {
        self.peek()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.'))
    }
}

fn numbers(s: &str) -> Result<Vec<f64>, ImportError> {
    let mut n = Numbers::new(s);
    let mut out = Vec::new();
    while n.peek().is_some() {
        out.push(n.number()?);
    }
    Ok(out)
}

struct Skip(&'static str, &'static str);
type Shape = Vec<(Vec<Segment>, bool)>;

fn length(attrs: &BTreeMap<String, String>, key: &str) -> Result<Option<f64>, Skip> {
    let Some(v) = attrs.get(key) else {
        return Ok(None);
    };
    let v = v.trim();
    let v = v.strip_suffix("px").unwrap_or(v).trim();
    match v.parse::<f64>() {
        Ok(n) if n.is_finite() && n.abs() < 1e12 => Ok(Some(n)),
        _ => Err(Skip(
            "svg.length_units",
            "Shapes with relative or physical attribute units were skipped; use user units",
        )),
    }
}

fn line(a: P, b: P) -> Option<Segment> {
    ((a[0] - b[0]).hypot(a[1] - b[1]) > 1e-12).then_some(Segment::Line { a, b })
}

fn shape(name: &str, attrs: &BTreeMap<String, String>) -> Result<Shape, Skip> {
    let degenerate = Skip("svg.degenerate", "Zero-size shapes were skipped");
    let num = |k: &str| length(attrs, k).map(|v| v.unwrap_or(0.));
    Ok(match name {
        "path" => {
            let d = attrs.get("d").map(String::as_str).unwrap_or("");
            path_data(d)
                .map_err(|_| Skip("svg.path_syntax", "Paths with malformed data were skipped"))?
        }
        "line" => match line([num("x1")?, num("y1")?], [num("x2")?, num("y2")?]) {
            Some(s) => vec![(vec![s], false)],
            None => return Err(degenerate),
        },
        "polyline" | "polygon" => {
            let n = numbers(attrs.get("points").map(String::as_str).unwrap_or(""))
                .map_err(|_| Skip("svg.path_syntax", "Paths with malformed data were skipped"))?;
            let points: Vec<P> = n.as_chunks::<2>().0.to_vec();
            let closed = name == "polygon";
            let count = if closed {
                points.len()
            } else {
                points.len().saturating_sub(1)
            };
            let segments: Vec<_> = (0..count)
                .filter_map(|i| line(points[i], points[(i + 1) % points.len()]))
                .collect();
            if segments.is_empty() {
                return Err(degenerate);
            }
            vec![(segments, closed && points.len() > 2)]
        }
        "circle" => {
            let r = num("r")?;
            if r <= 0. {
                return Err(degenerate);
            }
            vec![(
                vec![Segment::Circle {
                    center: [num("cx")?, num("cy")?],
                    radius: r,
                }],
                true,
            )]
        }
        "ellipse" => {
            let (rx, ry) = radii(length(attrs, "rx")?, length(attrs, "ry")?);
            if rx <= 0. || ry <= 0. {
                return Err(degenerate);
            }
            let c = [num("cx")?, num("cy")?];
            let s = if (rx - ry).abs() <= 1e-12 * rx {
                Segment::Circle {
                    center: c,
                    radius: rx,
                }
            } else {
                ellipse_arc(c, [rx, 0.], [0., ry], 0., TAU)
            };
            vec![(vec![s], true)]
        }
        "rect" => {
            let (x, y, w, h) = (num("x")?, num("y")?, num("width")?, num("height")?);
            if w <= 0. || h <= 0. {
                return Err(degenerate);
            }
            let (rx, ry) = radii(length(attrs, "rx")?, length(attrs, "ry")?);
            let (rx, ry) = (rx.clamp(0., w / 2.), ry.clamp(0., h / 2.));
            vec![(rectangle(x, y, w, h, rx, ry), true)]
        }
        _ => unreachable!(),
    })
}

fn radii(rx: Option<f64>, ry: Option<f64>) -> (f64, f64) {
    match (rx, ry) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, x),
        (None, Some(y)) => (y, y),
        (None, None) => (0., 0.),
    }
}

fn corner(c: P, rx: f64, ry: f64, t0: f64) -> Segment {
    if (rx - ry).abs() <= 1e-12 * rx.max(ry) {
        Segment::Arc {
            center: c,
            radius: rx,
            start: t0,
            sweep: FRAC_PI_2,
        }
    } else {
        ellipse_arc(c, [rx, 0.], [0., ry], t0, FRAC_PI_2)
    }
}

/// SVG user space is Y-down; corners run in increasing user-space angle.
fn rectangle(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> Vec<Segment> {
    if rx <= 0. || ry <= 0. {
        let p = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        return (0..4)
            .map(|i| Segment::Line {
                a: p[i],
                b: p[(i + 1) % 4],
            })
            .collect();
    }
    let mut out = Vec::new();
    let push_line = |out: &mut Vec<Segment>, a: P, b: P| {
        if let Some(s) = line(a, b) {
            out.push(s)
        }
    };
    push_line(&mut out, [x + rx, y], [x + w - rx, y]);
    out.push(corner([x + w - rx, y + ry], rx, ry, -FRAC_PI_2));
    push_line(&mut out, [x + w, y + ry], [x + w, y + h - ry]);
    out.push(corner([x + w - rx, y + h - ry], rx, ry, 0.));
    push_line(&mut out, [x + w - rx, y + h], [x + rx, y + h]);
    out.push(corner([x + rx, y + h - ry], rx, ry, FRAC_PI_2));
    push_line(&mut out, [x, y + h - ry], [x, y + ry]);
    out.push(corner([x + rx, y + ry], rx, ry, PI));
    out
}

/// SVG 1.1 implementation notes F.6.5/F.6.6: endpoint to center
/// parameterization with out-of-range radii scaled up.
#[allow(clippy::too_many_arguments)]
fn arc(p0: P, rx: f64, ry: f64, phi_deg: f64, large: bool, sweep: bool, p1: P) -> Option<Segment> {
    if (p0[0] - p1[0]).hypot(p0[1] - p1[1]) <= 1e-12 {
        return None;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx <= 1e-12 || ry <= 1e-12 {
        return line(p0, p1);
    }
    let phi = phi_deg.to_radians();
    let (s, c) = phi.sin_cos();
    let dx = (p0[0] - p1[0]) / 2.;
    let dy = (p0[1] - p1[1]) / 2.;
    let x1 = c * dx + s * dy;
    let y1 = -s * dx + c * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1. {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (num / den).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let center = [
        c * cx1 - s * cy1 + (p0[0] + p1[0]) / 2.,
        s * cx1 + c * cy1 + (p0[1] + p1[1]) / 2.,
    ];
    let angle = |ux: f64, uy: f64| uy.atan2(ux);
    let t0 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let t1 = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let mut dt = t1 - t0;
    if sweep && dt < 0. {
        dt += TAU;
    } else if !sweep && dt > 0. {
        dt -= TAU;
    }
    if (rx - ry).abs() <= 1e-12 * rx {
        Some(Segment::Arc {
            center,
            radius: rx,
            start: t0 + phi,
            sweep: dt,
        })
    } else {
        Some(ellipse_arc(
            center,
            [rx * c, rx * s],
            [-ry * s, ry * c],
            t0,
            dt,
        ))
    }
}

pub(crate) fn path_data(d: &str) -> Result<Shape, ImportError> {
    let mut n = Numbers::new(d);
    let mut out: Shape = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    let mut pen = [0., 0.];
    let mut start = [0., 0.];
    let mut last_cubic: Option<P> = None;
    let mut last_quad: Option<P> = None;
    let mut command: Option<u8> = None;
    let flush = |out: &mut Shape, current: &mut Vec<Segment>, closed: bool| {
        if !current.is_empty() {
            out.push((std::mem::take(current), closed));
        }
    };
    while let Some(c) = n.peek() {
        let cmd = if c.is_ascii_alphabetic() {
            n.i += 1;
            c
        } else {
            match command {
                Some(b'M') => b'L',
                Some(b'm') => b'l',
                Some(prev) if !matches!(prev, b'Z' | b'z') => prev,
                _ => return fail("SVG path data must start with a command"),
            }
        };
        let rel = cmd.is_ascii_lowercase();
        let off = |p: P, pen: P| {
            if rel {
                [p[0] + pen[0], p[1] + pen[1]]
            } else {
                p
            }
        };
        let mut cubic = None;
        let mut quad = None;
        match cmd.to_ascii_uppercase() {
            b'M' => {
                flush(&mut out, &mut current, false);
                pen = off([n.number()?, n.number()?], pen);
                start = pen;
            }
            b'L' => {
                let p = off([n.number()?, n.number()?], pen);
                current.extend(line(pen, p));
                pen = p;
            }
            b'H' => {
                let x = n.number()?;
                let p = [if rel { pen[0] + x } else { x }, pen[1]];
                current.extend(line(pen, p));
                pen = p;
            }
            b'V' => {
                let y = n.number()?;
                let p = [pen[0], if rel { pen[1] + y } else { y }];
                current.extend(line(pen, p));
                pen = p;
            }
            b'C' | b'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&b'C') {
                    off([n.number()?, n.number()?], pen)
                } else {
                    last_cubic.map_or(pen, |c| [2. * pen[0] - c[0], 2. * pen[1] - c[1]])
                };
                let c2 = off([n.number()?, n.number()?], pen);
                let p = off([n.number()?, n.number()?], pen);
                if (pen[0] - p[0]).hypot(pen[1] - p[1]) > 1e-12
                    || [c1, c2]
                        .iter()
                        .any(|c| (c[0] - p[0]).hypot(c[1] - p[1]) > 1e-12)
                {
                    current.push(bezier(vec![pen, c1, c2, p]));
                }
                cubic = Some(c2);
                pen = p;
            }
            b'Q' | b'T' => {
                let c1 = if cmd.eq_ignore_ascii_case(&b'Q') {
                    off([n.number()?, n.number()?], pen)
                } else {
                    last_quad.map_or(pen, |c| [2. * pen[0] - c[0], 2. * pen[1] - c[1]])
                };
                let p = off([n.number()?, n.number()?], pen);
                if (pen[0] - p[0]).hypot(pen[1] - p[1]) > 1e-12 {
                    current.push(bezier(vec![pen, c1, p]));
                }
                quad = Some(c1);
                pen = p;
            }
            b'A' => {
                let (rx, ry, phi) = (n.number()?, n.number()?, n.number()?);
                let (large, sweep) = (n.flag()?, n.flag()?);
                let p = off([n.number()?, n.number()?], pen);
                current.extend(arc(pen, rx, ry, phi, large, sweep, p));
                pen = p;
            }
            b'Z' => {
                current.extend(line(pen, start));
                pen = start;
                flush(&mut out, &mut current, true);
            }
            _ => return fail("Unknown SVG path command"),
        }
        last_cubic = cubic;
        last_quad = quad;
        command = Some(cmd);
        if !n.at_number() && n.peek().is_some_and(|c| !c.is_ascii_alphabetic()) {
            return fail("Malformed SVG path data");
        }
    }
    flush(&mut out, &mut current, false);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_grammar_handles_concatenated_numbers_flags_and_implicit_commands() {
        let shape = path_data("M0,0 10-0 10.5.5zm20 0h5v5h-5z a5 5 0 1110 0").unwrap();
        assert_eq!(shape.len(), 3);
        assert!(shape[0].1 && shape[1].1 && !shape[2].1);
        assert_eq!(shape[1].0.len(), 4);
        let Segment::Arc { radius, sweep, .. } = &shape[2].0[0] else {
            panic!("{:?}", shape[2])
        };
        assert!((radius - 5.).abs() < 1e-12 && (sweep.abs() - PI).abs() < 1e-9);
    }

    #[test]
    fn transforms_compose_right_to_left() {
        let m = parse_transform("translate(10 0) scale(2)").unwrap();
        assert_eq!(m.apply([1., 1.]), [12., 2.]);
        let r = parse_transform("rotate(90 5 5)").unwrap().apply([10., 5.]);
        assert!((r[0] - 5.).abs() < 1e-12 && (r[1] - 10.).abs() < 1e-12);
    }
}
