//! Shared developer renderer for real native Library geometry.
use multicad_core::{
    document::{Document, EntityId},
    project_file,
};
use multicad_model::Model;
type Error = Box<dyn std::error::Error>;
/// Reuse the native-mesh renderer with an explicit display palette.
pub fn render_preview_with_color(
    document: &Document,
    model: &Model,
    spread: bool,
    color_for: impl Fn(&str) -> [f32; 3],
) -> Result<project_file::info::PreviewImage, Error> {
    use multicad_model::export::mesh::tessellate;
    use multicad_ui::egui::{Color32, ColorImage};
    let (w, h) = (480usize, 640usize);
    // View basis: looking from the front left, slightly above.
    let yaw: f64 = (-35f64).to_radians();
    let pitch: f64 = 20f64.to_radians();
    let right = [yaw.cos(), yaw.sin(), 0.];
    let fwd = [
        -yaw.sin() * pitch.cos(),
        yaw.cos() * pitch.cos(),
        -pitch.sin(),
    ];
    let up = [
        right[1] * fwd[2] - right[2] * fwd[1],
        right[2] * fwd[0] - right[0] * fwd[2],
        right[0] * fwd[1] - right[1] * fwd[0],
    ];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let light = [-0.4, -0.7, 0.6];
    let ln = dot(light, light).sqrt();
    let mut tris = vec![];
    let mut roots: Vec<EntityId> = vec![];
    for (path, leaf) in &model.snapshot().leaf_occurrences {
        let index = roots
            .iter()
            .position(|r| *r == path.root)
            .unwrap_or_else(|| {
                roots.push(path.root);
                roots.len() - 1
            });
        let dx = if spread { index as f64 * 60. } else { 0. };
        let Some(solid) = model.leaf_solid(path) else {
            continue;
        };
        let name = document
            .get(leaf.component)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let mesh = tessellate(solid, &Default::default(), &name)?;
        let color = color_for(&name);
        for t in &mesh.triangles {
            let p = t.map(|i| {
                let v = mesh.vertices[i as usize];
                [v[0] + dx, v[1], v[2]]
            });
            tris.push((p, color));
        }
    }
    let project = |p: [f64; 3]| (dot(p, right), dot(p, up), dot(p, fwd));
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (p, _) in &tris {
        for q in p {
            let (x, y, _) = project(*q);
            min = [min[0].min(x), min[1].min(y)];
            max = [max[0].max(x), max[1].max(y)];
        }
    }
    let scale = ((w as f64 - 40.) / (max[0] - min[0])).min((h as f64 - 40.) / (max[1] - min[1]));
    let ox = (w as f64 - (max[0] - min[0]) * scale) / 2.;
    let oy = (h as f64 - (max[1] - min[1]) * scale) / 2.;
    let mut depth = vec![f64::MAX; w * h];
    let mut pixels = vec![Color32::from_rgb(244, 244, 242); w * h];
    for (p, color) in &tris {
        let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let b = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let nl = dot(n, n).sqrt();
        if nl < 1e-12 {
            continue;
        }
        let shade = 0.45 + 0.55 * (dot(n, light) / (nl * ln)).abs();
        let rgb = color.map(|c| (c as f64 * shade * 255.).clamp(0., 255.) as u8);
        let v = p.map(|q| {
            let (x, y, z) = project(q);
            (
                ox + (x - min[0]) * scale,
                h as f64 - (oy + (y - min[1]) * scale),
                z,
            )
        });
        let x0 = v
            .iter()
            .map(|q| q.0)
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let x1 = v
            .iter()
            .map(|q| q.0)
            .fold(f64::MIN, f64::max)
            .ceil()
            .min(w as f64 - 1.) as usize;
        let y0 = v
            .iter()
            .map(|q| q.1)
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let y1 = v
            .iter()
            .map(|q| q.1)
            .fold(f64::MIN, f64::max)
            .ceil()
            .min(h as f64 - 1.) as usize;
        let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((v[1].0 - px) * (v[2].1 - py) - (v[2].0 - px) * (v[1].1 - py)) / area;
                let w1 = ((v[2].0 - px) * (v[0].1 - py) - (v[0].0 - px) * (v[2].1 - py)) / area;
                let w2 = 1. - w0 - w1;
                if w0 < 0. || w1 < 0. || w2 < 0. {
                    continue;
                }
                let z = w0 * v[0].2 + w1 * v[1].2 + w2 * v[2].2;
                let i = y * w + x;
                if z < depth[i] {
                    depth[i] = z;
                    pixels[i] = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                }
            }
        }
    }
    let image = ColorImage::new([w, h], pixels);
    Ok(multicad_ui::preview_capture::preview_from(&image)?)
}
