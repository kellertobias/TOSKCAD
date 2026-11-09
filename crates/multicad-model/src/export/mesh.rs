//! OBJ, 3MF, STL and GLB writers over exact solids. Tessellation uses
//! the recipe's quality, independent of the viewport; display meshes are
//! never an input. 3MF packages use stored (uncompressed) ZIP entries.
use multicad_core::{
    exchange::{LengthUnit, MeshFormat, MeshQuality},
    materials::Appearance,
};
use multicad_kernel::{MeshOptions, Solid};
use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write,
};

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
    tessellate_with_connectivity(solid, quality, name, solid.uses_rust_geometry(), false)
}

pub(super) fn tessellate_recipe(
    solid: &Solid,
    quality: &MeshQuality,
    name: &str,
    format: MeshFormat,
    unit: LengthUnit,
) -> Result<MeshObject, String> {
    let object = if format == MeshFormat::ThreeMf {
        tessellate_manufacturing(solid, quality, name)?
    } else {
        tessellate(solid, quality, name)?
    };
    if solid.uses_rust_geometry() {
        match format {
            // STL objects are tessellated in world coordinates. The writer
            // receives an identity build item for each one.
            MeshFormat::Stl => {
                validate_float_positions(&object, unit, Some(&BuildItem::identity(0)))?
            }
            // glTF serializes local vertices in meters and composes each
            // placement separately in its node hierarchy.
            MeshFormat::Glb => validate_float_positions(&object, LengthUnit::Meter, None)?,
            MeshFormat::Obj | MeshFormat::ThreeMf => {}
        }
    }
    Ok(object)
}

/// Check the actual single-precision coordinate domain before writing a
/// certified mesh. Coincident source points keep separate index ownership.
pub(super) fn validate_float_positions(
    object: &MeshObject,
    unit: LengthUnit,
    placement: Option<&BuildItem>,
) -> Result<(), String> {
    let error = |reason| {
        format!(
            "Mesh object {:?}: single-precision output {reason}",
            object.name
        )
    };
    let factor = scale(unit);
    let place = |point| {
        placement
            .map_or(point, |item| item.apply(point))
            .map(|c| c * factor)
    };
    let mut points = Vec::with_capacity(object.vertices.len());
    let mut sources = HashMap::with_capacity(object.vertices.len());
    for &source in &object.vertices {
        let original = place(source);
        let encoded = original.map(|c| c as f32);
        if encoded.iter().any(|c| !c.is_finite()) {
            return Err(error("has nonfinite positions"));
        }
        let key = encoded.map(|c| if c == 0. { 0 } else { c.to_bits() });
        if *sources.entry(key).or_insert(source) != source {
            return Err(error("collapses distinct vertices"));
        }
        points.push((original, encoded.map(f64::from)));
    }
    drop(sources);
    let normal = |[a, b, c]: [[f64; 3]; 3]| {
        cross3(
            std::array::from_fn(|i| b[i] - a[i]),
            std::array::from_fn(|i| c[i] - a[i]),
        )
    };
    for triangle in &object.triangles {
        let vertices = triangle.map(|i| points[i as usize]);
        let original = normal(vertices.map(|(source, _)| source));
        let encoded = normal(vertices.map(|(_, point)| point));
        let orientation = original
            .into_iter()
            .zip(encoded)
            .map(|(a, b)| a * b)
            .sum::<f64>();
        if !orientation.is_finite() || orientation <= 0. {
            return Err(error("collapses or reverses a triangle"));
        }
    }
    Ok(())
}

/// Preserve exact indexed closure when it already succeeds. Otherwise obtain
/// shared-edge connectivity from the native solid to resolve face seams whose
/// independently evaluated coordinates differ by numerical roundoff.
pub fn tessellate_manufacturing(
    solid: &Solid,
    quality: &MeshQuality,
    name: &str,
) -> Result<MeshObject, String> {
    if solid.uses_rust_geometry() {
        return tessellate_with_connectivity(solid, quality, name, true, false);
    }
    let raw = tessellate_with_connectivity(solid, quality, name, false, true)?;
    match super::mesh_indexing::closed_indexed(&raw) {
        Ok(closed) => Ok(closed),
        Err(exact_error) => {
            let transported = tessellate_with_connectivity(solid, quality, name, true, true)
                .map_err(|native| format!("{exact_error}; native topology transport: {native}"))?;
            super::mesh_indexing::closed_indexed(&transported)
        }
    }
}

fn tessellate_with_connectivity(
    solid: &Solid,
    quality: &MeshQuality,
    name: &str,
    manufacturing: bool,
    normalize_exact_zero_triangles: bool,
) -> Result<MeshObject, String> {
    let options = MeshOptions {
        linear_deflection_mm: quality.chord_deviation_mm,
        angular_deflection_rad: quality.angular_deviation_deg.to_radians(),
        maximum_triangles: 2_000_000,
    };
    let mesh = if manufacturing {
        solid.manufacturing_mesh(options)
    } else {
        solid.mesh(options)
    }
    .map_err(|e| e.to_string())?;
    if mesh.vertices.len() % 3 != 0
        || mesh.indices.len() % 3 != 0
        || mesh.indices.is_empty()
        || mesh.face_indices.len() != mesh.indices.len() / 3
        || mesh.vertices.iter().any(|value| !value.is_finite())
    {
        return Err("Tessellation returned an invalid mesh".into());
    }
    let vertices: Vec<_> = mesh.vertices.as_chunks::<3>().0.to_vec();
    let mut triangles: Vec<_> = mesh.indices.as_chunks::<3>().0.to_vec();
    if triangles
        .iter()
        .flatten()
        .any(|i| *i as usize >= vertices.len())
    {
        return Err("Tessellation referenced a missing vertex".into());
    }
    if normalize_exact_zero_triangles {
        let original_faces: BTreeSet<_> = mesh.face_indices.iter().copied().collect();
        let mut retained_faces = BTreeSet::new();
        let original_count = triangles.len();
        let mut ordinal = 0;
        triangles.retain(|triangle| {
            let face = mesh.face_indices[ordinal];
            ordinal += 1;
            let [a, b, c] = triangle.map(|index| vertices[index as usize]);
            // Exact numeric equality includes signed zeros. Such a triangle
            // has precisely zero area and volume, without an epsilon or a
            // rounded cross-product test. This applies only to native-derived
            // manufacturing meshes; arbitrary writer input stays strict.
            if a == b || b == c || c == a {
                false
            } else {
                retained_faces.insert(face);
                true
            }
        });
        if original_faces != retained_faces || triangles.is_empty() {
            return Err(format!(
                "Removing {} exact zero-area native triangles would erase a native face",
                original_count - triangles.len()
            ));
        }
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
/// per placement, in the declared unit. Exact-coordinate indexing closes
/// face-local seams; ambiguous or open indexed topology returns an error.
pub fn model_xml(
    objects: &[MeshObject],
    items: &[BuildItem],
    unit: LengthUnit,
) -> Result<String, String> {
    let objects = objects
        .iter()
        .map(super::mesh_indexing::closed_indexed)
        .collect::<Result<Vec<_>, _>>()?;
    for item in items {
        if item.object >= objects.len()
            || item
                .rotation
                .iter()
                .flatten()
                .chain(item.translation_mm.iter())
                .any(|c| !c.is_finite())
        {
            return Err("3MF build item references an absent object or nonfinite placement".into());
        }
    }
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
    Ok(out)
}

/// A 3MF package: content types, relationships and the core model.
pub fn three_mf(
    objects: &[MeshObject],
    items: &[BuildItem],
    unit: LengthUnit,
) -> Result<Vec<u8>, String> {
    let model = model_xml(objects, items, unit)?;
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    Ok(stored_zip(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", relationships.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ]))
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

/// Binary STL of every placed item in world coordinates. STL has no unit
/// field: coordinates are written in `unit` and the 80-byte header states it.
/// Facet normals are computed from the counterclockwise vertex order.
pub fn stl(objects: &[MeshObject], items: &[BuildItem], unit: LengthUnit, title: &str) -> Vec<u8> {
    let factor = scale(unit);
    let mut header = format!(
        "MultiCAD binary STL; units={} (not stored by STL); {}",
        unit_name(unit),
        title
    )
    .into_bytes();
    header.truncate(80);
    header.resize(80, b' ');
    let count: usize = items
        .iter()
        .map(|i| objects[i.object].triangles.len())
        .sum();
    let mut out = header;
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for item in items {
        let object = &objects[item.object];
        let world: Vec<[f64; 3]> = object
            .vertices
            .iter()
            .map(|p| item.apply(*p).map(|c| c * factor))
            .collect();
        for t in &object.triangles {
            let [a, b, c] = t.map(|i| world[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = if length > 0. {
                n.map(|x| x / length)
            } else {
                [0.; 3]
            };
            for value in n.iter().chain(&a).chain(&b).chain(&c) {
                out.extend_from_slice(&(*value as f32).to_le_bytes());
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
    }
    out
}

/// One node of a GLB scene. A node with a mesh draws `objects[mesh.0]` with
/// `materials[mesh.1]`; its rigid transform (millimeters) is relative to its
/// parent, so each placement is applied exactly once along a path.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub name: String,
    pub parent: Option<usize>,
    pub mesh: Option<(usize, usize)>,
    pub rotation: [[f64; 3]; 3],
    pub translation_mm: [f64; 3],
}
/// An sRGB 8-bit channel as a linear factor.
pub fn linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
/// glTF 2.0 binary. glTF is defined in meters with +Y up: geometry and
/// translations are converted from millimeters, and one root node turns the
/// Z-up model into the Y-up scene, so importers such as Blender restore Z-up.
/// Mesh objects are shared by every node that places them.
pub fn glb(
    objects: &[MeshObject],
    materials: &[Appearance],
    nodes: &[SceneNode],
    title: &str,
) -> Vec<u8> {
    glb_scene(objects, materials, nodes, title, None)
}
/// A glTF camera placed in the Z-up model frame (millimeters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlbProjection {
    Perspective { yfov_rad: f64 },
    Orthographic { height_mm: f64 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct GlbCamera {
    pub name: String,
    pub eye_mm: [f64; 3],
    pub target_mm: [f64; 3],
    pub projection: GlbProjection,
}
/// A KHR_lights_punctual light: `intensity` is in lux for directional
/// lights and candela for point lights; `color` is linear RGB.
#[derive(Clone, Debug, PartialEq)]
pub enum GlbLight {
    Directional {
        name: String,
        direction: [f64; 3],
        color: [f64; 3],
        intensity_lux: f64,
    },
    Point {
        name: String,
        position_mm: [f64; 3],
        color: [f64; 3],
        intensity_cd: f64,
    },
}
/// Render additions to a GLB scene: a camera, lights and scene extras.
#[derive(Clone, Debug, PartialEq)]
pub struct GlbRender {
    pub camera: GlbCamera,
    pub lights: Vec<GlbLight>,
    pub scene_extras: serde_json::Value,
}
fn unit3(a: [f64; 3]) -> [f64; 3] {
    let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    a.map(|c| c / n)
}
fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// The column-major glTF matrix of a node at `at_mm` whose local −Z looks
/// along `direction`, keeping +Z (model up) as close to local +Y as possible.
fn look_matrix(at_mm: [f64; 3], direction: [f64; 3]) -> Vec<f64> {
    let d = unit3(direction);
    let mut right = cross3(d, [0., 0., 1.]);
    if right.iter().map(|c| c * c).sum::<f64>() < 1e-12 {
        right = cross3(d, [0., 1., 0.]);
    }
    let x = unit3(right);
    let y = cross3(x, d);
    let z = d.map(|c| -c);
    let t = at_mm.map(|c| c * 0.001);
    vec![
        x[0], x[1], x[2], 0., y[0], y[1], y[2], 0., z[0], z[1], z[2], 0., t[0], t[1], t[2], 1.,
    ]
}
/// glTF 2.0 binary with optional render additions.
pub fn glb_scene(
    objects: &[MeshObject],
    materials: &[Appearance],
    nodes: &[SceneNode],
    title: &str,
    render: Option<&GlbRender>,
) -> Vec<u8> {
    use serde_json::json;
    let mut bin: Vec<u8> = vec![];
    let (mut views, mut accessors, mut meshes) = (vec![], vec![], vec![]);
    let mut mesh_index: Vec<(usize, usize)> = vec![];
    let mut view = |bin: &mut Vec<u8>, bytes: Vec<u8>, target: u32| {
        let offset = bin.len();
        bin.extend_from_slice(&bytes);
        views.push(
            json!({"buffer": 0, "byteOffset": offset, "byteLength": bytes.len(), "target": target}),
        );
        views.len() - 1
    };
    for node in nodes {
        let Some(key) = node.mesh else { continue };
        if mesh_index.contains(&key) {
            continue;
        }
        let object = &objects[key.0];
        let meters: Vec<[f32; 3]> = object
            .vertices
            .iter()
            .map(|p| p.map(|c| (c * 0.001) as f32))
            .collect();
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in &meters {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        let positions = view(
            &mut bin,
            meters
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
            34962,
        );
        let indices = view(
            &mut bin,
            object
                .triangles
                .iter()
                .flatten()
                .flat_map(|i| i.to_le_bytes())
                .collect(),
            34963,
        );
        accessors.push(json!({"bufferView": positions, "componentType": 5126, "count": meters.len(), "type": "VEC3", "min": min, "max": max}));
        accessors.push(json!({"bufferView": indices, "componentType": 5125, "count": object.triangles.len() * 3, "type": "SCALAR"}));
        meshes.push(json!({"name": object.name, "primitives": [{
            "attributes": {"POSITION": accessors.len() - 2},
            "indices": accessors.len() - 1,
            "material": key.1,
            "mode": 4,
        }]}));
        mesh_index.push(key);
    }
    let materials: Vec<_> = materials
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let [r, g, b, alpha] = a.color_rgba;
            let mut m = json!({
                "name": format!("appearance-{}", i + 1),
                "pbrMetallicRoughness": {
                    "baseColorFactor": [linear(r), linear(g), linear(b), f64::from(alpha) / 255.],
                    "metallicFactor": a.metallic,
                    "roughnessFactor": a.roughness,
                },
                "doubleSided": false,
            });
            if alpha < 255 {
                m["alphaMode"] = json!("BLEND");
            }
            m
        })
        .collect();
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let mut json_nodes = vec![json!({
        "name": title,
        "rotation": [-half, 0., 0., half],
        "extras": {"units": "meter", "source_up_axis": "Z", "generator": "MultiCAD"},
    })];
    let mut children: Vec<Vec<usize>> = vec![vec![]; nodes.len() + 1];
    for (index, node) in nodes.iter().enumerate() {
        let r = node.rotation;
        let t = node.translation_mm.map(|c| c * 0.001);
        let mut value = json!({
            "name": node.name,
            "matrix": [
                r[0][0], r[1][0], r[2][0], 0.,
                r[0][1], r[1][1], r[2][1], 0.,
                r[0][2], r[1][2], r[2][2], 0.,
                t[0], t[1], t[2], 1.,
            ],
        });
        if let Some(key) = node.mesh {
            value["mesh"] = json!(mesh_index.iter().position(|k| *k == key));
        }
        json_nodes.push(value);
        children[node.parent.map_or(0, |p| p + 1)].push(index + 1);
    }
    // Cameras and lights are children of the root, so they sit in the
    // Z-up model frame like the parts.
    let (mut cameras, mut lights) = (vec![], vec![]);
    if let Some(render) = render {
        let c = &render.camera;
        cameras.push(match c.projection {
            GlbProjection::Perspective { yfov_rad } => json!({
                "name": c.name, "type": "perspective",
                "perspective": {"yfov": yfov_rad, "znear": 0.01},
            }),
            GlbProjection::Orthographic { height_mm } => json!({
                "name": c.name, "type": "orthographic",
                "orthographic": {"xmag": height_mm * 0.0005, "ymag": height_mm * 0.0005, "znear": 0.01, "zfar": 10000.0},
            }),
        });
        let view = std::array::from_fn(|i| c.target_mm[i] - c.eye_mm[i]);
        json_nodes
            .push(json!({"name": c.name, "camera": 0, "matrix": look_matrix(c.eye_mm, view)}));
        children[0].push(json_nodes.len() - 1);
        for (index, light) in render.lights.iter().enumerate() {
            let (value, matrix) = match light {
                GlbLight::Directional {
                    name,
                    direction,
                    color,
                    intensity_lux,
                } => (
                    json!({"name": name, "type": "directional", "color": color, "intensity": intensity_lux}),
                    look_matrix([0.; 3], *direction),
                ),
                GlbLight::Point {
                    name,
                    position_mm,
                    color,
                    intensity_cd,
                } => (
                    json!({"name": name, "type": "point", "color": color, "intensity": intensity_cd}),
                    look_matrix(*position_mm, [0., 0., -1.]),
                ),
            };
            let name = value["name"].clone();
            lights.push(value);
            json_nodes.push(json!({
                "name": name, "matrix": matrix,
                "extensions": {"KHR_lights_punctual": {"light": index}},
            }));
            children[0].push(json_nodes.len() - 1);
        }
    }
    for (index, list) in children.into_iter().enumerate() {
        if !list.is_empty() {
            json_nodes[index]["children"] = json!(list);
        }
    }
    let mut document = json!({
        "asset": {"version": "2.0", "generator": "MultiCAD"},
        "scene": 0,
        "scenes": [{"name": title, "nodes": [0]}],
        "nodes": json_nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": bin.len()}],
    });
    if let Some(render) = render {
        document["cameras"] = json!(cameras);
        document["scenes"][0]["extras"] = render.scene_extras.clone();
        if !lights.is_empty() {
            document["extensionsUsed"] = json!(["KHR_lights_punctual"]);
            document["extensions"] = json!({"KHR_lights_punctual": {"lights": lights}});
        }
    }
    let mut text = serde_json::to_vec(&document).expect("glTF JSON is serializable");
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
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

/// Read back an STL written by [`stl`]: the unit from its header and every
/// facet in world millimeters. STL keeps no objects, so each facet is one item.
pub fn read_stl(bytes: &[u8]) -> Result<ReadMesh, String> {
    if bytes.len() < 84 {
        return Err("STL is shorter than its header".into());
    }
    let header = String::from_utf8_lossy(&bytes[..80]);
    let unit = [
        LengthUnit::Millimeter,
        LengthUnit::Centimeter,
        LengthUnit::Meter,
        LengthUnit::Inch,
    ]
    .into_iter()
    .find(|u| header.contains(&format!("units={} ", unit_name(*u))))
    .ok_or("STL header states no unit")?;
    let count = u32::from_le_bytes(bytes[80..84].try_into().expect("four bytes")) as usize;
    if bytes.len() != 84 + count * 50 {
        return Err("STL length does not match its facet count".into());
    }
    let factor = scale(unit);
    let f = |at: usize| {
        f64::from(f32::from_le_bytes(
            bytes[at..at + 4].try_into().expect("four bytes"),
        )) / factor
    };
    let world = (0..count)
        .map(|t| {
            let at = 84 + t * 50 + 12;
            let p = |v: usize| std::array::from_fn(|i| f(at + v * 12 + i * 4));
            (p(0), p(1), p(2))
        })
        .collect();
    Ok(ReadMesh {
        unit_scale: factor,
        objects: 0,
        items: 0,
        triangles: count,
        world,
    })
}
/// Read back a GLB written by [`glb`]: compose node transforms from the scene
/// root and return every drawn triangle in Z-up world millimeters.
pub fn read_glb(bytes: &[u8]) -> Result<ReadMesh, String> {
    use serde_json::Value;
    let word = |at: usize| -> Result<usize, String> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("four bytes")) as usize)
            .ok_or_else(|| "GLB is truncated".to_string())
    };
    if bytes.get(..4) != Some(b"glTF") || word(4)? != 2 || word(8)? != bytes.len() {
        return Err("Not a glTF 2.0 binary".into());
    }
    let json_len = word(12)?;
    if bytes.get(16..20) != Some(b"JSON") {
        return Err("GLB has no JSON chunk".into());
    }
    let json: Value = serde_json::from_slice(
        bytes
            .get(20..20 + json_len)
            .ok_or("GLB JSON is truncated")?,
    )
    .map_err(|e| e.to_string())?;
    let at = 20 + json_len;
    let bin_len = word(at)?;
    if bytes.get(at + 4..at + 8) != Some(b"BIN\0") {
        return Err("GLB has no binary chunk".into());
    }
    let bin = bytes
        .get(at + 8..at + 8 + bin_len)
        .ok_or("GLB binary is truncated")?;
    let number = |v: &Value| v.as_u64().map(|n| n as usize).ok_or("Invalid glTF index");
    let accessor = |index: usize| -> Result<Vec<f64>, String> {
        let a = &json["accessors"][index];
        let view = &json["bufferViews"][number(&a["bufferView"])?];
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let width = if a["type"] == "VEC3" { 3 } else { 1 };
        let count = number(&a["count"])? * width;
        let float = a["componentType"] == 5126;
        (0..count)
            .map(|i| {
                let raw: [u8; 4] = bin
                    .get(offset + i * 4..offset + i * 4 + 4)
                    .ok_or("glTF accessor exceeds its buffer")?
                    .try_into()
                    .expect("four bytes");
                Ok(if float {
                    f64::from(f32::from_le_bytes(raw))
                } else {
                    f64::from(u32::from_le_bytes(raw))
                })
            })
            .collect()
    };
    type M = [[f64; 4]; 4];
    let local = |node: &Value| -> M {
        if let Some(m) = node["matrix"].as_array() {
            let m: Vec<f64> = m.iter().map(|v| v.as_f64().unwrap_or(0.)).collect();
            std::array::from_fn(|i| std::array::from_fn(|j| m[j * 4 + i]))
        } else {
            let q: Vec<f64> = node["rotation"]
                .as_array()
                .map(|r| r.iter().map(|v| v.as_f64().unwrap_or(0.)).collect())
                .unwrap_or_else(|| vec![0., 0., 0., 1.]);
            let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
            [
                [
                    1. - 2. * (y * y + z * z),
                    2. * (x * y - z * w),
                    2. * (x * z + y * w),
                    0.,
                ],
                [
                    2. * (x * y + z * w),
                    1. - 2. * (x * x + z * z),
                    2. * (y * z - x * w),
                    0.,
                ],
                [
                    2. * (x * z - y * w),
                    2. * (y * z + x * w),
                    1. - 2. * (x * x + y * y),
                    0.,
                ],
                [0., 0., 0., 1.],
            ]
        }
    };
    let identity: M = std::array::from_fn(|i| std::array::from_fn(|j| f64::from(i == j)));
    let mut stack = vec![];
    for n in json["scenes"][0]["nodes"]
        .as_array()
        .ok_or("glTF scene has no nodes")?
    {
        stack.push((number(n)?, identity, 0usize));
    }
    let (mut world, mut items) = (vec![], 0);
    while let Some((index, parent, depth)) = stack.pop() {
        if depth > 64 {
            return Err("glTF node hierarchy is too deep".into());
        }
        let node = &json["nodes"][index];
        let l = local(node);
        let m: M = std::array::from_fn(|i| {
            std::array::from_fn(|j| (0..4).map(|k| parent[i][k] * l[k][j]).sum())
        });
        if let Some(mesh) = node["mesh"].as_u64() {
            items += 1;
            let primitive = &json["meshes"][mesh as usize]["primitives"][0];
            let p = accessor(number(&primitive["attributes"]["POSITION"])?)?;
            let indices = accessor(number(&primitive["indices"])?)?;
            for t in indices.chunks(3) {
                let point = |v: usize| -> Result<[f64; 3], String> {
                    let at = t[v] as usize * 3;
                    let c = p
                        .get(at..at + 3)
                        .ok_or("glTF index exceeds its positions")?;
                    let q: [f64; 3] = std::array::from_fn(|r| {
                        (0..3).map(|k| m[r][k] * c[k]).sum::<f64>() + m[r][3]
                    });
                    Ok([q[0] * 1000., -q[2] * 1000., q[1] * 1000.])
                };
                world.push((point(0)?, point(1)?, point(2)?));
            }
        }
        for child in node["children"].as_array().into_iter().flatten() {
            stack.push((number(child)?, m, depth + 1));
        }
    }
    Ok(ReadMesh {
        unit_scale: 0.001,
        objects: json["meshes"].as_array().map_or(0, Vec::len),
        items,
        triangles: world.len(),
        world,
    })
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    #[test]
    fn float_positions_refuse_flattened_or_reversed_triangles_with_distinct_vertices() {
        let x = 8_388_608.;
        for (b, c) in [(0.1, 0.), (0.49, 0.51)] {
            let object = MeshObject {
                name: "Thin triangle".into(),
                vertices: vec![[x, 0., 0.], [x + b, 1., 0.], [x + c, 2., 0.]],
                triangles: vec![[0, 1, 2]],
            };
            let error =
                validate_float_positions(&object, LengthUnit::Millimeter, None).unwrap_err();
            assert!(
                error.contains("collapses or reverses a triangle"),
                "{error}"
            );
        }
    }

    #[test]
    fn float_positions_allow_index_separated_coincident_source_points() {
        let object = MeshObject {
            name: "Coincident source corners".into(),
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [-0., 0., 0.],
                [-1., 0., 0.],
                [0., -1., 0.],
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
        };
        validate_float_positions(&object, LengthUnit::Millimeter, None).unwrap();
    }
}
