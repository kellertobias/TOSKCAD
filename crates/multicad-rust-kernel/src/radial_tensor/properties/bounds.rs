//! Physical collars and bounded closure contributions in the source chart.
use super::*;
pub(super) struct Bounds {
    pub length: f64,
    pub volume_error: I,
    pub area_errors: Vec<I>,
    pub volume_scale: I,
}
pub(super) fn prove(
    body: &RadialBoundary,
    faces: &[faces::FaceSources],
    work: &mut Work<'_>,
) -> Result<Bounds, Error> {
    let axial = &body.ordered.caps.axial;
    let chart = &axial.chart;
    let e = I::point(body.error_mm);
    let slope = I::point(chart.slope.magnitude());
    let minimum = (I::point(chart.radius_range.lo) - e * (I::point(1.) + slope)).positive()?;
    let mut radius = chart.radius_range.hi;
    for tensor in &axial.tensors {
        for chunk in tensor.controls.chunks(128) {
            work.charge(chunk.len())?;
            for p in chunk {
                radius = radius.max((p[0].square() + p[1].square()).sqrt().hi);
            }
        }
    }
    let radius = (I::point(radius) + e * (I::point(1.) + slope)).positive()?;
    let height = (chart.height + I::point(2.) * e).positive()?;
    let length = radius.hi.max(height.hi);
    let pi = I::new(
        std::f64::consts::PI.next_down(),
        std::f64::consts::PI.next_up(),
    );
    let angular = I::point(8.) * e / minimum;
    let corners = I::point((6 * axial.cones.len()) as f64);
    // Complete cap slabs, endpoint angular collars through the retained
    // height, and the radial sewing band bound any admitted closure's
    // signed-volume difference. The same cylinder bounds its moments.
    let cap = I::point(4.) * pi * radius.square() * e;
    let seam = corners * angular * radius.square() * height / I::point(2.);
    let radial = I::point(4.) * pi * radius * e * height;
    let volume_error = (cap + seam + radial).checked()?;
    let metric = (I::point(1.) + chart.slope.square()).sqrt();
    let mut area_errors = Vec::with_capacity(faces.len());
    for face in faces {
        work.step()?;
        let edges = I::point(face.edge_count as f64);
        let error = match face.density {
            measure::Density::Tensor => I::point(0.),
            measure::Density::Cap(_) => {
                // Vertical projection strips and short planar endpoint
                // joins; circle/spline curve lengths are independently
                // enclosed by the actual arc or Bernstein control polygon.
                e * face.length + I::point(2.) * e * radius * edges + pi * e.square() * edges
            }
            measure::Density::Cone => {
                // Radial seam strip: |delta_r| (1+R |theta'|), with
                // integral |theta'| <= retained rail length/r_min.
                let strips = e * face.spline_length * (I::point(1.) + radius / minimum);
                // The missing short endpoint one-form has |A1|<=R H;
                // source/vertex angular disagreement is <=8e/r_min.
                let endpoints = edges * angular * metric * radius * height;
                let ends = e * metric * face.circular_length * radius / minimum;
                strips + endpoints + ends
            }
        };
        area_errors.push(error.checked()?);
    }
    let inner = (I::point(axial.chart.radius_range.lo) - e).positive()?;
    let depth = (I::point(body.inner_z[1]) - I::point(body.inner_z[0])).positive()?;
    let volume_scale = (pi * inner.square() * depth).positive()?;
    Ok(Bounds {
        length,
        volume_error,
        area_errors,
        volume_scale,
    })
}
