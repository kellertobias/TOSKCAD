//! Small audited SVG subset for the reviewed workbench artwork in
//! `assets/icons`. It parses the monochrome icon contract (64 x 64 view box,
//! `currentColor` strokes and fills, rounded caps and joins) and rasterizes it
//! into an alpha mask that the ribbon tints for enabled, disabled, hover and
//! selected states. Unsupported elements or path commands are rejected instead
//! of being drawn incorrectly.

/// Parsed artwork: a sequence of painted elements in view-box units.
#[derive(Clone, Debug)]
pub struct SvgIcon {
    pub view_box: [f32; 4],
    pub title: String,
    /// Root `color` attribute when it is not black (the review set's finish
    /// check carries its own success green).
    pub color: Option<[u8; 3]>,
    elements: Vec<Element>,
}

#[derive(Clone, Debug)]
struct Element {
    /// Flattened subpaths in view-box units: points and a closed flag.
    subpaths: Vec<(Vec<[f32; 2]>, bool)>,
    fill: f32,
    stroke: Option<f32>,
    stroke_opacity: f32,
    dashes: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine([f32; 6]);
impl Affine {
    const IDENTITY: Self = Self([1., 0., 0., 1., 0., 0.]);
    fn apply(self, p: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }
    fn then(self, inner: Self) -> Self {
        // self ∘ inner
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = inner.0;
        Self([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }
    fn scale_factor(self) -> f32 {
        let [a, b, c, d, ..] = self.0;
        ((a * d - b * c).abs()).sqrt()
    }
}

#[derive(Clone, Debug)]
struct Style {
    transform: Affine,
    fill: Option<f32>,
    fill_opacity: f32,
    stroke: bool,
    stroke_width: f32,
    stroke_opacity: f32,
    dashes: Vec<f32>,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            transform: Affine::IDENTITY,
            // SVG's initial fill is black; icon roots override it with "none".
            fill: Some(1.),
            fill_opacity: 1.,
            stroke: false,
            stroke_width: 1.,
            stroke_opacity: 1.,
            dashes: vec![],
        }
    }
}

const MAX_SOURCE: usize = 64 * 1024;
const MAX_POINTS: usize = 20_000;

pub fn parse(source: &str) -> Result<SvgIcon, String> {
    if source.len() > MAX_SOURCE {
        return Err("SVG icon source is too large".into());
    }
    let mut icon = SvgIcon {
        view_box: [0., 0., 0., 0.],
        title: String::new(),
        color: None,
        elements: vec![],
    };
    let mut stack: Vec<(String, Style)> = vec![];
    let mut style = Style::default();
    let mut seen_root = false;
    let mut points = 0usize;
    let mut rest = source;
    while let Some(start) = rest.find('<') {
        let text = &rest[..start];
        if stack.last().is_some_and(|(name, _)| name == "title") {
            icon.title.push_str(text.trim());
        }
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->").ok_or("Unterminated SVG comment")?;
            rest = &after[end + 3..];
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            let end = rest.find('>').ok_or("Unterminated SVG declaration")?;
            rest = &rest[end + 1..];
            continue;
        }
        let end = find_tag_end(rest).ok_or("Unterminated SVG tag")?;
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            let (open, previous) = stack.pop().ok_or("Unbalanced SVG closing tag")?;
            if open != name {
                return Err(format!("Mismatched SVG tag </{name}>"));
            }
            style = previous;
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, attributes) = split_tag(tag)?;
        let attributes = attributes_of(attributes)?;
        let get = |key: &str| {
            attributes
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
        };
        let previous = style.clone();
        if !seen_root {
            if name != "svg" {
                return Err("SVG icon must start with an <svg> element".into());
            }
            seen_root = true;
            let view_box = get("viewBox").ok_or("SVG icon needs a viewBox")?;
            let values = numbers(view_box)?;
            if values.len() != 4 || values[2] <= 0. || values[3] <= 0. {
                return Err("SVG viewBox must have four values and a positive size".into());
            }
            icon.view_box = [values[0], values[1], values[2], values[3]];
            if let Some(color) = get("color") {
                let rgb = parse_color(color)?;
                if rgb != [0, 0, 0] {
                    icon.color = Some(rgb);
                }
            }
        } else if name == "svg" {
            return Err("Nested <svg> elements are not supported".into());
        }
        apply_style(&mut style, &attributes)?;
        match name {
            "svg" | "g" | "title" | "desc" => {}
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => {
                let subpaths = geometry(name, &get, style.transform)?;
                points += subpaths.iter().map(|(p, _)| p.len()).sum::<usize>();
                if points > MAX_POINTS {
                    return Err("SVG icon geometry is too complex".into());
                }
                let scale = style.transform.scale_factor();
                icon.elements.push(Element {
                    subpaths,
                    fill: style.fill.unwrap_or(0.) * style.fill_opacity,
                    stroke: style.stroke.then_some(style.stroke_width * scale),
                    stroke_opacity: style.stroke_opacity,
                    dashes: style.dashes.iter().map(|d| d * scale).collect(),
                });
            }
            other => return Err(format!("Unsupported SVG element <{other}>")),
        }
        if self_closing {
            style = previous;
        } else {
            stack.push((name.to_string(), previous));
        }
    }
    if !seen_root {
        return Err("No <svg> element".into());
    }
    if !stack.is_empty() {
        return Err("Unclosed SVG element".into());
    }
    if icon.elements.is_empty() {
        return Err("SVG icon has no drawable elements".into());
    }
    Ok(icon)
}

fn find_tag_end(source: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in source.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if q == c => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn split_tag(tag: &str) -> Result<(&str, &str), String> {
    let tag = tag.trim();
    let end = tag.find(char::is_whitespace).unwrap_or(tag.len());
    let name = &tag[..end];
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("Invalid SVG tag name '{name}'"));
    }
    Ok((name, &tag[end..]))
}

fn attributes_of(mut source: &str) -> Result<Vec<(&str, String)>, String> {
    let mut out = vec![];
    loop {
        source = source.trim_start();
        if source.is_empty() {
            return Ok(out);
        }
        let eq = source.find('=').ok_or("SVG attribute without a value")?;
        let key = source[..eq].trim();
        source = source[eq + 1..].trim_start();
        let quote = source
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or("SVG attribute values must be quoted")?;
        let close = source[1..]
            .find(quote)
            .ok_or("Unterminated SVG attribute value")?;
        out.push((key, source[1..1 + close].to_string()));
        source = &source[close + 2..];
    }
}

fn parse_color(value: &str) -> Result<[u8; 3], String> {
    let hex = value
        .trim()
        .strip_prefix('#')
        .ok_or_else(|| format!("Unsupported SVG color '{value}'"))?;
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let bytes = hex.as_bytes();
    let rgb = match bytes.len() {
        3 => [bytes[0], bytes[1], bytes[2]]
            .map(|c| digit(c).map(|d| d * 17))
            .into_iter()
            .collect::<Option<Vec<_>>>(),
        6 => (0..3)
            .map(|i| Some(digit(bytes[2 * i])? * 16 + digit(bytes[2 * i + 1])?))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    }
    .ok_or_else(|| format!("Unsupported SVG color '{value}'"))?;
    Ok([rgb[0], rgb[1], rgb[2]])
}

fn paint_value(value: &str) -> Result<Option<f32>, String> {
    match value.trim() {
        "none" => Ok(None),
        "currentColor" => Ok(Some(1.)),
        other => parse_color(other).map(|_| Some(1.)),
    }
}

fn apply_style(style: &mut Style, attributes: &[(&str, String)]) -> Result<(), String> {
    let mut local = Affine::IDENTITY;
    for (key, value) in attributes {
        match *key {
            "fill" => style.fill = paint_value(value)?,
            "fill-opacity" => style.fill_opacity = number(value)?.clamp(0., 1.),
            "stroke" => style.stroke = paint_value(value)?.is_some(),
            "stroke-width" => style.stroke_width = number(value)?.max(0.),
            "stroke-opacity" => style.stroke_opacity = number(value)?.clamp(0., 1.),
            "stroke-dasharray" => {
                style.dashes = if value.trim() == "none" {
                    vec![]
                } else {
                    let mut dashes = numbers(value)?;
                    if dashes.iter().any(|d| *d < 0.) {
                        return Err("Negative SVG dash length".into());
                    }
                    if dashes.len() % 2 == 1 {
                        dashes.extend(dashes.clone());
                    }
                    if dashes.iter().sum::<f32>() <= 0. {
                        vec![]
                    } else {
                        dashes
                    }
                }
            }
            "transform" => local = transform(value)?,
            _ => {}
        }
    }
    style.transform = style.transform.then(local);
    Ok(())
}

fn transform(value: &str) -> Result<Affine, String> {
    let mut result = Affine::IDENTITY;
    let mut rest = value.trim();
    while !rest.is_empty() {
        let open = rest.find('(').ok_or("Invalid SVG transform")?;
        let close = rest.find(')').ok_or("Invalid SVG transform")?;
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let args = numbers(&rest[open + 1..close])?;
        let next = match (name, args.as_slice()) {
            ("translate", [x]) => Affine([1., 0., 0., 1., *x, 0.]),
            ("translate", [x, y]) => Affine([1., 0., 0., 1., *x, *y]),
            ("scale", [s]) => Affine([*s, 0., 0., *s, 0., 0.]),
            ("scale", [x, y]) => Affine([*x, 0., 0., *y, 0., 0.]),
            ("rotate", [a]) => rotation(*a),
            ("rotate", [a, cx, cy]) => Affine([1., 0., 0., 1., *cx, *cy])
                .then(rotation(*a))
                .then(Affine([1., 0., 0., 1., -cx, -cy])),
            ("matrix", [a, b, c, d, e, f]) => Affine([*a, *b, *c, *d, *e, *f]),
            _ => return Err(format!("Unsupported SVG transform '{name}'")),
        };
        result = result.then(next);
        rest = rest[close + 1..].trim();
    }
    Ok(result)
}

fn rotation(degrees: f32) -> Affine {
    let (s, c) = degrees.to_radians().sin_cos();
    Affine([c, s, -s, c, 0., 0.])
}

fn number(value: &str) -> Result<f32, String> {
    let values = numbers(value)?;
    match values.as_slice() {
        [v] => Ok(*v),
        _ => Err(format!("Expected one number in '{value}'")),
    }
}

fn numbers(value: &str) -> Result<Vec<f32>, String> {
    let mut scanner = Scanner::new(value);
    let mut out = vec![];
    while scanner.skip_separators() {
        out.push(scanner.number()?);
    }
    Ok(out)
}

struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Scanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
        }
    }
    /// Skip whitespace and commas; true while input remains.
    fn skip_separators(&mut self) -> bool {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            self.pos += 1;
        }
        self.pos < self.bytes.len()
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn starts_number(&mut self) -> bool {
        self.skip_separators()
            && self
                .peek()
                .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.'))
    }
    fn number(&mut self) -> Result<f32, String> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.pos += 1;
        }
        let mut digits = false;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
            digits = true;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
                digits = true;
            }
        }
        if digits && matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'-' | b'+')) {
                self.pos += 1;
            }
            if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = mark;
            }
        }
        if !digits {
            return Err("Expected a number in SVG data".into());
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .ok_or_else(|| "Invalid number in SVG data".into())
    }
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_separators();
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err("Expected an SVG arc flag".into()),
        }
    }
}

type Subpaths = Vec<(Vec<[f32; 2]>, bool)>;

fn geometry<'a>(
    name: &str,
    get: &dyn Fn(&str) -> Option<&'a str>,
    transform: Affine,
) -> Result<Subpaths, String> {
    let value = |key: &str| -> Result<f32, String> {
        get(key).map(number).transpose().map(|v| v.unwrap_or(0.))
    };
    let mut subpaths: Subpaths = match name {
        "path" => path_data(get("d").ok_or("SVG path without d")?)?,
        "rect" => {
            let (x, y, w, h) = (value("x")?, value("y")?, value("width")?, value("height")?);
            if w <= 0. || h <= 0. {
                return Err("SVG rect needs a positive size".into());
            }
            let mut rx = get("rx").map(number).transpose()?;
            let mut ry = get("ry").map(number).transpose()?;
            if rx.is_none() {
                rx = ry;
            }
            if ry.is_none() {
                ry = rx;
            }
            let (rx, ry) = (
                rx.unwrap_or(0.).clamp(0., w / 2.),
                ry.unwrap_or(0.).clamp(0., h / 2.),
            );
            if rx > 0. && ry > 0. {
                let mut points = vec![];
                for (cx, cy, start) in [
                    (x + w - rx, y + ry, -90f32),
                    (x + w - rx, y + h - ry, 0.),
                    (x + rx, y + h - ry, 90.),
                    (x + rx, y + ry, 180.),
                ] {
                    for step in 0..=8 {
                        let a = (start + step as f32 * 90. / 8.).to_radians();
                        points.push([cx + rx * a.cos(), cy + ry * a.sin()]);
                    }
                }
                vec![(points, true)]
            } else {
                vec![(vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true)]
            }
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (value("cx")?, value("cy")?);
            let (rx, ry) = if name == "circle" {
                let r = value("r")?;
                (r, r)
            } else {
                (value("rx")?, value("ry")?)
            };
            if rx <= 0. || ry <= 0. {
                return Err("SVG circle or ellipse needs a positive radius".into());
            }
            let steps = 48;
            let points = (0..steps)
                .map(|i| {
                    let a = i as f32 / steps as f32 * std::f32::consts::TAU;
                    [cx + rx * a.cos(), cy + ry * a.sin()]
                })
                .collect();
            vec![(points, true)]
        }
        "line" => vec![(
            vec![[value("x1")?, value("y1")?], [value("x2")?, value("y2")?]],
            false,
        )],
        "polyline" | "polygon" => {
            let values = numbers(get("points").unwrap_or(""))?;
            if values.len() < 4 || values.len() % 2 == 1 {
                return Err("SVG polyline needs coordinate pairs".into());
            }
            vec![(
                values.chunks(2).map(|p| [p[0], p[1]]).collect(),
                name == "polygon",
            )]
        }
        _ => unreachable!(),
    };
    for (points, _) in &mut subpaths {
        for p in points.iter_mut() {
            *p = transform.apply(*p);
        }
    }
    Ok(subpaths)
}

fn path_data(d: &str) -> Result<Subpaths, String> {
    let mut scanner = Scanner::new(d);
    let mut out: Subpaths = vec![];
    let mut current: Vec<[f32; 2]> = vec![];
    let mut pen = [0f32, 0.];
    let mut start = [0f32, 0.];
    let mut last_control: Option<(u8, [f32; 2])> = None;
    let mut command: Option<u8> = None;
    let flush = |current: &mut Vec<[f32; 2]>, out: &mut Subpaths, closed: bool| {
        if current.len() > 1 || (closed && !current.is_empty()) {
            out.push((std::mem::take(current), closed));
        } else if current.len() == 1 {
            // A lone moveto paints a round-cap dot only when stroked with a
            // zero-length segment; keep it as an open degenerate segment.
            let p = current[0];
            current.push(p);
            out.push((std::mem::take(current), false));
        }
        current.clear();
    };
    while scanner.skip_separators() {
        let c = scanner.peek().unwrap();
        if c.is_ascii_alphabetic() {
            scanner.pos += 1;
            command = Some(c);
            if matches!(c, b'Z' | b'z') {
                if !current.is_empty() {
                    flush(&mut current, &mut out, true);
                }
                pen = start;
                last_control = None;
                command = None;
                continue;
            }
        } else if command.is_none() {
            return Err("SVG path data must start with a command".into());
        }
        let cmd = command.ok_or("Missing SVG path command")?;
        let relative = cmd.is_ascii_lowercase();
        let base = if relative { pen } else { [0., 0.] };
        let point = |scanner: &mut Scanner| -> Result<[f32; 2], String> {
            let x = scanner.number_after_separators()?;
            let y = scanner.number_after_separators()?;
            Ok([base[0] + x, base[1] + y])
        };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let p = point(&mut scanner)?;
                if !current.is_empty() {
                    flush(&mut current, &mut out, false);
                }
                current.push(p);
                pen = p;
                start = p;
                last_control = None;
                // Subsequent pairs are implicit linetos.
                command = Some(if relative { b'l' } else { b'L' });
            }
            b'L' => {
                let p = point(&mut scanner)?;
                ensure_start(&mut current, pen);
                current.push(p);
                pen = p;
                last_control = None;
            }
            b'H' => {
                let x = scanner.number_after_separators()?;
                ensure_start(&mut current, pen);
                pen = [if relative { pen[0] + x } else { x }, pen[1]];
                current.push(pen);
                last_control = None;
            }
            b'V' => {
                let y = scanner.number_after_separators()?;
                ensure_start(&mut current, pen);
                pen = [pen[0], if relative { pen[1] + y } else { y }];
                current.push(pen);
                last_control = None;
            }
            b'C' | b'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&b'C') {
                    point(&mut scanner)?
                } else {
                    match last_control {
                        Some((b'C', c)) => [2. * pen[0] - c[0], 2. * pen[1] - c[1]],
                        _ => pen,
                    }
                };
                let c2 = point(&mut scanner)?;
                let end = point(&mut scanner)?;
                ensure_start(&mut current, pen);
                flatten_cubic(&mut current, pen, c1, c2, end);
                pen = end;
                last_control = Some((b'C', c2));
            }
            b'Q' | b'T' => {
                let c = if cmd.eq_ignore_ascii_case(&b'Q') {
                    point(&mut scanner)?
                } else {
                    match last_control {
                        Some((b'Q', c)) => [2. * pen[0] - c[0], 2. * pen[1] - c[1]],
                        _ => pen,
                    }
                };
                let end = point(&mut scanner)?;
                ensure_start(&mut current, pen);
                let c1 = [
                    pen[0] + 2. / 3. * (c[0] - pen[0]),
                    pen[1] + 2. / 3. * (c[1] - pen[1]),
                ];
                let c2 = [
                    end[0] + 2. / 3. * (c[0] - end[0]),
                    end[1] + 2. / 3. * (c[1] - end[1]),
                ];
                flatten_cubic(&mut current, pen, c1, c2, end);
                pen = end;
                last_control = Some((b'Q', c));
            }
            b'A' => {
                let rx = scanner.number_after_separators()?.abs();
                let ry = scanner.number_after_separators()?.abs();
                let rotation = scanner.number_after_separators()?;
                let large = scanner.flag()?;
                let sweep = scanner.flag()?;
                let end = point(&mut scanner)?;
                ensure_start(&mut current, pen);
                flatten_arc(&mut current, pen, rx, ry, rotation, large, sweep, end);
                pen = end;
                last_control = None;
            }
            other => {
                return Err(format!("Unsupported SVG path command '{}'", other as char));
            }
        }
        if !scanner.starts_number() && scanner.peek().is_some_and(|b| !b.is_ascii_alphabetic()) {
            return Err("Unexpected character in SVG path data".into());
        }
    }
    if !current.is_empty() {
        flush(&mut current, &mut out, false);
    }
    Ok(out)
}

impl Scanner<'_> {
    fn number_after_separators(&mut self) -> Result<f32, String> {
        self.skip_separators();
        self.number()
    }
}

fn ensure_start(current: &mut Vec<[f32; 2]>, pen: [f32; 2]) {
    if current.is_empty() {
        current.push(pen);
    }
}

fn flatten_cubic(out: &mut Vec<[f32; 2]>, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) {
    let length = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
    let steps = ((length / 1.5).ceil() as usize).clamp(4, 64);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1. - t;
        let w = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
        out.push([
            w[0] * p0[0] + w[1] * p1[0] + w[2] * p2[0] + w[3] * p3[0],
            w[0] * p0[1] + w[1] * p1[1] + w[2] * p2[1] + w[3] * p3[1],
        ]);
    }
}

/// SVG 1.1 implementation note F.6.5 endpoint-to-center conversion.
#[allow(clippy::too_many_arguments)]
fn flatten_arc(
    out: &mut Vec<[f32; 2]>,
    from: [f32; 2],
    mut rx: f32,
    mut ry: f32,
    rotation: f32,
    large: bool,
    sweep: bool,
    to: [f32; 2],
) {
    if rx == 0. || ry == 0. || dist(from, to) < 1e-6 {
        out.push(to);
        return;
    }
    let (sin, cos) = rotation.to_radians().sin_cos();
    let dx = (from[0] - to[0]) / 2.;
    let dy = (from[1] - to[1]) / 2.;
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1. {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut factor = (numerator / denominator).max(0.).sqrt();
    if large == sweep {
        factor = -factor;
    }
    let cx1 = factor * rx * y1 / ry;
    let cy1 = -factor * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (from[0] + to[0]) / 2.;
    let cy = sin * cx1 + cos * cy1 + (from[1] + to[1]) / 2.;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let ux = (x1 - cx1) / rx;
    let uy = (y1 - cy1) / ry;
    let vx = (-x1 - cx1) / rx;
    let vy = (-y1 - cy1) / ry;
    let theta = angle(1., 0., ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0. {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0. {
        delta += std::f32::consts::TAU;
    }
    let steps = ((delta.abs() * rx.max(ry) / 1.5).ceil() as usize).clamp(4, 96);
    for i in 1..=steps {
        let t = theta + delta * i as f32 / steps as f32;
        let (s, c) = t.sin_cos();
        out.push([
            cx + cos * rx * c - sin * ry * s,
            cy + sin * rx * c + cos * ry * s,
        ]);
    }
    if let Some(last) = out.last_mut() {
        *last = to;
    }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Square alpha mask, row-major, one byte per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub size: usize,
    pub alpha: Vec<u8>,
}
impl Mask {
    pub fn coverage(&self) -> usize {
        self.alpha.iter().filter(|a| **a > 127).count()
    }
}

/// Minimum stroke width in pixels so 16 px renderings keep visible lines.
const MIN_STROKE_PX: f32 = 1.1;

impl SvgIcon {
    pub fn rasterize(&self, size: usize) -> Mask {
        let size = size.clamp(4, 512);
        let [vx, vy, vw, vh] = self.view_box;
        let scale = size as f32 / vw.max(vh);
        let offset = [
            -vx * scale + (size as f32 - vw * scale) / 2.,
            -vy * scale + (size as f32 - vh * scale) / 2.,
        ];
        let map = |p: [f32; 2]| [p[0] * scale + offset[0], p[1] * scale + offset[1]];
        let mut dst = vec![0f32; size * size];
        let mut layer = vec![0f32; size * size];
        for element in &self.elements {
            let subpaths: Vec<(Vec<[f32; 2]>, bool)> = element
                .subpaths
                .iter()
                .map(|(points, closed)| (points.iter().map(|p| map(*p)).collect(), *closed))
                .collect();
            if element.fill > 0. {
                layer.fill(0.);
                fill_nonzero(&subpaths, size, &mut layer);
                composite(&mut dst, &layer, element.fill);
            }
            if let Some(width) = element.stroke
                && width > 0.
                && element.stroke_opacity > 0.
            {
                layer.fill(0.);
                let width = (width * scale).max(MIN_STROKE_PX);
                let dashes: Vec<f32> = element.dashes.iter().map(|d| d * scale).collect();
                for (points, closed) in &subpaths {
                    let mut points = points.clone();
                    if *closed && points.first() != points.last() {
                        points.push(points[0]);
                    }
                    for segment in dash(&points, &dashes) {
                        stroke_polyline(&segment, width / 2., size, &mut layer);
                    }
                }
                composite(&mut dst, &layer, element.stroke_opacity);
            }
        }
        Mask {
            size,
            alpha: dst
                .into_iter()
                .map(|a| (a.clamp(0., 1.) * 255.).round() as u8)
                .collect(),
        }
    }
}

fn composite(dst: &mut [f32], layer: &[f32], opacity: f32) {
    for (d, s) in dst.iter_mut().zip(layer) {
        let s = s * opacity;
        *d = *d + s * (1. - *d);
    }
}

/// Split a polyline into dash pieces; returns the input when not dashed.
fn dash(points: &[[f32; 2]], pattern: &[f32]) -> Vec<Vec<[f32; 2]>> {
    if pattern.is_empty() {
        return vec![points.to_vec()];
    }
    let mut out = vec![];
    let mut index = 0;
    let mut remaining = pattern[0];
    let mut on = true;
    let mut piece = vec![points[0]];
    for window in points.windows(2) {
        let (mut a, b) = (window[0], window[1]);
        let mut length = dist(a, b);
        while length > remaining {
            let t = remaining / length;
            let split = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if on {
                piece.push(split);
                out.push(std::mem::take(&mut piece));
            } else {
                piece = vec![split];
            }
            on = !on;
            length -= remaining;
            a = split;
            index = (index + 1) % pattern.len();
            remaining = pattern[index];
        }
        remaining -= length;
        if on {
            piece.push(b);
        } else {
            piece = vec![b];
        }
    }
    if on && piece.len() > 1 {
        out.push(piece);
    }
    out
}

/// Round caps and joins follow from taking the maximum distance-based
/// coverage of every segment.
fn stroke_polyline(points: &[[f32; 2]], half: f32, size: usize, layer: &mut [f32]) {
    let segments: Vec<([f32; 2], [f32; 2])> = if points.len() == 1 {
        vec![(points[0], points[0])]
    } else {
        points.windows(2).map(|w| (w[0], w[1])).collect()
    };
    for (a, b) in segments {
        let min_x = (a[0].min(b[0]) - half - 1.).floor().max(0.) as usize;
        let max_x = ((a[0].max(b[0]) + half + 1.).ceil().max(0.) as usize).min(size);
        let min_y = (a[1].min(b[1]) - half - 1.).floor().max(0.) as usize;
        let max_y = ((a[1].max(b[1]) + half + 1.).ceil().max(0.) as usize).min(size);
        for y in min_y..max_y {
            for x in min_x..max_x {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let d = segment_distance(p, a, b);
                let coverage = (half + 0.5 - d).clamp(0., 1.);
                let cell = &mut layer[y * size + x];
                if coverage > *cell {
                    *cell = coverage;
                }
            }
        }
    }
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length <= 1e-12 {
        0.
    } else {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0., 1.)
    };
    dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t])
}

/// Nonzero-winding fill with 4 x 4 supersampling.
fn fill_nonzero(subpaths: &[(Vec<[f32; 2]>, bool)], size: usize, layer: &mut [f32]) {
    const SAMPLES: usize = 4;
    let edges: Vec<([f32; 2], [f32; 2])> = subpaths
        .iter()
        .filter(|(points, _)| points.len() > 2)
        .flat_map(|(points, _)| {
            points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(a, b)| (*a, *b))
                .collect::<Vec<_>>()
        })
        .filter(|(a, b)| a[1] != b[1])
        .collect();
    if edges.is_empty() {
        return;
    }
    let mut crossings: Vec<(f32, i32)> = vec![];
    for y in 0..size {
        for sy in 0..SAMPLES {
            let sample_y = y as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
            crossings.clear();
            for (a, b) in &edges {
                let (top, bottom, winding) = if a[1] < b[1] { (a, b, 1) } else { (b, a, -1) };
                if sample_y >= top[1] && sample_y < bottom[1] {
                    let t = (sample_y - top[1]) / (bottom[1] - top[1]);
                    crossings.push((top[0] + (bottom[0] - top[0]) * t, winding));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            for x in 0..size {
                for sx in 0..SAMPLES {
                    let sample_x = x as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let winding: i32 = crossings
                        .iter()
                        .take_while(|(cx, _)| *cx < sample_x)
                        .map(|(_, w)| w)
                        .sum();
                    if winding != 0 {
                        layer[y * size + x] += 1. / (SAMPLES * SAMPLES) as f32;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">"#;

    fn icon(body: &str) -> Result<SvgIcon, String> {
        parse(&format!("{ROOT}<title>Test</title>{body}</svg>"))
    }

    #[test]
    fn compact_path_numbers_and_relative_commands_flatten_to_expected_points() {
        let paths = path_data("M10-5.5h4v-2l3 1zm1 1L2 2").unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].1);
        assert_eq!(
            paths[0].0,
            vec![[10., -5.5], [14., -5.5], [14., -7.5], [17., -6.5]]
        );
        // Implicit lineto after the first moveto pair.
        let implicit = path_data("M1 2 3 4").unwrap();
        assert_eq!(implicit[0].0, vec![[1., 2.], [3., 4.]]);
        // After z the pen returns to the subpath start.
        assert_eq!(paths[1].0[0], [11., -4.5]);
    }

    #[test]
    fn arcs_with_compact_flags_end_exactly_on_their_endpoint() {
        let paths = path_data("M0 0a10 10 0 0110 10").unwrap();
        let points = &paths[0].0;
        assert_eq!(*points.last().unwrap(), [10., 10.]);
        // A quarter circle centred at (0, 10) stays at radius 10.
        for p in points {
            assert!((dist(*p, [0., 10.]) - 10.).abs() < 0.05, "{p:?}");
        }
    }

    #[test]
    fn malformed_or_unsupported_artwork_is_rejected() {
        assert!(icon(r#"<image href="x.png"/>"#).is_err());
        assert!(icon(r#"<path d="X10 10"/>"#).is_err());
        assert!(icon(r#"<path d="M10"/>"#).is_err());
        assert!(icon(r#"<rect x="1" y="1" width="0" height="4"/>"#).is_err());
        assert!(parse("<g/>").is_err());
        assert!(parse(r#"<svg viewBox="0 0 0 64"><path d="M0 0h1"/></svg>"#).is_err());
        assert!(icon(r#"<g transform="skewX(10)"><path d="M0 0h1"/></g>"#).is_err());
        assert!(icon("").is_err(), "no drawable element");
        assert!(icon(r#"<g><path d="M0 0h1"/>"#).is_err(), "unclosed group");
    }

    #[test]
    fn strokes_fills_and_dashes_rasterize_inside_their_geometry() {
        let filled = icon(r#"<path d="M8 8h48v48H8Z" fill="currentColor" stroke="none"/>"#)
            .unwrap()
            .rasterize(64);
        assert_eq!(filled.alpha[32 * 64 + 32], 255);
        assert_eq!(filled.alpha[2 * 64 + 2], 0);
        assert!((filled.coverage() as i32 - 48 * 48).abs() < 100);
        let translucent = icon(
            r#"<path d="M8 8h48v48H8Z" fill="currentColor" fill-opacity=".3" stroke="none"/>"#,
        )
        .unwrap()
        .rasterize(64);
        assert!((translucent.alpha[32 * 64 + 32] as i32 - 77).abs() <= 1);
        let solid = icon(r#"<path d="M8 32h48"/>"#).unwrap().rasterize(64);
        let dashed = icon(r#"<path d="M8 32h48" stroke-dasharray="2 5"/>"#)
            .unwrap()
            .rasterize(64);
        assert!(solid.alpha[32 * 64 + 30] > 200);
        assert!(dashed.coverage() > 20 && dashed.coverage() < solid.coverage() * 3 / 4);
        // Round caps extend half the stroke width past the end point.
        assert!(solid.alpha[32 * 64 + 6] > 0);
        assert_eq!(solid.alpha[32 * 64 + 3], 0);
    }

    #[test]
    fn rotated_groups_and_root_colors_are_honoured() {
        let rotated = icon(r#"<g transform="rotate(90 32 32)"><path d="M8 32h48"/></g>"#)
            .unwrap()
            .rasterize(64);
        assert!(rotated.alpha[10 * 64 + 32] > 200, "now vertical");
        assert_eq!(rotated.alpha[32 * 64 + 10], 0);
        let green = parse(r##"<svg viewBox="0 0 64 64" color="#24aa60" fill="none" stroke="currentColor"><path d="M0 0h9"/></svg>"##).unwrap();
        assert_eq!(green.color, Some([0x24, 0xaa, 0x60]));
        let black = parse(r##"<svg viewBox="0 0 64 64" color="#000" stroke="currentColor"><path d="M0 0h9"/></svg>"##).unwrap();
        assert_eq!(black.color, None);
    }
}
