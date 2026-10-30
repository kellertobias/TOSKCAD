//! KiCad `.kicad_pcb` reading: the board outline from Edge.Cuts, drilled
//! holes, and footprint placements with courtyard envelopes and 3D model
//! references. KiCad uses millimeters with Y pointing down; everything here
//! is converted to Y up. Unsupported items are listed, never dropped
//! silently.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}
impl Sexp {
    fn head(&self) -> Option<&str> {
        match self {
            Self::List(items) => match items.first() {
                Some(Self::Atom(a)) => Some(a),
                _ => None,
            },
            Self::Atom(_) => None,
        }
    }
    fn items(&self) -> &[Sexp] {
        match self {
            Self::List(items) => items,
            Self::Atom(_) => &[],
        }
    }
    fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(a) => Some(a),
            Self::List(_) => None,
        }
    }
    fn child(&self, name: &str) -> Option<&Sexp> {
        self.items().iter().find(|i| i.head() == Some(name))
    }
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Sexp> + 'a {
        self.items().iter().filter(move |i| i.head() == Some(name))
    }
    /// The atoms after the head, as numbers.
    fn numbers(&self) -> Vec<f64> {
        self.items()
            .iter()
            .skip(1)
            .filter_map(|i| i.atom()?.parse().ok())
            .collect()
    }
    fn text(&self, index: usize) -> Option<&str> {
        self.items().get(index)?.atom()
    }
}

/// Parse one S-expression document.
pub fn parse_sexp(text: &str) -> Result<Sexp, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0;
    let mut stack: Vec<Vec<Sexp>> = vec![];
    let mut root = None;
    while pos < chars.len() {
        let c = chars[pos];
        match c {
            '(' => {
                stack.push(vec![]);
                pos += 1;
            }
            ')' => {
                let list = stack.pop().ok_or("Unbalanced ')' in KiCad file")?;
                let node = Sexp::List(list);
                match stack.last_mut() {
                    Some(parent) => parent.push(node),
                    None => {
                        if root.is_some() {
                            return Err("KiCad file has more than one top-level list".into());
                        }
                        root = Some(node);
                    }
                }
                pos += 1;
            }
            '"' => {
                let mut s = String::new();
                pos += 1;
                loop {
                    let Some(&c) = chars.get(pos) else {
                        return Err("Unterminated string in KiCad file".into());
                    };
                    pos += 1;
                    match c {
                        '"' => break,
                        '\\' => {
                            let next = chars.get(pos).copied().unwrap_or('\\');
                            pos += 1;
                            s.push(match next {
                                'n' => '\n',
                                't' => '\t',
                                other => other,
                            });
                        }
                        other => s.push(other),
                    }
                }
                stack
                    .last_mut()
                    .ok_or("Text outside a list in KiCad file")?
                    .push(Sexp::Atom(s));
            }
            c if c.is_whitespace() => pos += 1,
            _ => {
                let start = pos;
                while pos < chars.len()
                    && !chars[pos].is_whitespace()
                    && !matches!(chars[pos], '(' | ')' | '"')
                {
                    pos += 1;
                }
                stack
                    .last_mut()
                    .ok_or("Text outside a list in KiCad file")?
                    .push(Sexp::Atom(chars[start..pos].iter().collect()));
            }
        }
        if stack.len() > 256 {
            return Err("KiCad file nests too deeply".into());
        }
    }
    if !stack.is_empty() {
        return Err("Unbalanced '(' in KiCad file".into());
    }
    root.ok_or_else(|| "Empty KiCad file".into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutlineEdge {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Arc {
        center: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
        clockwise: bool,
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
}
impl OutlineEdge {
    fn ends(&self) -> Option<([f64; 2], [f64; 2])> {
        match self {
            Self::Line { a, b } | Self::Arc { a, b, .. } => Some((*a, *b)),
            Self::Circle { .. } => None,
        }
    }
    fn reversed(&self) -> Self {
        match self {
            Self::Line { a, b } => Self::Line { a: *b, b: *a },
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => Self::Arc {
                center: *center,
                a: *b,
                b: *a,
                clockwise: !clockwise,
            },
            c @ Self::Circle { .. } => c.clone(),
        }
    }
    /// A polyline approximation used for loop areas and containment.
    fn samples(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Line { a, .. } => vec![*a],
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => {
                let r = (a[0] - center[0]).hypot(a[1] - center[1]);
                let s = (a[1] - center[1]).atan2(a[0] - center[0]);
                let mut e = (b[1] - center[1]).atan2(b[0] - center[0]);
                if *clockwise {
                    while e > s {
                        e -= std::f64::consts::TAU;
                    }
                } else {
                    while e < s {
                        e += std::f64::consts::TAU;
                    }
                }
                (0..16)
                    .map(|i| {
                        let t = s + (e - s) * i as f64 / 16.;
                        [center[0] + r * t.cos(), center[1] + r * t.sin()]
                    })
                    .collect()
            }
            Self::Circle { center, radius } => (0..32)
                .map(|i| {
                    let t = std::f64::consts::TAU * i as f64 / 32.;
                    [center[0] + radius * t.cos(), center[1] + radius * t.sin()]
                })
                .collect(),
        }
    }
}
fn loop_area(edges: &[OutlineEdge]) -> f64 {
    let pts: Vec<_> = edges.iter().flat_map(OutlineEdge::samples).collect();
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
        / 2.
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardHole {
    pub center: [f64; 2],
    pub diameter: f64,
    pub plated: bool,
    /// The footprint reference the hole belongs to.
    pub footprint: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelReference {
    /// The path as written in the board, before variable substitution.
    pub path: String,
    pub offset: [f64; 3],
    pub scale: [f64; 3],
    pub rotate: [f64; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub reference: String,
    pub value: String,
    pub library: String,
    pub at: [f64; 2],
    pub rotation_deg: f64,
    pub back: bool,
    /// Courtyard (or pad) extents in footprint coordinates, Y up.
    pub envelope: [[f64; 2]; 2],
    pub models: Vec<ModelReference>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct KicadBoard {
    pub thickness_mm: f64,
    pub outer: Vec<OutlineEdge>,
    pub cutouts: Vec<Vec<OutlineEdge>>,
    pub holes: Vec<BoardHole>,
    pub footprints: Vec<Footprint>,
    /// Items that were read but not represented, with counts.
    pub unsupported: Vec<String>,
}

fn xy(s: &Sexp) -> Option<[f64; 2]> {
    let n = s.numbers();
    (n.len() >= 2).then(|| [n[0], -n[1]])
}
fn layer(s: &Sexp) -> Option<&str> {
    s.child("layer")?.text(1)
}
/// Rotate footprint-local (Y up) coordinates by the footprint rotation.
fn place(at: [f64; 2], rot_deg: f64, p: [f64; 2]) -> [f64; 2] {
    let (s, c) = rot_deg.to_radians().sin_cos();
    [at[0] + p[0] * c - p[1] * s, at[1] + p[0] * s + p[1] * c]
}
fn arc_from_three(a: [f64; 2], m: [f64; 2], b: [f64; 2]) -> Option<OutlineEdge> {
    let d = 2. * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-12 {
        return None;
    }
    let sq = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d;
    let uy = (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d;
    let cross = (m[0] - a[0]) * (b[1] - a[1]) - (m[1] - a[1]) * (b[0] - a[0]);
    Some(OutlineEdge::Arc {
        center: [ux, uy],
        a,
        b,
        clockwise: cross < 0.,
    })
}

/// Read a board. `min_hole_mm` keeps only drilled holes at least that wide.
pub fn read_board(text: &str, min_hole_mm: f64) -> Result<KicadBoard, String> {
    let root = parse_sexp(text)?;
    if root.head() != Some("kicad_pcb") {
        return Err("Not a KiCad board: the file does not start with (kicad_pcb".into());
    }
    let mut unsupported = std::collections::BTreeMap::<String, usize>::new();
    let mut note = |what: String| *unsupported.entry(what).or_default() += 1;
    let thickness_mm = root
        .child("general")
        .and_then(|g| g.child("thickness"))
        .and_then(|t| t.numbers().first().copied())
        .unwrap_or(1.6);
    let mut edges = vec![];
    for item in root.items() {
        let Some(head) = item.head() else { continue };
        if !head.starts_with("gr_") || layer(item) != Some("Edge.Cuts") {
            continue;
        }
        let start = item.child("start").and_then(xy);
        let end = item.child("end").and_then(xy);
        match (head, start, end) {
            ("gr_line", Some(a), Some(b)) => edges.push(OutlineEdge::Line { a, b }),
            ("gr_rect", Some(a), Some(b)) => {
                let c = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]];
                for i in 0..4 {
                    edges.push(OutlineEdge::Line {
                        a: c[i],
                        b: c[(i + 1) % 4],
                    });
                }
            }
            ("gr_arc", Some(a), Some(b)) => match item.child("mid").and_then(xy) {
                Some(m) => match arc_from_three(a, m, b) {
                    Some(arc) => edges.push(arc),
                    None => note("degenerate Edge.Cuts arc".into()),
                },
                None => note("KiCad 5 Edge.Cuts arc (start/end/angle)".into()),
            },
            ("gr_circle", _, Some(e)) => {
                if let Some(c) = item.child("center").and_then(xy) {
                    edges.push(OutlineEdge::Circle {
                        center: c,
                        radius: (e[0] - c[0]).hypot(e[1] - c[1]),
                    });
                }
            }
            ("gr_poly", _, _) => {
                let pts: Vec<_> = item
                    .child("pts")
                    .map(|p| p.children("xy").filter_map(xy).collect())
                    .unwrap_or_default();
                for i in 0..pts.len() {
                    edges.push(OutlineEdge::Line {
                        a: pts[i],
                        b: pts[(i + 1) % pts.len()],
                    });
                }
            }
            _ => note(format!("Edge.Cuts {head}")),
        }
    }
    let mut loops = chain(edges)?;
    if loops.is_empty() {
        return Err("The board has no closed Edge.Cuts outline".into());
    }
    loops.sort_by(|a, b| loop_area(b).abs().total_cmp(&loop_area(a).abs()));
    let mut outer = loops.remove(0);
    if loop_area(&outer) < 0. {
        outer = outer.iter().rev().map(OutlineEdge::reversed).collect();
    }
    let cutouts = loops
        .into_iter()
        .map(|l| {
            if loop_area(&l) > 0. {
                l.iter().rev().map(OutlineEdge::reversed).collect()
            } else {
                l
            }
        })
        .collect();
    let mut holes = vec![];
    let mut footprints = vec![];
    for fp in root
        .items()
        .iter()
        .filter(|i| matches!(i.head(), Some("footprint" | "module")))
    {
        let library = fp.text(1).unwrap_or_default().to_owned();
        let at_n = fp.child("at").map(Sexp::numbers).unwrap_or_default();
        let at = [
            at_n.first().copied().unwrap_or(0.),
            -at_n.get(1).copied().unwrap_or(0.),
        ];
        let rotation_deg = at_n.get(2).copied().unwrap_or(0.);
        let back = layer(fp) == Some("B.Cu");
        let property = |name: &str| {
            fp.children("property")
                .find(|p| p.text(1) == Some(name))
                .and_then(|p| p.text(2))
                .or_else(|| {
                    fp.children("fp_text")
                        .find(|t| t.text(1) == Some(&name.to_lowercase()))
                        .and_then(|t| t.text(2))
                })
                .unwrap_or_default()
                .to_owned()
        };
        let reference = property("Reference");
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        let mut grow = |p: [f64; 2]| {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        };
        let mut courtyard = false;
        for g in fp.items() {
            let Some(head) = g.head() else { continue };
            if head.starts_with("fp_") && matches!(layer(g), Some("F.CrtYd" | "B.CrtYd")) {
                courtyard = true;
                // A circle's end is a point on it; its extent is center ± radius.
                if head == "fp_circle"
                    && let (Some(c), Some(e)) =
                        (g.child("center").and_then(xy), g.child("end").and_then(xy))
                {
                    let r = (e[0] - c[0]).hypot(e[1] - c[1]);
                    grow([c[0] - r, c[1] - r]);
                    grow([c[0] + r, c[1] + r]);
                    continue;
                }
                for key in ["start", "end", "center", "mid"] {
                    if let Some(p) = g.child(key).and_then(xy) {
                        grow(p);
                    }
                }
                if let Some(pts) = g.child("pts") {
                    pts.children("xy").filter_map(xy).for_each(&mut grow);
                }
            }
        }
        let mut pad_lo = [f64::INFINITY; 2];
        let mut pad_hi = [f64::NEG_INFINITY; 2];
        for pad in fp.children("pad") {
            let kind = pad.text(2).unwrap_or_default();
            let local = pad.child("at").and_then(xy).unwrap_or([0., 0.]);
            let size = pad.child("size").map(Sexp::numbers).unwrap_or_default();
            if size.len() >= 2 {
                for k in 0..2 {
                    pad_lo[k] = pad_lo[k].min(local[k] - size[k] / 2.);
                    pad_hi[k] = pad_hi[k].max(local[k] + size[k] / 2.);
                }
            }
            if let Some(drill) = pad.child("drill") {
                let n = drill.numbers();
                if drill.items().iter().any(|i| i.atom() == Some("oval")) {
                    note("oval drill".into());
                    continue;
                }
                if let Some(&d) = n.first()
                    && d + 1e-9 >= min_hole_mm
                {
                    holes.push(BoardHole {
                        center: place(at, rotation_deg, local),
                        diameter: d,
                        plated: kind == "thru_hole",
                        footprint: reference.clone(),
                    });
                }
            }
        }
        if !courtyard && pad_lo[0].is_finite() {
            grow([pad_lo[0] - 0.25, pad_lo[1] - 0.25]);
            grow([pad_hi[0] + 0.25, pad_hi[1] + 0.25]);
        }
        let models = fp
            .children("model")
            .map(|m| {
                let triple = |name: &str, default: f64| {
                    m.child(name)
                        .and_then(|c| c.child("xyz"))
                        .map(|x| {
                            let n = x.numbers();
                            [0, 1, 2].map(|i| n.get(i).copied().unwrap_or(default))
                        })
                        .unwrap_or([default; 3])
                };
                ModelReference {
                    path: m.text(1).unwrap_or_default().to_owned(),
                    offset: triple("offset", 0.),
                    scale: triple("scale", 1.),
                    rotate: triple("rotate", 0.),
                }
            })
            .collect();
        footprints.push(Footprint {
            reference,
            value: property("Value"),
            library,
            at,
            rotation_deg,
            back,
            envelope: if lo[0].is_finite() {
                [lo, hi]
            } else {
                [[-0.5, -0.5], [0.5, 0.5]]
            },
            models,
        });
    }
    if root.children("via").next().is_some() {
        note(format!("{} vias", root.children("via").count()));
    }
    Ok(KicadBoard {
        thickness_mm,
        outer,
        cutouts,
        holes,
        footprints,
        unsupported: unsupported
            .into_iter()
            .map(|(what, n)| format!("{n} × {what}"))
            .collect(),
    })
}

/// Chain outline edges into closed loops; circles are loops on their own.
fn chain(edges: Vec<OutlineEdge>) -> Result<Vec<Vec<OutlineEdge>>, String> {
    let close = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-3;
    let mut loops = vec![];
    let mut open: Vec<OutlineEdge> = vec![];
    for e in edges {
        if matches!(e, OutlineEdge::Circle { .. }) {
            loops.push(vec![e]);
        } else {
            open.push(e);
        }
    }
    while let Some(first) = open.pop() {
        let (start, mut end) = first.ends().expect("open edge");
        let mut current = vec![first];
        while !close(end, start) {
            let next = open.iter().position(|e| {
                let (a, b) = e.ends().expect("open edge");
                close(a, end) || close(b, end)
            });
            let Some(i) = next else {
                return Err(format!(
                    "The Edge.Cuts outline is not closed near ({:.3}, {:.3})",
                    end[0], -end[1]
                ));
            };
            let e = open.remove(i);
            let (a, _) = e.ends().expect("open edge");
            let e = if close(a, end) { e } else { e.reversed() };
            end = e.ends().expect("open edge").1;
            current.push(e);
        }
        loops.push(current);
    }
    Ok(loops)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sexp_strings_escapes_and_nesting_parse() {
        let s = parse_sexp(r#"(a "b c" (d 1 -2.5) "e\"f")"#).unwrap();
        assert_eq!(s.text(1), Some("b c"));
        assert_eq!(s.child("d").unwrap().numbers(), vec![1., -2.5]);
        assert_eq!(s.text(3), Some("e\"f"));
        assert!(parse_sexp("(a (b)").is_err());
        assert!(parse_sexp("(a))").is_err());
    }
    #[test]
    fn circular_courtyards_give_a_square_envelope_around_the_circle() {
        let board = read_board(
            r#"(kicad_pcb (general (thickness 1.6))
  (footprint "MountingHole:M3" (layer "F.Cu") (at 5 5)
    (property "Reference" "H1")
    (fp_circle (center 0 0) (end 3.45 0) (layer "F.CrtYd"))
    (pad "" np_thru_hole circle (at 0 0) (size 3.2 3.2) (drill 3.2)))
  (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts")))"#,
            2.,
        )
        .unwrap();
        let [lo, hi] = board.footprints[0].envelope;
        for (v, expected) in [lo[0], lo[1], hi[0], hi[1]]
            .into_iter()
            .zip([-3.45, -3.45, 3.45, 3.45])
        {
            assert!(
                (v - expected).abs() < 1e-9,
                "{:?}",
                board.footprints[0].envelope
            );
        }
    }
    #[test]
    fn rotated_footprint_pads_land_in_y_up_board_coordinates() {
        // A footprint at (10, 20) turned 90°: its pad at local x = 1 moves up.
        assert_eq!(
            place([10., -20.], 90., [1., 0.]).map(|v| (v * 1e9).round() / 1e9),
            [10., -19.]
        );
    }
}
