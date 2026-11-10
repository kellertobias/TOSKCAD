//! Recover the centerline of two cylindrical legs from their three rims.
use super::{axial_cuts::verify, *};

pub(super) fn reconstruct(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
) -> Result<Option<ImportedWarped>> {
    if boundary.faces.len() != 10
        || boundary.curves.len() != 20
        || boundary.faces.iter().any(|face| face.loop_.len() != 4)
    {
        return Ok(None);
    }

    let mut caps = boundary
        .faces
        .iter()
        .filter(|face| matches!(face.surface, Surface::Plane { .. }));
    let (Some(start), Some(end), None) = (caps.next(), caps.next(), caps.next()) else {
        return Ok(None);
    };
    let [Some(start), Some(end)] = [start, end].map(|face| circle(boundary, face)) else {
        return Ok(None);
    };
    let mut joins = boundary.curves.values().filter_map(|&(_, curve)| {
        let TrimCurve::Arc(arc) = curve else {
            return None;
        };
        let ellipse = arc.ellipse();
        let [major, minor] = ellipse.semiaxes_mm();
        (major > minor).then_some(ellipse)
    });
    let Some(miter) = joins.next() else {
        return Ok(None);
    };
    if joins.count() != 3 {
        return Ok(None);
    }

    // This only proposes a centerline. The replay matches every oriented
    // STEP edge and face against the shell derived from it.
    let origin_mm = start.center_mm();
    let u = [1., 0., 0.];
    let v = [0., 1., 0.];
    let chart = Chart {
        origin: origin_mm,
        u,
        v,
        up: [0., 0., 1.],
        band: Band {
            center_z_mm: 0.,
            radius_mm: 1.,
        },
    };
    let path = [origin_mm, miter.center_mm(), end.center_mm()].map(|point| chart.local(point));
    let shell = warped_union::circular_miter(path, start.semiaxes_mm()[0], reader.tolerance)
        .map_err(axial_cuts::replay_error)?;
    verify::replayed(reader, boundary, chart, &shell)?;
    Ok(Some(ImportedWarped {
        shell,
        origin_mm,
        u,
        v,
    }))
}

fn circle(boundary: &Boundary, face: &FaceLoop) -> Option<Ellipse> {
    let TrimCurve::Arc(arc) = boundary.curves[&face.loop_[0].0].1 else {
        return None;
    };
    let ellipse = arc.ellipse();
    let [major, minor] = ellipse.semiaxes_mm();
    (major == minor).then_some(ellipse)
}

#[cfg(test)]
mod tests;
