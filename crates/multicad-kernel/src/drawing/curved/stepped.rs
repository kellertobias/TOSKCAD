//! Exact drawing edges for full, solid stepped coaxial bodies.
use super::*;
type Point = [f64; 3];

pub(super) fn rounds(
    solid: &Solid,
    owner: usize,
    options: DrawingProjection,
) -> Result<Option<Vec<RoundOccluder>>, GeometryError> {
    let Some((body, placement)) = solid.revolution_body() else {
        return Ok(None);
    };
    let [ring] = body.geometry.rings() else {
        return Ok(None);
    };
    if !body.geometry.is_full() {
        return Ok(None);
    }
    let Some(bands) = axial_bands(ring, options.tangent_edges) else {
        return Ok(None);
    };
    let screen = planar::view_axes(options.frame);
    let vector = |p| screen.map(|axis| dot(axis, p));
    let point = |p: Point| vector(std::array::from_fn(|i| p[i] - options.frame.origin[i]));
    let axis = vector(unit(placement.normal()));

    // Every cap rim belongs to the authored outline: at a change in
    // radius the two caps supply the inner and outer shoulder edges.
    let rounds = bands
        .into_iter()
        .map(|[radius, low, high]| {
            let base = crate::rust_prism_topology::position(placement, [0., 0., low]);
            RoundOccluder::new(point(base), axis, radius, high - low, None)
                .map(|round| round.with_contour_owner(owner))
                .map_err(planar::projection_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(rounds))
}

fn axial_bands(ring: &[[f64; 2]], tangent_edges: bool) -> Option<Vec<[f64; 3]>> {
    let mut bands = Vec::new();
    let mut axial_wall = None;
    for (&[r0, z0], &[r1, z1]) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if z0 == z1 {
            continue;
        }
        if r0 != r1 {
            return None;
        }
        if r0 == 0. {
            if z1 >= z0 || axial_wall.replace([z1, z0]).is_some() {
                return None;
            }
        } else {
            if z1 <= z0 {
                return None;
            }
            bands.push([r0, z0, z1]);
        }
    }
    bands.sort_unstable_by(|a, b| a[1].total_cmp(&b[1]));
    let [start, end] = axial_wall?;
    if bands.first()?[1] != start
        || bands.last()?[2] != end
        || bands.windows(2).any(|pair| pair[0][2] != pair[1][1])
    {
        return None;
    }
    if tangent_edges {
        return Some(bands);
    }
    let mut merged: Vec<[f64; 3]> = Vec::new();
    for band in bands {
        if let Some(last) = merged.last_mut()
            && last[0] == band[0]
        {
            last[2] = band[2];
        } else {
            merged.push(band);
        }
    }
    Some(merged)
}
