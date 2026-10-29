//! OBJ and 3MF writers over exact native solids. Tessellation uses the
//! recipe's quality, independent of the viewport; display meshes are never
//! an input. 3MF packages use stored (uncompressed) ZIP entries.
use multicad_core::exchange::{LengthUnit, MeshQuality};
use multicad_kernel::{MeshOptions, Solid};
use std::fmt::Write;

/// One mesh in millimeters. For 3MF it is a resource object; for OBJ it is
/// written in world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshObject {
    pub name: String,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}
/// A 3MF build item: an object placed by a rigid transform in millimeters
/// (column-vector rotation and translation).
#[derive(Clone, Debug, PartialEq)]
pub struct BuildItem {
    pub object: usize,
    pub rotation: [[f64; 3]; 3],
    pub translation_mm: [f64; 3],
}
impl BuildItem {
    pub fn identity(object: usize) -> Self {
        Self {
            object,
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [0.; 3],
        }
    }
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| {
            (0..3).map(|j| self.rotation[i][j] * p[j]).sum::<f64>() + self.translation_mm[i]
        })
    }
}

pub fn scale(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Millimeter => 1.,
        LengthUnit::Centimeter => 0.1,
        LengthUnit::Meter => 0.001,
        LengthUnit::Inch => 1. / 25.4,
    }
}
fn unit_name(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Millimeter => "millimeter",
        LengthUnit::Centimeter => "centimeter",
        LengthUnit::Meter => "meter",
        LengthUnit::Inch => "inch",
    }
}

/// Tessellate an exact solid at the recipe's chord and angular deviation.
pub fn tessellate(solid: &Solid, quality: &MeshQuality, name: &str) -> Result<MeshObject, String> {
    let mesh = solid
        .mesh(MeshOptions {
            linear_deflection_mm: quality.chord_deviation_mm,
            angular_deflection_rad: quality.angular_deviation_deg.to_radians(),
            maximum_triangles: 2_000_000,
        })
        .map_err(|e| e.to_string())?;
    if mesh.vertices.len() % 3 != 0 || mesh.indices.len() % 3 != 0 || mesh.indices.is_empty() {
        return Err("Native tessellation returned an invalid mesh".into());
    }
    let vertices: Vec<_> = mesh.vertices.as_chunks::<3>().0.to_vec();
    let triangles: Vec<_> = mesh.indices.as_chunks::<3>().0.to_vec();
    if triangles
        .iter()
        .flatten()
        .any(|i| *i as usize >= vertices.len())
    {
        return Err("Native tessellation referenced a missing vertex".into());
    }
    Ok(MeshObject {
        name: name.into(),
        vertices,
        triangles,
    })
}

/// Enclosed volume by the divergence theorem, in cubic millimeters.
pub fn volume(vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> f64 {
    triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.
        })
        .sum::<f64>()
        .abs()
}

fn number(value: f64) -> String {
    // Shortest round-trip representation; never exponent-free garbage.
    let v = if value == 0. { 0. } else { value };
    format!("{v}")
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn obj_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    if cleaned.is_empty() {
        "part".into()
    } else {
        cleaned
    }
}

/// Wavefront OBJ with one named object per mesh, in world coordinates. OBJ
/// has no unit field; the unit is stated in a header comment and applied to
/// the coordinates.
pub fn obj(objects: &[MeshObject], unit: LengthUnit) -> Vec<u8> {
    let s = scale(unit);
    let mut out = String::new();
    let _ = writeln!(out, "# MultiCAD OBJ export");
    let _ = writeln!(out, "# units: {}", unit_name(unit));
    let mut base = 1usize;
    for object in objects {
        let _ = writeln!(out, "o {}", obj_name(&object.name));
        for v in &object.vertices {
            let _ = writeln!(
                out,
                "v {} {} {}",
                number(v[0] * s),
                number(v[1] * s),
                number(v[2] * s)
            );
        }
        for t in &object.triangles {
            let _ = writeln!(
                out,
                "f {} {} {}",
                base + t[0] as usize,
                base + t[1] as usize,
                base + t[2] as usize
            );
        }
        base += object.vertices.len();
    }
    out.into_bytes()
}

/// The 3MF core model XML: one resource object per mesh and one build item
/// per placement, in the declared unit.
pub fn model_xml(objects: &[MeshObject], items: &[BuildItem], unit: LengthUnit) -> String {
    let s = scale(unit);
    let mut out = String::new();
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        out,
        r#"<model unit="{}" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">"#,
        unit_name(unit)
    );
    let _ = writeln!(out, r#"<metadata name="Application">MultiCAD</metadata>"#);
    let _ = writeln!(out, "<resources>");
    for (index, object) in objects.iter().enumerate() {
        let _ = writeln!(
            out,
            r#"<object id="{}" type="model" name="{}"><mesh><vertices>"#,
            index + 1,
            escape(&object.name)
        );
        for v in &object.vertices {
            let _ = writeln!(
                out,
                r#"<vertex x="{}" y="{}" z="{}"/>"#,
                number(v[0] * s),
                number(v[1] * s),
                number(v[2] * s)
            );
        }
        let _ = writeln!(out, "</vertices><triangles>");
        for t in &object.triangles {
            let _ = writeln!(
                out,
                r#"<triangle v1="{}" v2="{}" v3="{}"/>"#,
                t[0], t[1], t[2]
            );
        }
        let _ = writeln!(out, "</triangles></mesh></object>");
    }
    let _ = writeln!(out, "</resources>");
    let _ = writeln!(out, "<build>");
    for item in items {
        let r = item.rotation;
        let t = item.translation_mm.map(|v| v * s);
        // 3MF rows multiply row vectors: m_ij = R_ji, translation last.
        let values = [
            r[0][0], r[1][0], r[2][0], r[0][1], r[1][1], r[2][1], r[0][2], r[1][2], r[2][2], t[0],
            t[1], t[2],
        ]
        .map(number)
        .join(" ");
        let _ = writeln!(
            out,
            r#"<item objectid="{}" transform="{values}"/>"#,
            item.object + 1
        );
    }
    let _ = writeln!(out, "</build>");
    let _ = writeln!(out, "</model>");
    out
}

/// A 3MF package: content types, relationships and the core model.
pub fn three_mf(objects: &[MeshObject], items: &[BuildItem], unit: LengthUnit) -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    stored_zip(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", relationships.as_bytes()),
        (
            "3D/3dmodel.model",
            model_xml(objects, items, unit).as_bytes(),
        ),
    ])
}

/// A ZIP archive with stored entries and a fixed timestamp, so identical
/// content produces identical bytes.
fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    // 1980-01-01 00:00, the earliest DOS timestamp.
    let (time, date) = (0u16, (1u16 << 5) | 1);
    for (name, data) in entries {
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let header = |signature: u32, central: bool| {
            let mut h = Vec::new();
            h.extend(signature.to_le_bytes());
            if central {
                h.extend(20u16.to_le_bytes()); // version made by
            }
            h.extend(20u16.to_le_bytes()); // version needed
            h.extend(0u16.to_le_bytes()); // flags
            h.extend(0u16.to_le_bytes()); // stored
            h.extend(time.to_le_bytes());
            h.extend(date.to_le_bytes());
            h.extend(crc.to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend(0u16.to_le_bytes()); // comment
                h.extend(0u16.to_le_bytes()); // disk
                h.extend(0u16.to_le_bytes()); // internal attributes
                h.extend(0u32.to_le_bytes()); // external attributes
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(*data);
        central.extend(header(0x0201_4b50, true));
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// What a written mesh file contains, recovered from its bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadMesh {
    pub unit_scale: f64,
    pub objects: usize,
    pub items: usize,
    pub triangles: usize,
    /// Placed world vertices and triangles, in millimeters.
    pub world: Vec<([f64; 3], [f64; 3], [f64; 3])>,
}
impl ReadMesh {
    pub fn volume_mm3(&self) -> f64 {
        self.world
            .iter()
            .map(|(a, b, c)| {
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.
            })
            .sum::<f64>()
            .abs()
    }
    pub fn bounds_mm(&self) -> Option<[[f64; 3]; 2]> {
        let mut b = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for p in self.world.iter().flat_map(|(a, b, c)| [a, b, c]) {
            for i in 0..3 {
                b[0][i] = b[0][i].min(p[i]);
                b[1][i] = b[1][i].max(p[i]);
            }
        }
        b[0][0].is_finite().then_some(b)
    }
}

/// Read back an OBJ written by [`obj`].
pub fn read_obj(bytes: &[u8]) -> Result<ReadMesh, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut unit = None;
    let (mut vertices, mut objects, mut world) = (vec![], 0, vec![]);
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("#") if line.starts_with("# units: ") => {
                unit = Some(match &line[9..] {
                    "millimeter" => 1.,
                    "centimeter" => 10.,
                    "meter" => 1000.,
                    "inch" => 25.4,
                    other => return Err(format!("Unknown OBJ unit {other}")),
                });
            }
            Some("o") => objects += 1,
            Some("v") => {
                let v: Vec<f64> = parts
                    .map(|p| p.parse::<f64>().map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?;
                let [x, y, z] = v[..] else {
                    return Err("OBJ vertex needs three coordinates".into());
                };
                vertices.push([x, y, z]);
            }
            Some("f") => {
                let f: Vec<usize> = parts
                    .map(|p| p.parse::<usize>().map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?;
                let [a, b, c] = f[..] else {
                    return Err("OBJ face must be a triangle".into());
                };
                let get = |i: usize| {
                    vertices
                        .get(i.wrapping_sub(1))
                        .copied()
                        .ok_or("OBJ face index out of range")
                };
                world.push((get(a)?, get(b)?, get(c)?));
            }
            _ => {}
        }
    }
    let unit = unit.ok_or("OBJ has no unit header")?;
    let scale = |p: [f64; 3]| p.map(|v| v * unit);
    Ok(ReadMesh {
        unit_scale: unit,
        objects,
        items: objects,
        triangles: world.len(),
        world: world
            .into_iter()
            .map(|(a, b, c)| (scale(a), scale(b), scale(c)))
            .collect(),
    })
}

/// Entries of a stored ZIP archive, checking each CRC.
pub fn read_stored_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            bytes
                .get(i..i + 2)
                .ok_or("Truncated ZIP")?
                .try_into()
                .unwrap(),
        ))
    };
    let u32_at = |i: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes
                .get(i..i + 4)
                .ok_or("Truncated ZIP")?
                .try_into()
                .unwrap(),
        ))
    };
    let mut entries = vec![];
    let mut at = 0;
    while u32_at(at)? == 0x0403_4b50 {
        if u16_at(at + 8)? != 0 {
            return Err("Only stored ZIP entries are written".into());
        }
        let crc = u32_at(at + 14)?;
        let size = u32_at(at + 18)? as usize;
        let name_len = u16_at(at + 26)? as usize;
        let extra = u16_at(at + 28)? as usize;
        let name = String::from_utf8(
            bytes
                .get(at + 30..at + 30 + name_len)
                .ok_or("Truncated ZIP")?
                .to_vec(),
        )
        .map_err(|e| e.to_string())?;
        let start = at + 30 + name_len + extra;
        let data = bytes
            .get(start..start + size)
            .ok_or("Truncated ZIP")?
            .to_vec();
        if crc32fast::hash(&data) != crc {
            return Err(format!("ZIP entry {name} fails its CRC"));
        }
        entries.push((name, data));
        at = start + size;
    }
    Ok(entries)
}

/// Read back a 3MF written by [`three_mf`], placing every build item.
pub fn read_three_mf(bytes: &[u8]) -> Result<ReadMesh, String> {
    let entries = read_stored_zip(bytes)?;
    let model = entries
        .iter()
        .find(|(name, _)| name == "3D/3dmodel.model")
        .ok_or("3MF package has no 3D model part")?;
    let text = std::str::from_utf8(&model.1).map_err(|e| e.to_string())?;
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!(" {name}=\"");
        tag.find(&key).map(|i| {
            let rest = &tag[i + key.len()..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_owned()
        })
    };
    let unit = match attr(
        &text[..text
            .find("<resources")
            .ok_or("3MF model has no resources")?],
        "unit",
    )
    .as_deref()
    {
        Some("millimeter") => 1.,
        Some("centimeter") => 10.,
        Some("meter") => 1000.,
        Some("inch") => 25.4,
        other => return Err(format!("Unsupported 3MF unit {other:?}")),
    };
    type Object = (Vec<[f64; 3]>, Vec<[usize; 3]>);
    let mut objects: Vec<Object> = vec![];
    let mut items = vec![];
    for tag in text.split('<').filter(|t| !t.is_empty()) {
        let f = |name: &str| -> Result<f64, String> {
            attr(tag, name)
                .ok_or(format!("Missing {name}"))?
                .parse::<f64>()
                .map_err(|e| e.to_string())
        };
        if tag.starts_with("object ") {
            objects.push((vec![], vec![]));
        } else if tag.starts_with("vertex ") {
            let object = objects.last_mut().ok_or("Vertex outside an object")?;
            object.0.push([f("x")?, f("y")?, f("z")?]);
        } else if tag.starts_with("triangle ") {
            let object = objects.last_mut().ok_or("Triangle outside an object")?;
            object
                .1
                .push([f("v1")? as usize, f("v2")? as usize, f("v3")? as usize]);
        } else if tag.starts_with("item ") {
            let id = f("objectid")? as usize;
            let m: Vec<f64> = attr(tag, "transform")
                .unwrap_or_else(|| "1 0 0 0 1 0 0 0 1 0 0 0".into())
                .split_whitespace()
                .map(|v| v.parse::<f64>().map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?;
            if m.len() != 12 || id == 0 {
                return Err("Invalid 3MF build item".into());
            }
            items.push((id - 1, m));
        }
    }
    let mut world = vec![];
    for (id, m) in &items {
        let (vertices, triangles) = objects
            .get(*id)
            .ok_or("Build item names a missing object")?;
        let place = |p: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|j| {
                (p[0] * m[j] + p[1] * m[3 + j] + p[2] * m[6 + j] + m[9 + j]) * unit
            })
        };
        for t in triangles {
            let get = |i: usize| {
                vertices
                    .get(i)
                    .copied()
                    .ok_or("Triangle index out of range")
            };
            world.push((place(get(t[0])?), place(get(t[1])?), place(get(t[2])?)));
        }
    }
    Ok(ReadMesh {
        unit_scale: unit,
        objects: objects.len(),
        items: items.len(),
        triangles: world.len(),
        world,
    })
}
