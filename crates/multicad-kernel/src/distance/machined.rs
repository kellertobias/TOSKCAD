//! A stock minimum remains a material minimum when its witnesses avoid every
//! removed volume. Cut bounding boxes are conservative rejection tests only.
use super::*;
use crate::{Bounds, PlaneFrame};
use multicad_rust_kernel::planar_distance::{Face, Shape};

type Point = [f64; 3];
struct Prepared {
    shapes: Vec<Shape>,
    cuts: Vec<Bounds>,
}

pub(super) fn measure(
    a: &Solid,
    b: &Solid,
    tolerance: f64,
) -> Option<Result<MinimumDistance, GeometryError>> {
    let changed = |solid: &Solid| {
        solid
            .orthogonal_body()
            .is_some_and(|(body, _)| body.has_curved_cuts())
            || solid.polygon_prism().is_some_and(|body| {
                body.bore_treatments.is_none()
                    && (!body.pockets.is_empty() || !body.prism.circular_holes().is_empty())
            })
    };
    if !changed(a) && !changed(b) {
        return None;
    }
    Some((|| {
        let a = prepare(a, tolerance)?;
        let b = prepare(b, tolerance)?;
        let result = planar::measure_prepared(&a.shapes, &b.shapes, tolerance)?;
        if !result.witnesses_complete {
            return Err(error(
                GeometryErrorKind::Unsupported,
                format!(
                    "Machined distance needs a complete finite stock witness enumeration (distance {}, witnesses {}/{})",
                    result.distance_mm,
                    result.witnesses.len(),
                    result.total_solutions
                ),
            ));
        }
        let clear = |point: Point, cuts: &[Bounds]| {
            cuts.iter()
                .all(|cut| (0..3).any(|i| point[i] < cut.min[i] || point[i] > cut.max[i]))
        };
        if result
            .witnesses
            .iter()
            .any(|w| !clear(w.a_mm, &a.cuts) || !clear(w.b_mm, &b.cuts))
        {
            return Err(error(
                GeometryErrorKind::Unsupported,
                "Stock distance witnesses do not certify retained material outside the cuts",
            ));
        }
        // Every finite stock candidate survives. A bounding-box rejection
        // would be unproved survival, so never filter it and claim completeness.
        Ok(result)
    })())
}

fn point(frame: PlaneFrame, p: Point) -> Point {
    let n = frame.normal();
    std::array::from_fn(|i| frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + n[i] * p[2])
}

fn prepare(solid: &Solid, tolerance: f64) -> Result<Prepared, GeometryError> {
    if let Some(body) = solid.polygon_prism().filter(|body| {
        body.bore_treatments.is_none()
            && (!body.pockets.is_empty() || !body.prism.circular_holes().is_empty())
    }) {
        let prism = multicad_rust_kernel::polygon::PolygonPrism::new(
            body.prism.rings().to_vec(),
            body.prism.height(),
        )
        .map_err(|cause| {
            error(
                GeometryErrorKind::InvalidResult,
                format!("Invalid machining stock: {cause:?}"),
            )
        })?;
        let edges = body.edges.len() - body.prism.circular_holes().len();
        let origins: Vec<_> = body.edges[..edges]
            .iter()
            .map(|(origin, _)| origin.clone())
            .collect();
        let reverse: Vec<_> = body.edges[..edges]
            .iter()
            .map(|(_, reverse)| *reverse)
            .collect();
        let stock = Solid::from_polygon_prism_sweep(
            prism,
            body.frame,
            body.reverse_sweep,
            body.feature,
            &origins,
            &reverse,
            solid.geometry_policy(),
        )?;
        let sign = if body.reverse_sweep { -1. } else { 1. };
        let height = body.prism.height().as_f64();
        let mut cuts = Vec::new();
        for pocket in body.pockets.iter() {
            let [x, y] = pocket.center_mm;
            cuts.push(cut_bounds(
                body.frame,
                &[
                    [x, y, sign * pocket.mouth_z_mm(height)],
                    [x, y, sign * pocket.floor_z_mm(height)],
                ],
                pocket.radius_mm,
                tolerance,
            )?);
        }
        for bore in body.prism.circular_holes() {
            let [x, y] = bore.center().map(multicad_rust_kernel::Rational::as_f64);
            cuts.push(cut_bounds(
                body.frame,
                &[[x, y, 0.], [x, y, sign * height]],
                bore.radius().as_f64(),
                tolerance,
            )?);
        }
        return Ok(Prepared {
            shapes: planar::prepare(&stock, BRepSelection::Whole)?,
            cuts,
        });
    }
    let Some((body, frame)) = solid
        .orthogonal_body()
        .filter(|(body, _)| body.has_curved_cuts())
    else {
        return Ok(Prepared {
            shapes: planar::prepare(solid, BRepSelection::Whole)?,
            cuts: vec![],
        });
    };
    let stock = body.drawing_stock();
    let mut shape = Shape {
        solid: true,
        ..Default::default()
    };
    for face in stock.faces {
        let mut boundary = Vec::new();
        for ring in &face.rings {
            if shape.points.len().saturating_add(ring.len()) > 200_000 {
                return Err(error(
                    GeometryErrorKind::Limit,
                    "Machined stock distance exceeds its vertex budget",
                ));
            }
            let ring: Vec<_> = ring.iter().map(|&p| point(frame, p)).collect();
            for i in 0..ring.len() {
                let edge = [ring[i], ring[(i + 1) % ring.len()]];
                boundary.push(edge);
                shape.edges.push(edge);
            }
            shape.points.extend(ring);
        }
        let n = frame.normal();
        shape.faces.push(Face {
            point: boundary[0][0],
            normal: std::array::from_fn(|i| {
                frame.u[i] * face.normal[0] + frame.v[i] * face.normal[1] + n[i] * face.normal[2]
            }),
            boundary,
        });
    }
    let mut cuts = Vec::new();
    for pocket in stock.pockets {
        cuts.push(cut_bounds(
            frame,
            &[
                pocket.center_mm,
                pocket.floor_center_mm(stock.faces[pocket.face].normal),
            ],
            pocket.radius_mm,
            tolerance,
        )?);
    }
    for bore in stock.through_bores {
        cuts.push(cut_bounds(
            frame,
            &bore.centers_mm,
            bore.radius_mm,
            tolerance,
        )?);
    }
    for slot in stock.slots {
        let floor = slot.floor_centers_mm(stock.faces[slot.face].normal);
        cuts.push(cut_bounds(
            frame,
            &[slot.centers_mm[0], slot.centers_mm[1], floor[0], floor[1]],
            slot.radius_mm,
            tolerance,
        )?);
    }
    Ok(Prepared {
        shapes: vec![shape],
        cuts,
    })
}

fn cut_bounds(
    frame: PlaneFrame,
    centers: &[Point],
    radius: f64,
    tolerance: f64,
) -> Result<Bounds, GeometryError> {
    // A cylinder/stadium prism lies inside this local box. Transform all
    // corners, retaining frame drift instead of assuming an exact rotation.
    let lo: Point = std::array::from_fn(|i| {
        (centers.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min) - radius).next_down()
    });
    let hi: Point = std::array::from_fn(|i| {
        (centers
            .iter()
            .map(|p| p[i])
            .fold(f64::NEG_INFINITY, f64::max)
            + radius)
            .next_up()
    });
    let corners: [Point; 8] = std::array::from_fn(|bits| {
        point(
            frame,
            std::array::from_fn(|i| if bits & (1 << i) == 0 { lo[i] } else { hi[i] }),
        )
    });
    let n = frame.normal();
    let magnitude = (0..3)
        .map(|i| {
            frame.origin[i].abs()
                + frame.u[i].abs() * lo[0].abs().max(hi[0].abs())
                + frame.v[i].abs() * lo[1].abs().max(hi[1].abs())
                + n[i].abs() * lo[2].abs().max(hi[2].abs())
        })
        .fold(1., f64::max);
    let arithmetic = 128. * f64::EPSILON * magnitude;
    if !arithmetic.is_finite() || arithmetic > tolerance / 16. {
        return Err(error(
            GeometryErrorKind::Unsupported,
            "Machining distance placement exceeds the numerical budget",
        ));
    }
    let margin = tolerance * 8. + arithmetic;
    Ok(Bounds {
        min: std::array::from_fn(|i| {
            (corners.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min) - margin).next_down()
        }),
        max: std::array::from_fn(|i| {
            (corners
                .iter()
                .map(|p| p[i])
                .fold(f64::NEG_INFINITY, f64::max)
                + margin)
                .next_up()
        }),
    })
}
