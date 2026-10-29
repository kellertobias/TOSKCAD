//! ASCII DXF model-space reader. Only the ENTITIES section and `$INSUNITS`
//! are interpreted; blocks, paper space and annotation content are reported.
//! Entity semantics follow Autodesk's DXF reference: OCS entities with a
//! negative Z extrusion are mirrored by the arbitrary-axis rule.
use super::{
    Artwork, Collector, Diagnostics, Format, ImportError, UnitChoice, fail,
    geometry::{Affine, P, Segment, ellipse_arc, transform},
};
use std::f64::consts::TAU;

const MAX_PAIRS: usize = 8_000_000;

type Pair<'a> = (i32, &'a str);

struct Record<'a> {
    kind: &'a str,
    codes: Vec<Pair<'a>>,
}
impl<'a> Record<'a> {
    fn text(&self, code: i32) -> Option<&'a str> {
        self.codes.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
    }
    fn number(&self, code: i32) -> Result<Option<f64>, ImportError> {
        self.text(code).map(|v| number(v, code)).transpose()
    }
    fn required(&self, code: i32) -> Result<f64, ImportError> {
        self.number(code)?
            .ok_or_else(|| ImportError(format!("{} is missing group code {code}", self.kind)))
    }
    fn int(&self, code: i32) -> Result<i64, ImportError> {
        Ok(self.number(code)?.unwrap_or(0.) as i64)
    }
    fn layer(&self) -> &'a str {
        self.text(8).filter(|l| !l.is_empty()).unwrap_or("0")
    }
    /// `Some(true)` for +Z, `Some(false)` for −Z, `None` for a tilted OCS.
    fn extrusion(&self) -> Result<Option<bool>, ImportError> {
        let n = [
            self.number(210)?.unwrap_or(0.),
            self.number(220)?.unwrap_or(0.),
            self.number(230)?.unwrap_or(1.),
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if !len.is_finite() || len == 0. {
            return fail(format!("{} has an invalid extrusion direction", self.kind));
        }
        Ok((n[0].abs() / len < 1e-9 && n[1].abs() / len < 1e-9).then_some(n[2] > 0.))
    }
}

fn number(value: &str, code: i32) -> Result<f64, ImportError> {
    let parsed: f64 = value
        .trim()
        .parse()
        .map_err(|_| ImportError(format!("Group code {code} has a non-numeric value")))?;
    if !parsed.is_finite() || parsed.abs() > 1e12 {
        return fail(format!("Group code {code} is nonfinite or out of range"));
    }
    Ok(parsed)
}

fn pairs(text: &str) -> Result<Vec<Pair<'_>>, ImportError> {
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(code) = lines.next() {
        let Some(value) = lines.next() else { break };
        let code = code.trim();
        if code.is_empty() && value.trim().is_empty() {
            continue;
        }
        let code: i32 = code.parse().map_err(|_| {
            ImportError("Malformed DXF group code; only ASCII DXF is supported".into())
        })?;
        out.push((code, value.trim()));
        if out.len() > MAX_PAIRS {
            return fail("DXF exceeds the group-code parsing budget");
        }
    }
    Ok(out)
}

fn insunits(code: i64) -> Option<(f64, &'static str)> {
    Some(match code {
        1 => (25.4, "inches"),
        2 => (304.8, "feet"),
        3 => (1_609_344., "miles"),
        4 => (1., "millimeters"),
        5 => (10., "centimeters"),
        6 => (1000., "meters"),
        7 => (1_000_000., "kilometers"),
        8 => (25.4e-6, "microinches"),
        9 => (0.0254, "mils"),
        10 => (914.4, "yards"),
        13 => (1e-3, "microns"),
        14 => (100., "decimeters"),
        _ => return None,
    })
}

pub fn parse(bytes: &[u8], units: UnitChoice) -> Result<Artwork, ImportError> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return fail("Binary DXF is not supported; save the drawing as ASCII DXF");
    }
    let text = String::from_utf8_lossy(bytes);
    let pairs = pairs(&text)?;
    let mut declared = None;
    let mut entities: Vec<Record> = Vec::new();
    let mut i = 0;
    let mut saw_entities = false;
    while i < pairs.len() {
        if pairs[i] == (0, "SECTION") {
            let name = pairs.get(i + 1).filter(|p| p.0 == 2).map(|p| p.1);
            i += 2;
            let end = pairs[i.min(pairs.len())..]
                .iter()
                .position(|p| *p == (0, "ENDSEC"))
                .map_or(pairs.len(), |p| p + i);
            let section = &pairs[i.min(end)..end];
            match name {
                Some("HEADER") => {
                    if let Some(at) = section.iter().position(|p| *p == (9, "$INSUNITS"))
                        && let Some((70, v)) = section.get(at + 1)
                    {
                        declared = Some(number(v, 70)? as i64);
                    }
                }
                Some("ENTITIES") => {
                    saw_entities = true;
                    for pair in section {
                        if pair.0 == 0 {
                            entities.push(Record {
                                kind: pair.1,
                                codes: Vec::new(),
                            });
                        } else if let Some(last) = entities.last_mut() {
                            last.codes.push(*pair);
                        }
                    }
                }
                _ => {}
            }
            i = end + 1;
        } else if pairs[i] == (0, "EOF") {
            break;
        } else {
            i += 1;
        }
    }
    if !saw_entities {
        return fail("DXF has no ENTITIES section");
    }
    let (millimeters, unit_note) = match units {
        UnitChoice::Explicit(unit) => (
            unit.millimeters(),
            format!("Drawing units explicitly interpreted as {}", unit.label()),
        ),
        UnitChoice::FromFile => match declared {
            Some(code) => match insunits(code) {
                Some((mm, name)) => (mm, format!("$INSUNITS {code} declares {name}")),
                None if code == 0 => {
                    return fail("DXF is unitless ($INSUNITS 0); choose source units explicitly");
                }
                None => {
                    return fail(format!(
                        "DXF $INSUNITS {code} is not supported; choose source units explicitly"
                    ));
                }
            },
            None => return fail("DXF declares no $INSUNITS; choose source units explicitly"),
        },
    };
    let mut out = Collector::default();
    let mut diagnostics = Diagnostics::default();
    let mut notes = vec![unit_note];
    let mut projected = 0usize;
    let mut index = 0;
    while index < entities.len() {
        let record = &entities[index];
        index += 1;
        if record.int(67)? == 1 {
            diagnostics.add(
                "dxf.paper_space",
                "Paper-space entities are not model geometry",
            );
            continue;
        }
        if record.int(60)? == 1 {
            diagnostics.add("dxf.invisible", "Invisible entities are not imported");
            continue;
        }
        let layer = record.layer();
        out.layer(layer);
        match record.kind {
            "LINE" => {
                let a = [record.required(10)?, record.required(20)?];
                let b = [record.required(11)?, record.required(21)?];
                if record.number(30)?.unwrap_or(0.) != record.number(31)?.unwrap_or(0.) {
                    projected += 1;
                }
                match line(a, b) {
                    Some(s) => out.push(layer, vec![s], false)?,
                    None => diagnostics.add("dxf.degenerate", "Zero-length curves were skipped"),
                }
            }
            "CIRCLE" | "ARC" => {
                let Some(up) = record.extrusion()? else {
                    diagnostics.add(
                        "dxf.tilted_ocs",
                        "Entities outside the XY plane were skipped",
                    );
                    continue;
                };
                let center = [record.required(10)?, record.required(20)?];
                let radius = record.required(40)?;
                if radius <= 0. {
                    diagnostics.add("dxf.degenerate", "Zero-length curves were skipped");
                    continue;
                }
                let segment = if record.kind == "CIRCLE" {
                    Segment::Circle { center, radius }
                } else {
                    let start = record.required(50)?.to_radians();
                    let mut sweep = (record.required(51)?.to_radians() - start).rem_euclid(TAU);
                    if sweep < 1e-12 {
                        sweep = TAU;
                    }
                    if sweep >= TAU - 1e-12 {
                        Segment::Circle { center, radius }
                    } else {
                        Segment::Arc {
                            center,
                            radius,
                            start,
                            sweep,
                        }
                    }
                };
                let closed = matches!(segment, Segment::Circle { .. });
                out.push(layer, vec![ocs(&segment, up)], closed)?;
            }
            "LWPOLYLINE" => {
                let Some(up) = record.extrusion()? else {
                    diagnostics.add(
                        "dxf.tilted_ocs",
                        "Entities outside the XY plane were skipped",
                    );
                    continue;
                };
                let mut vertices: Vec<(P, f64)> = Vec::new();
                for (code, value) in &record.codes {
                    match code {
                        10 => vertices.push(([number(value, 10)?, 0.], 0.)),
                        20 => {
                            if let Some(v) = vertices.last_mut() {
                                v.0[1] = number(value, 20)?;
                            }
                        }
                        42 => {
                            if let Some(v) = vertices.last_mut() {
                                v.1 = number(value, 42)?;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = record.int(70)? & 1 == 1;
                emit_polyline(&mut out, &mut diagnostics, layer, &vertices, closed, up)?;
            }
            "POLYLINE" => {
                let flags = record.int(70)?;
                let mut vertices = Vec::new();
                while index < entities.len() && entities[index].kind == "VERTEX" {
                    let v = &entities[index];
                    index += 1;
                    let vflags = v.int(70)?;
                    if vflags & (16 | 64 | 128) != 0 {
                        continue;
                    }
                    vertices.push((
                        [v.required(10)?, v.required(20)?],
                        v.number(42)?.unwrap_or(0.),
                    ));
                }
                if entities.get(index).is_some_and(|r| r.kind == "SEQEND") {
                    index += 1;
                }
                if flags & (16 | 64) != 0 {
                    diagnostics.add(
                        "dxf.mesh",
                        "Polygon and polyface meshes are not sketch curves",
                    );
                    continue;
                }
                let up = if flags & 8 != 0 {
                    projected += 1;
                    Some(true)
                } else {
                    record.extrusion()?
                };
                let Some(up) = up else {
                    diagnostics.add(
                        "dxf.tilted_ocs",
                        "Entities outside the XY plane were skipped",
                    );
                    continue;
                };
                emit_polyline(
                    &mut out,
                    &mut diagnostics,
                    layer,
                    &vertices,
                    flags & 1 == 1,
                    up,
                )?;
            }
            "SPLINE" => match spline(record)? {
                Ok(segment) => {
                    let closed = segment.start() == segment.end();
                    out.push(layer, vec![segment], closed)?
                }
                Err((kind, message)) => diagnostics.add(kind, message),
            },
            "ELLIPSE" => {
                let Some(up) = record.extrusion()? else {
                    diagnostics.add(
                        "dxf.tilted_ocs",
                        "Entities outside the XY plane were skipped",
                    );
                    continue;
                };
                let c = [record.required(10)?, record.required(20)?];
                let u = [record.required(11)?, record.required(21)?];
                let ratio = record.required(40)?;
                let start = record.number(41)?.unwrap_or(0.);
                let end = record.number(42)?.unwrap_or(TAU);
                if u[0].hypot(u[1]) <= 0. || !(ratio > 0. && ratio <= 1. + 1e-9) {
                    diagnostics.add("dxf.degenerate", "Zero-length curves were skipped");
                    continue;
                }
                let z = if up { 1. } else { -1. };
                let v = [-z * u[1] * ratio, z * u[0] * ratio];
                let mut sweep = (end - start).rem_euclid(TAU);
                if sweep < 1e-12 {
                    sweep = TAU;
                }
                let full = sweep >= TAU - 1e-12;
                let segment = if (ratio - 1.).abs() < 1e-12 {
                    let r = u[0].hypot(u[1]);
                    if full {
                        Segment::Circle {
                            center: c,
                            radius: r,
                        }
                    } else {
                        Segment::Arc {
                            center: c,
                            radius: r,
                            start: u[1].atan2(u[0]) + z * start,
                            sweep: z * sweep,
                        }
                    }
                } else {
                    ellipse_arc(c, u, v, start, if full { TAU } else { sweep })
                };
                out.push(layer, vec![segment], full)?;
            }
            "INSERT" => diagnostics.add(
                "dxf.insert",
                "Block references (INSERT) are not exploded; explode them before export",
            ),
            "VERTEX" | "SEQEND" => {}
            other => diagnostics.add(
                &format!("dxf.{}", other.to_ascii_lowercase()),
                format!("{other} entities are not sketch geometry"),
            ),
        }
    }
    if projected > 0 {
        notes.push(format!(
            "{projected} entities had Z coordinates; they were projected onto the XY plane"
        ));
    }
    let scale = Affine::scale(millimeters, millimeters);
    let paths = out
        .paths
        .into_iter()
        .map(|mut p| {
            p.segments = p.segments.iter().map(|s| transform(s, &scale)).collect();
            p
        })
        .collect();
    Ok(Artwork {
        format: Format::Dxf,
        paths,
        layers: out.layers,
        unsupported: diagnostics.finish(),
        notes,
        millimeters_per_unit: millimeters,
    })
}

fn line(a: P, b: P) -> Option<Segment> {
    ((a[0] - b[0]).hypot(a[1] - b[1]) > 1e-12).then_some(Segment::Line { a, b })
}

/// Mirrors an OCS curve with a −Z extrusion into WCS (X ↦ −X).
fn ocs(segment: &Segment, up: bool) -> Segment {
    if up {
        segment.clone()
    } else {
        transform(segment, &Affine::scale(-1., 1.))
    }
}

/// Bulge `b = tan(θ/4)`, positive counterclockwise, on the edge after a vertex.
pub(crate) fn bulge_segment(a: P, b: P, bulge: f64) -> Option<Segment> {
    let chord = [b[0] - a[0], b[1] - a[1]];
    let d = chord[0].hypot(chord[1]);
    if d <= 1e-12 {
        return None;
    }
    if bulge.abs() < 1e-12 {
        return line(a, b);
    }
    let sagitta = bulge * d / 2.;
    let r = ((d / 2.).powi(2) + sagitta * sagitta) / (2. * sagitta);
    let n = [-chord[1] / d, chord[0] / d];
    let m = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
    let offset = r - sagitta;
    let center = [m[0] + n[0] * offset, m[1] + n[1] * offset];
    Some(Segment::Arc {
        center,
        radius: r.abs(),
        start: (a[1] - center[1]).atan2(a[0] - center[0]),
        sweep: 4. * bulge.atan(),
    })
}

fn emit_polyline(
    out: &mut Collector,
    diagnostics: &mut Diagnostics,
    layer: &str,
    vertices: &[(P, f64)],
    closed: bool,
    up: bool,
) -> Result<(), ImportError> {
    let n = vertices.len();
    let edges = if closed { n } else { n.saturating_sub(1) };
    let mut segments = Vec::new();
    for i in 0..edges {
        let (a, bulge) = vertices[i];
        let b = vertices[(i + 1) % n].0;
        match bulge_segment(a, b, bulge) {
            Some(s) => segments.push(ocs(&s, up)),
            None if closed && i + 1 == n => {}
            None => diagnostics.add("dxf.degenerate", "Zero-length curves were skipped"),
        }
    }
    out.push(layer, segments, closed && n > 1)
}

type Unsupported = (&'static str, &'static str);

fn spline(record: &Record) -> Result<Result<Segment, Unsupported>, ImportError> {
    let flags = record.int(70)?;
    let degree = record.int(71)?;
    let mut poles: Vec<P> = Vec::new();
    let mut knots = Vec::new();
    let mut weights = Vec::new();
    let mut fit = 0;
    for (code, value) in &record.codes {
        match code {
            10 => poles.push([number(value, 10)?, 0.]),
            20 => {
                if let Some(p) = poles.last_mut() {
                    p[1] = number(value, 20)?
                }
            }
            40 => knots.push(number(value, 40)?),
            41 => weights.push(number(value, 41)?),
            11 => fit += 1,
            _ => {}
        }
    }
    if poles.is_empty() {
        return Ok(Err((
            "dxf.spline_fit",
            if fit > 0 {
                "Fit-point-only SPLINE entities need control points; re-export with control vertices"
            } else {
                "SPLINE without control points was skipped"
            },
        )));
    }
    if !(1..=25).contains(&degree) || poles.len() < degree as usize + 1 {
        return Ok(Err((
            "dxf.spline_invalid",
            "SPLINE has an invalid degree or pole count",
        )));
    }
    if poles.len() > crate::constraint_sketch::MAX_SPLINE_POLES {
        return Ok(Err((
            "dxf.spline_large",
            "SPLINE exceeds the 256-pole sketch budget",
        )));
    }
    if knots.len() != poles.len() + degree as usize + 1 || knots.windows(2).any(|k| k[1] < k[0]) {
        return Ok(Err((
            "dxf.spline_invalid",
            "SPLINE knot vector does not match its poles",
        )));
    }
    let span = knots[knots.len() - 1] - knots[0];
    if span <= 0. {
        return Ok(Err((
            "dxf.spline_invalid",
            "SPLINE knot vector does not match its poles",
        )));
    }
    let tolerance = span * 1e-10;
    let mut distinct: Vec<f64> = Vec::new();
    let mut multiplicities: Vec<u32> = Vec::new();
    for k in knots {
        match distinct.last() {
            Some(last) if (k - last).abs() <= tolerance => {
                *multiplicities.last_mut().expect("knot") += 1
            }
            _ => {
                distinct.push(k);
                multiplicities.push(1);
            }
        }
    }
    let d = degree as u32;
    if flags & 2 != 0
        || multiplicities[0] != d + 1
        || *multiplicities.last().expect("knot") != d + 1
    {
        return Ok(Err((
            "dxf.spline_periodic",
            "Periodic or unclamped SPLINE entities are not supported; re-export as clamped",
        )));
    }
    if multiplicities[1..multiplicities.len() - 1]
        .iter()
        .any(|m| *m > d)
    {
        return Ok(Err((
            "dxf.spline_invalid",
            "SPLINE interior knot multiplicity exceeds its degree",
        )));
    }
    let rational = !weights.is_empty() && weights.iter().any(|w| (w - 1.).abs() > 1e-15);
    if rational && (weights.len() != poles.len() || weights.iter().any(|w| *w <= 0.)) {
        return Ok(Err((
            "dxf.spline_invalid",
            "SPLINE weights do not match its poles",
        )));
    }
    let first = distinct[0];
    Ok(Ok(Segment::Spline {
        degree: d,
        poles,
        knots: distinct.iter().map(|k| (k - first) / span).collect(),
        multiplicities,
        weights: rational.then_some(weights),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulge_of_one_is_a_counterclockwise_semicircle() {
        let Some(Segment::Arc {
            center,
            radius,
            sweep,
            ..
        }) = bulge_segment([0., 0.], [10., 0.], 1.)
        else {
            panic!()
        };
        assert!((center[0] - 5.).abs() < 1e-12 && center[1].abs() < 1e-12);
        assert!((radius - 5.).abs() < 1e-12 && (sweep - std::f64::consts::PI).abs() < 1e-12);
        let arc = bulge_segment([0., 0.], [10., 0.], -0.5).unwrap();
        let end = arc.end().unwrap();
        assert!((end[0] - 10.).abs() < 1e-12 && end[1].abs() < 1e-12);
    }
}
