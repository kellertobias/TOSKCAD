//! Drawing adapter for exact circular cylinders.
use super::*;
use multicad_rust_kernel::curved_projection::{self, Curve, Cylinder, Detail, Error};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}

fn unit(vector: [f64; 3]) -> [f64; 3] {
    let length = dot(vector, vector).sqrt();
    vector.map(|v| v / length)
}

fn projection_error(error: Error) -> GeometryError {
    match error {
        Error::Invalid => err(
            GeometryErrorKind::InvalidResult,
            "Invalid analytic cylinder projection",
        ),
        Error::Limit => err(
            GeometryErrorKind::Limit,
            "Curved drawing projection exceeded its point budget",
        ),
    }
}

pub(super) fn project(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (cylinder, placement) = input.solid.axial_cylinder_frame().ok_or_else(|| {
        err(
            GeometryErrorKind::Unsupported,
            "Rust curved projection requires a circular cylinder",
        )
    })?;
    let frame = options.frame;
    let [u, v, n] = planar::view_axes(frame);
    let base = std::array::from_fn(|i| placement.origin[i] - frame.origin[i]);
    let axis = unit(placement.normal());
    let edges = curved_projection::project(
        Cylinder {
            base: [dot(base, u), dot(base, v)],
            axis: [u, v, n].map(|direction| dot(direction, axis)),
            radius: cylinder.circle().radius().as_f64(),
            height: cylinder.height().as_f64(),
        },
        options.hidden_lines,
        options
            .detail
            .map(|(center, radius)| Detail { center, radius }),
    )
    .map_err(projection_error)?;
    if edges.len() > options.maximum_curves as usize {
        return Err(err(
            GeometryErrorKind::Limit,
            "Curved drawing projection exceeded its curve budget",
        ));
    }
    let mut points = options.maximum_points as usize;
    edges
        .into_iter()
        .map(|edge| {
            let preview_mm = edge
                .curve
                .preview(options.chord_tolerance_mm, points)
                .map_err(projection_error)?;
            points -= preview_mm.len();
            let curve = match edge.curve {
                Curve::Line([a, b]) => VectorCurve::Line { a, b },
                Curve::Ellipse(ellipse) => VectorCurve::Ellipse {
                    center: ellipse.center,
                    u: ellipse.u,
                    v: ellipse.v,
                    major_mm: ellipse.major,
                    minor_mm: ellipse.minor,
                    start_rad: ellipse.trim[0],
                    end_rad: ellipse.trim[1],
                },
            };
            Ok(ProjectedVector {
                source_index: 0,
                role: if edge.hidden {
                    VectorRole::Hidden
                } else {
                    VectorRole::Visible
                },
                curve,
                preview_mm,
            })
        })
        .collect()
}
