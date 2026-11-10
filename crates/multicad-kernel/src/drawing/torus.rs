//! Analytic isolated torus silhouettes and axial sections.
use super::*;
use multicad_rust_kernel::curved_projection::{Curve, Detail, Edge, Ellipse};
use std::f64::consts::{PI, TAU};

fn unsupported() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust torus drawings require an isolated axial or transverse view, or an axial section without geometry behind it",
    )
}

pub(super) fn project(
    input: &DrawingSolid<'_>,
    torus: PlaneFrame,
    [major, minor]: [f64; 2],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let axes = planar::view_axes(options.frame);
    let local = |point: [f64; 3]| {
        axes.map(|axis| axis.into_iter().zip(point).map(|(a, b)| a * b).sum::<f64>())
    };
    let [x, y, z] = local(std::array::from_fn(|i| {
        torus.origin[i] - options.frame.origin[i]
    }));
    let center = [x, y];
    let [a, b, facing] = local(torus.normal());
    let projected_axis = a.hypot(b);
    let roundoff = 64. * f64::EPSILON;
    let ellipse = |center, radius, u, v, trim| Ellipse {
        center,
        major: radius,
        minor: radius,
        u,
        v,
        trim,
    };
    let visible = |curve| Edge {
        hidden: false,
        curve,
    };
    if projected_axis <= roundoff {
        if let Some(depth) = options.section_offset_mm {
            if !options.section_only || options.detail.is_some() {
                return Err(unsupported());
            }
            let delta = (depth - z).abs();
            if delta >= minor {
                return Ok(vec![]);
            }
            let half_width = ((minor - delta) * (minor + delta)).sqrt();
            return section::project_concentric(
                center,
                [major + half_width, major - half_width],
                input,
                options,
            );
        }
        let detail = options
            .detail
            .map(|(center, radius)| Detail { center, radius });
        let edges = [major + minor, major - minor]
            .into_iter()
            .flat_map(|radius| {
                let rim = ellipse(center, radius, [1., 0.], [0., 1.], [0., TAU]);
                let trims = detail.map_or_else(|| vec![rim.trim], |detail| rim.in_detail(detail));
                trims
                    .into_iter()
                    .map(move |trim| visible(Curve::Ellipse(Ellipse { trim, ..rim })))
            })
            .collect();
        return curved::publish(edges, options, 0);
    }
    if facing.abs() > roundoff || options.section_offset_mm.is_some() || options.detail.is_some() {
        return Err(unsupported());
    }
    // Edge-on, the generating circle projects to a segment. The silhouette
    // is that segment offset by the minor-radius disk.
    let axial = [a / projected_axis, b / projected_axis];
    let across = [-axial[1], axial[0]];
    let caps = [-major, major]
        .map(|distance| std::array::from_fn(|i| across[i].mul_add(distance, center[i])));
    let ends =
        |offset: f64| caps.map(|point| std::array::from_fn(|i| axial[i].mul_add(offset, point[i])));
    let edges = vec![
        visible(Curve::Line(ends(-minor))),
        visible(Curve::Line(ends(minor))),
        visible(Curve::Ellipse(ellipse(
            caps[0],
            minor,
            axial,
            across,
            [PI, TAU],
        ))),
        visible(Curve::Ellipse(ellipse(
            caps[1],
            minor,
            axial,
            across,
            [0., PI],
        ))),
    ];
    curved::publish(edges, options, 0)
}
