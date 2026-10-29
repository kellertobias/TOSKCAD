//! Exact planar manufacturing DXF. Circular arcs remain analytic bulges;
//! display tessellation and drawing scales are not inputs.
use multicad_core::{
    exchange::{Layers, LengthUnit},
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
};
use std::fmt::Write;

pub const MAX_DXF_BYTES: usize = 16 * 1024 * 1024;
pub fn unit_scale(unit: LengthUnit) -> (f64, u16) {
    match unit {
        LengthUnit::Millimeter => (1., 4),
        LengthUnit::Centimeter => (0.1, 5),
        LengthUnit::Meter => (0.001, 6),
        LengthUnit::Inch => (1. / 25.4, 1),
    }
}
pub fn layer_names(layers: &Layers) -> Result<[String; 4], String> {
    let names = [
        layers.cut.clone(),
        format!("{}_INNER", layers.cut),
        layers.construction.clone(),
        layers.annotation.clone(),
    ];
    if names.iter().any(|s| {
        s.trim().is_empty()
            || s == "0"
            || s.len() > 255
            || s.chars()
                .any(|c| c.is_control() || "<>/\\\":;?*|=,".contains(c))
    }) || names
        .iter()
        .map(|s| s.to_uppercase())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != 4
    {
        return Err(
            "DXF outer, inner, center and annotation layers must have distinct valid names".into(),
        );
    }
    Ok(names)
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Vertex {
    pub(crate) p: [f64; 2],
    pub(crate) bulge: f64,
}
pub(crate) fn vertices(wire: &PlanarLoop) -> Vec<Vertex> {
    let mut result = vec![];
    for edge in &wire.edges {
        match *edge {
            PlanarEdge::Line { a, .. } => result.push(Vertex { p: a, bulge: 0. }),
            PlanarEdge::Arc {
                a,
                center,
                sweep_rad,
                ..
            } => {
                if sweep_rad.abs() > std::f64::consts::PI + 1e-12 {
                    let half = sweep_rad * 0.5;
                    let (sin, cos) = half.sin_cos();
                    let d = [a[0] - center[0], a[1] - center[1]];
                    let mid = if (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-10 {
                        [2. * center[0] - a[0], 2. * center[1] - a[1]]
                    } else {
                        [
                            center[0] + d[0] * cos - d[1] * sin,
                            center[1] + d[0] * sin + d[1] * cos,
                        ]
                    };
                    let bulge = (half * 0.25).tan();
                    result.push(Vertex { p: a, bulge });
                    result.push(Vertex { p: mid, bulge });
                } else {
                    result.push(Vertex {
                        p: a,
                        bulge: (sweep_rad * 0.25).tan(),
                    });
                }
            }
        }
    }
    result
}
fn circular_center(wire: &PlanarLoop, tolerance: f64) -> Option<[f64; 2]> {
    if wire.outer {
        return None;
    }
    let PlanarEdge::Arc { a, center, .. } = *wire.edges.first()? else {
        return None;
    };
    let radius = (a[0] - center[0]).hypot(a[1] - center[1]);
    let mut sweep = 0.;
    for edge in &wire.edges {
        let PlanarEdge::Arc {
            a,
            center: c,
            sweep_rad,
            ..
        } = *edge
        else {
            return None;
        };
        if (c[0] - center[0]).hypot(c[1] - center[1]) > tolerance
            || ((a[0] - c[0]).hypot(a[1] - c[1]) - radius).abs() > tolerance
        {
            return None;
        }
        sweep += sweep_rad;
    }
    ((sweep.abs() - std::f64::consts::TAU).abs() < 1e-9).then_some(center)
}
pub(crate) struct Writer {
    pub(crate) text: String,
    pub(crate) handle: u32,
}
impl Writer {
    pub(crate) fn pair(&mut self, code: u16, value: impl std::fmt::Display) {
        writeln!(self.text, "{code}\n{value}").expect("String write");
    }
    pub(crate) fn entity(&mut self, kind: &str, layer: &str, class: &str) {
        self.pair(0, kind);
        self.pair(5, format!("{:X}", self.handle));
        self.handle += 1;
        self.pair(100, "AcDbEntity");
        self.pair(8, layer);
        self.pair(100, class);
    }
}
pub fn encode(
    contours: &PlanarContours,
    units: LengthUnit,
    layers: &Layers,
    label: &str,
    tolerance: f64,
) -> Result<Vec<u8>, String> {
    contours.validate(tolerance)?;
    let names = layer_names(layers)?;
    if label.len() > 512 || label.chars().any(char::is_control) {
        return Err("DXF label must be bounded single-line text".into());
    }
    let (scale, unit) = unit_scale(units);
    let mut w = Writer {
        text: String::new(),
        handle: 0x100,
    };
    w.pair(0, "SECTION");
    w.pair(2, "HEADER");
    w.pair(9, "$ACADVER");
    w.pair(1, "AC1021");
    w.pair(9, "$INSUNITS");
    w.pair(70, unit);
    w.pair(9, "$MEASUREMENT");
    w.pair(70, if units == LengthUnit::Inch { 0 } else { 1 });
    w.pair(9, "$LUNITS");
    w.pair(70, 2);
    w.pair(9, "$LUPREC");
    w.pair(70, 9);
    w.pair(0, "ENDSEC");
    w.pair(0, "SECTION");
    w.pair(2, "TABLES");
    w.pair(0, "TABLE");
    w.pair(2, "LTYPE");
    w.pair(70, 1);
    w.pair(0, "LTYPE");
    w.pair(2, "CONTINUOUS");
    w.pair(70, 0);
    w.pair(3, "Solid line");
    w.pair(72, 65);
    w.pair(73, 0);
    w.pair(40, 0);
    w.pair(0, "ENDTAB");
    w.pair(0, "TABLE");
    w.pair(2, "LAYER");
    w.pair(70, 5);
    for (i, name) in std::iter::once("0")
        .chain(names.iter().map(String::as_str))
        .enumerate()
    {
        w.pair(0, "LAYER");
        w.pair(100, "AcDbSymbolTableRecord");
        w.pair(100, "AcDbLayerTableRecord");
        w.pair(2, name);
        w.pair(70, 0);
        w.pair(62, [7, 1, 3, 5, 7][i]);
        w.pair(6, "CONTINUOUS");
    }
    w.pair(0, "ENDTAB");
    w.pair(0, "ENDSEC");
    w.pair(0, "SECTION");
    w.pair(2, "BLOCKS");
    w.pair(0, "ENDSEC");
    w.pair(0, "SECTION");
    w.pair(2, "ENTITIES");
    for wire in &contours.loops {
        let vertices = vertices(wire);
        let layer = &names[usize::from(!wire.outer)];
        w.entity("LWPOLYLINE", layer, "AcDbPolyline");
        w.pair(90, vertices.len());
        w.pair(70, 1);
        for vertex in vertices {
            w.pair(10, vertex.p[0] * scale);
            w.pair(20, vertex.p[1] * scale);
            w.pair(42, vertex.bulge);
        }
        if let Some(c) = circular_center(wire, tolerance) {
            w.entity("POINT", &names[2], "AcDbPoint");
            w.pair(10, c[0] * scale);
            w.pair(20, c[1] * scale);
            w.pair(30, 0);
        }
    }
    if !label.is_empty() {
        let [min, _] = contours
            .bounds_mm()
            .ok_or("Missing finite contour bounds")?;
        w.entity("TEXT", &names[3], "AcDbText");
        w.pair(10, min[0] * scale);
        w.pair(20, (min[1] - 4.) * scale);
        w.pair(30, 0);
        w.pair(40, 2.5 * scale);
        w.pair(1, label);
        w.pair(50, 0);
        w.pair(7, "STANDARD");
    }
    w.pair(0, "ENDSEC");
    w.pair(0, "EOF");
    if w.text.len() > MAX_DXF_BYTES {
        return Err("DXF output exceeds its bounded byte budget".into());
    }
    Ok(w.text.into_bytes())
}
/// Read the actual written file's record structure before publishing. The
/// expected byte comparison also checks every exact analytic coordinate, bulge,
/// layer and label. Independent reader tests additionally verify interoperability.
pub fn verify(bytes: &[u8], expected: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_DXF_BYTES || bytes != expected {
        return Err("DXF readback differs from the current exact contour output".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "DXF is not valid UTF-8")?;
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() % 2 != 0 {
        return Err("DXF has an incomplete group-code pair".into());
    }
    let pairs = lines
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| p[0].parse::<u16>().map(|code| (code, p[1])))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "DXF has an invalid group code")?;
    if pairs.last() != Some(&(0, "EOF")) || !pairs.contains(&(0, "LWPOLYLINE")) {
        return Err("DXF has no complete closed contour section".into());
    }
    let mut in_poly = false;
    let mut count = 0;
    let mut closed = false;
    let mut x = 0;
    for (code, value) in pairs {
        if code == 0 {
            if in_poly && (!closed || x != count || count < 2) {
                return Err("DXF polyline closure or vertex count is invalid".into());
            }
            in_poly = value == "LWPOLYLINE";
            count = 0;
            closed = false;
            x = 0;
        }
        if in_poly {
            match code {
                90 => count = value.parse().map_err(|_| "Invalid DXF vertex count")?,
                70 => closed = value == "1",
                10 => x += 1,
                _ => {}
            }
        }
    }
    Ok(())
}
