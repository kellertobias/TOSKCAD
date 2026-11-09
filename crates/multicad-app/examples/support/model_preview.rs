use multicad_core::document::{Document, EntityData, EntityId};
use multicad_kernel::{MeshOptions, Solid};
use multicad_ui::egui::{Color32, ColorImage};

type Error = Box<dyn std::error::Error>;

// Preview: a small headless software rasterizer over the native solids, so
// the shipped file carries a picture of its real geometry without a GPU.
const PREVIEW: [usize; 2] = [1280, 960];

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l > 0. { a.map(|c| c / l) } else { a }
}

/// Render the given Parts in an orthographic isometric view from the front
/// left, each in its material color with simple directional shading.
pub fn render_preview<'a>(
    document: &Document,
    parts: impl IntoIterator<Item = (EntityId, &'a Solid)>,
) -> Result<ColorImage, Error> {
    // View axes: right, up and toward the viewer.
    let toward = normalize([-1., -1.6, 1.1]);
    let right = normalize(cross([0., 0., 1.], toward));
    let up = cross(toward, right);
    let light = normalize([-0.4, -0.8, 1.]);
    let mut triangles: Vec<([[f64; 3]; 3], [u8; 4])> = vec![];
    for (part, solid) in parts {
        let color = match document.get(part).map(|e| &e.data) {
            Some(EntityData::Part {
                material: Some(m), ..
            }) => match document.get(*m).map(|e| &e.data) {
                Some(EntityData::Material { color_rgba, .. }) => *color_rgba,
                _ => [160, 160, 160, 255],
            },
            _ => [160, 160, 160, 255],
        };
        let mesh = solid.mesh(MeshOptions {
            linear_deflection_mm: 0.5,
            angular_deflection_rad: 0.3,
            maximum_triangles: 2_000_000,
        })?;
        let v = |i: u32| {
            let i = i as usize * 3;
            [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
        };
        for t in mesh.indices.as_chunks::<3>().0 {
            triangles.push(([v(t[0]), v(t[1]), v(t[2])], color));
        }
    }
    if triangles.is_empty() {
        return Err("Nothing to render".into());
    }
    let project = |p: [f64; 3]| [dot(p, right), dot(p, up), dot(p, toward)];
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (t, _) in &triangles {
        for p in t {
            let q = project(*p);
            for i in 0..2 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
    }
    let [w, h] = PREVIEW;
    let margin = 0.06;
    let scale = ((w as f64) * (1. - 2. * margin) / (hi[0] - lo[0]))
        .min((h as f64) * (1. - 2. * margin) / (hi[1] - lo[1]));
    let offset = [
        (w as f64 - (hi[0] - lo[0]) * scale) / 2.,
        (h as f64 - (hi[1] - lo[1]) * scale) / 2.,
    ];
    let screen = |p: [f64; 3]| {
        let q = project(p);
        [
            offset[0] + (q[0] - lo[0]) * scale,
            h as f64 - (offset[1] + (q[1] - lo[1]) * scale),
            q[2],
        ]
    };
    // Match the dark background of the in-app project preview.
    let mut pixels = vec![Color32::from_rgb(12, 17, 24); w * h];
    let mut depth = vec![f64::MIN; w * h];
    for (t, color) in &triangles {
        let n = normalize(cross(sub(t[1], t[0]), sub(t[2], t[0])));
        // Two-sided lighting keeps inconsistently wound triangles visible.
        let shade = 0.35 + 0.65 * dot(n, light).abs();
        let rgb = [0, 1, 2].map(|i| (f64::from(color[i]) * shade).clamp(0., 255.) as u8);
        let s = t.map(screen);
        let area =
            (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = s
            .iter()
            .map(|p| p[0])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let x1 = (s.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as usize).min(w - 1);
        let y0 = s
            .iter()
            .map(|p| p[1])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let y1 = (s.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as usize).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let edge = |a: [f64; 3], b: [f64; 3]| {
                    ((b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])) / area
                };
                let (l0, l1, l2) = (edge(s[1], s[2]), edge(s[2], s[0]), edge(s[0], s[1]));
                if l0 < 0. || l1 < 0. || l2 < 0. {
                    continue;
                }
                let z = l0 * s[0][2] + l1 * s[1][2] + l2 * s[2][2];
                let i = y * w + x;
                if z > depth[i] {
                    depth[i] = z;
                    pixels[i] = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                }
            }
        }
    }
    Ok(ColorImage::new(PREVIEW, pixels))
}
