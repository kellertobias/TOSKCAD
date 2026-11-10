//! End-on projection of a straight polygon sweep with concentric bore layers.

use super::*;

pub(in crate::drawing) fn project_chamfered_bores(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let stock = input.solid.polygon_prism()?;
    let treatments = stock.bore_treatments.as_ref()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = stock.frame.normal();
    if dot(axis, u).hypot(dot(axis, v)) > 32. * f64::EPSILON {
        return None;
    }
    if options.detail.is_some() || !stock.pockets.is_empty() {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust face-on chamfered stock requires an uncropped view of through-bores",
        )));
    }
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    let project = |p| screen.point(stock.frame.point(p));
    let end = (dot(axis, toward_viewer) > 0.) != stock.reverse_sweep;
    let mut edges = Vec::new();
    for ring in stock.prism.rings() {
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            edges.push(Edge {
                hidden: false,
                curve: Curve::Line([a, b].map(|point| project(point.map(|v| v.as_f64())))),
            });
        }
    }
    for stack in treatments.geometry.stacks() {
        let center = project(stack.center_mm);
        // Every radius is linear between adjacent rim planes. The
        // minimum of the preceding radii therefore certifies the clear
        // ray to this entire concentric rim. Identical projected circles
        // retain the nearest visible or hidden occurrence only.
        let layers = &stack.layers;
        for step in 0..layers.len() {
            let i = if end { layers.len() - 1 - step } else { step };
            let radius = layers[i].radius_mm;
            let preceding = if end { &layers[i + 1..] } else { &layers[..i] };
            if preceding.iter().any(|rim| rim.radius_mm == radius) {
                continue;
            }
            let hidden = preceding.iter().any(|rim| rim.radius_mm < radius);
            if !hidden || options.hidden_lines {
                edges.push(Edge {
                    hidden,
                    curve: Curve::Ellipse(Ellipse {
                        center,
                        u: [1., 0.],
                        v: [0., 1.],
                        major: radius,
                        minor: radius,
                        trim: [0., TAU],
                    }),
                });
            }
        }
    }
    Some(curved::publish(edges, options, 0))
}
