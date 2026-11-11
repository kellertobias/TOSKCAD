//! Exact axial and transverse torus silhouettes and principal plane cuts.
use super::*;
use multicad_rust_kernel::{
    curved_projection::{Curve, Detail, Edge, Ellipse},
    planar_projection::{AnalyticFace, SpatialRimCandidate},
};
use std::f64::consts::{PI, TAU};

mod transverse;

const ANGULAR: f64 = 64. * f64::EPSILON;

fn unsupported() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust torus drawings require an axial or transverse view. Retained sections require an axial view.",
    )
}

fn drawing_frame(torus: PlaneFrame, view: PlaneFrame) -> ([f64; 3], [f64; 3]) {
    let axes = planar::view_axes(view);
    let local = |point: [f64; 3]| {
        axes.map(|axis| axis.into_iter().zip(point).map(|(a, b)| a * b).sum::<f64>())
    };
    let offset = std::array::from_fn(|i| torus.origin[i] - view.origin[i]);
    (local(offset), local(torus.normal()))
}

pub(super) fn axial_center(torus: PlaneFrame, view: PlaneFrame) -> Result<[f64; 3], GeometryError> {
    let (center, [a, b, _]) = drawing_frame(torus, view);
    if a.hypot(b) > ANGULAR {
        return Err(unsupported());
    }
    Ok(center)
}

pub(super) fn section_radii([major, minor]: [f64; 2], delta: f64) -> [f64; 2] {
    let half = ((minor - delta.abs()) * (minor + delta.abs())).sqrt();
    [major + half, major - half]
}

fn circle(center: [f64; 2], radius: f64, u: [f64; 2], v: [f64; 2], trim: [f64; 2]) -> Ellipse {
    Ellipse {
        center,
        major: radius,
        minor: radius,
        u,
        v,
        trim,
    }
}

pub(super) fn scene_member(
    torus: PlaneFrame,
    [major, minor]: [f64; 2],
    view: PlaneFrame,
    source: usize,
) -> Result<(AnalyticFace, Vec<SpatialRimCandidate>), GeometryError> {
    let (center, axis) = drawing_frame(torus, view);
    let [x, y, depth] = center;
    let center = [x, y];
    let [a, b, facing] = axis;
    let projected = a.hypot(b);
    if projected > ANGULAR && facing.abs() > ANGULAR {
        return Err(unsupported());
    }
    let face = AnalyticFace::torus([x, y, depth], axis, [major, minor])
        .map_err(planar::projection_error)?;
    let rim = |ellipse, depth_sine| SpatialRimCandidate {
        source,
        ellipse,
        depth,
        depth_sine,
    };
    let curves = if projected <= ANGULAR {
        [major + minor, major - minor]
            .map(|radius| rim(circle(center, radius, [1., 0.], [0., 1.], [0., TAU]), 0.))
            .to_vec()
    } else {
        let axial = [a / projected, b / projected];
        let across = [-axial[1], axial[0]];
        let shift = |direction: [f64; 2], distance| {
            std::array::from_fn(|i| direction[i].mul_add(distance, center[i]))
        };
        let caps = [-major, major].map(|distance| shift(across, distance));
        let mut curves = vec![
            rim(circle(caps[0], minor, axial, across, [PI, TAU]), 0.),
            rim(circle(caps[1], minor, axial, across, [0., PI]), 0.),
        ];
        for offset in [-minor, minor] {
            // The two apparent straight edges are projected half-circles.
            // Their actual depth varies along the projection.
            curves.push(rim(
                Ellipse {
                    minor: 0.,
                    ..circle(shift(axial, offset), major, across, axial, [0., PI])
                },
                major,
            ));
        }
        curves
    };
    Ok((face, curves))
}

pub(super) fn project(
    input: &DrawingSolid<'_>,
    torus: PlaneFrame,
    [major, minor]: [f64; 2],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let ([x, y, z], [a, b, facing]) = drawing_frame(torus, options.frame);
    let center = [x, y];
    let axial = a.hypot(b) <= ANGULAR;
    if let Some(depth) = options.section_offset_mm {
        if options.detail.is_some() {
            return Err(unsupported());
        }
        if axial {
            return axial_section(input, center, z, [major, minor], depth, options);
        }
        if options.section_only && facing.abs() <= ANGULAR {
            let delta = depth - z;
            if delta.abs() >= major + minor {
                return Ok(Vec::new());
            }
            let roundoff = ANGULAR * z.abs().max(depth.abs()).max(major);
            let length = a.hypot(b);
            let across = [-b / length, a / length];
            if delta.abs() <= roundoff {
                return diametral_cut(input, center, across, [major, minor], options);
            }
            return transverse::project(
                input,
                center,
                across,
                [major, minor],
                delta.abs(),
                options,
            );
        }
        return Err(unsupported());
    }
    if !axial {
        if facing.abs() > ANGULAR {
            return Err(unsupported());
        }
        return curved::project_scene(std::slice::from_ref(input), options);
    }
    let detail = options
        .detail
        .map(|(center, radius)| Detail { center, radius });
    let edges = [major + minor, major - minor]
        .into_iter()
        .flat_map(|radius| {
            let rim = circle(center, radius, [1., 0.], [0., 1.], [0., TAU]);
            let trims = detail.map_or_else(|| vec![rim.trim], |detail| rim.in_detail(detail));
            trims.into_iter().map(move |trim| Edge {
                hidden: false,
                curve: Curve::Ellipse(Ellipse { trim, ..rim }),
            })
        })
        .collect();
    curved::publish(edges, options, 0)
}

fn diametral_cut(
    input: &DrawingSolid<'_>,
    center: [f64; 2],
    across: [f64; 2],
    [major, minor]: [f64; 2],
    mut options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut output = Vec::new();
    // The plane through the revolution axis cuts one tube circle on each
    // side. The material between those disjoint circles is empty.
    for offset in [-major, major] {
        let disk_center = std::array::from_fn(|i| across[i].mul_add(offset, center[i]));
        let vectors = section::project_concentric(disk_center, [minor, 0.], input, options)?;
        consume_vectors(&mut options, &vectors);
        output.extend(vectors);
    }
    Ok(output)
}

fn consume_vectors(options: &mut DrawingProjection, vectors: &[ProjectedVector]) {
    options.maximum_curves -= vectors.len() as u32;
    options.maximum_points -= vectors
        .iter()
        .map(|vector| vector.preview_mm.len() as u32)
        .sum::<u32>();
}

fn axial_section(
    input: &DrawingSolid<'_>,
    center: [f64; 2],
    z: f64,
    [major, minor]: [f64; 2],
    depth: f64,
    mut options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let near = depth - z;
    let radii = |delta| section_radii([major, minor], delta);
    let cuts_near = near.abs() < minor;
    if options.section_only {
        return if cuts_near {
            section::project_concentric(center, radii(near), input, options)
        } else {
            Ok(Vec::new())
        };
    }
    let far = options
        .section_depth_mm
        .map_or(f64::NEG_INFINITY, |distance| near - distance);
    if near <= -minor || far >= minor {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    let hidden_lines = options.hidden_lines;
    let mut publish = |level, role| {
        let section_input = DrawingSolid {
            solid: input.solid,
            hatch: input.hatch.filter(|_| role == VectorRole::Section),
        };
        let mut vectors =
            section::project_concentric(center, radii(level), &section_input, options)?;
        if role != VectorRole::Section {
            for vector in &mut vectors {
                vector.role = role;
            }
        }
        consume_vectors(&mut options, &vectors);
        output.extend(vectors);
        Ok(())
    };
    if cuts_near {
        publish(near, VectorRole::Section)?;
    }
    let widest = 0_f64.clamp(far, near);
    if widest != near {
        publish(widest, VectorRole::Visible)?;
    }
    if hidden_lines && far.abs() < minor && far != widest {
        publish(far, VectorRole::Hidden)?;
    }
    Ok(output)
}
