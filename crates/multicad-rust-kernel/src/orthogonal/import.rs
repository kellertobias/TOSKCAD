//! Reconstruct orthogonal material from a complete set of oriented faces.

use super::{BooleanError, Grid, MAX_QUADS, OrthogonalSolid, UNVISITED, canonical};

const MAX_FACE_SCAN_POINTS: usize = 2_000_000;

pub(crate) struct OrientedFace {
    pub normal: [f64; 3],
    pub rings: Vec<Vec<[f64; 3]>>,
}

/// Rings use the exact grid coordinates, with winding relative to the outward
/// normal. Reconstruct material from one axis and prove its signed boundary on
/// every axis. Adjacent coplanar face partitions are permitted.
pub(crate) fn solid_from_oriented_faces(
    faces: &[OrientedFace],
    tolerance_mm: f64,
) -> Result<OrthogonalSolid, BooleanError> {
    if !tolerance_mm.is_finite() || tolerance_mm < 0. || faces.is_empty() {
        return Err(BooleanError::Invalid);
    }
    if faces.len() > MAX_QUADS {
        return Err(BooleanError::Limit);
    }
    let mut axes = std::array::from_fn(|_| Vec::new());
    for face in faces {
        for point in face.rings.iter().flatten() {
            if !point.iter().all(|value| value.is_finite()) {
                return Err(BooleanError::Invalid);
            }
            if axes[0].len() == 4 * MAX_QUADS {
                return Err(BooleanError::Limit);
            }
            for (axis, value) in axes.iter_mut().zip(point) {
                axis.push(canonical(*value));
            }
        }
    }
    let grid = Grid::from_axes(axes, tolerance_mm)?;
    if grid.dims.iter().any(|&dim| dim < 2) {
        return Err(BooleanError::Invalid);
    }
    let mut patches = std::array::from_fn(|_| vec![0; grid.points]);
    let mut remaining = MAX_FACE_SCAN_POINTS;
    for face in faces {
        paint_face(&grid, &mut patches, face, &mut remaining)?;
    }
    let mut material = vec![0; grid.points];
    let width = grid.dims[0];
    // An outward +X face leaves material. Each scanline starts and ends
    // outside the solid.
    for (index, &patch) in patches[0].iter().enumerate() {
        let x = index % width;
        let previous = x > 0 && material[index - 1] != 0;
        match i8::from(previous) - patch {
            0 => {}
            1 if x + 1 < width => material[index] = UNVISITED,
            _ => return Err(BooleanError::Unresolved),
        }
    }
    // Every remaining oriented patch must be an actual material transition.
    for (axis, patches) in patches.iter().enumerate().skip(1) {
        for (index, &patch) in patches.iter().enumerate() {
            let at = grid.coordinates(index);
            let previous = grid
                .neighbor(index, at, axis, false)
                .is_some_and(|previous| material[previous] != 0);
            let current = material[index] != 0;
            if patch != i8::from(previous) - i8::from(current) {
                return Err(BooleanError::Unresolved);
            }
        }
    }
    let count = grid.label_components(&mut material)?;
    if count != 1 {
        return Err(BooleanError::Unresolved);
    }
    grid.boundary(&material, count)
}

fn paint_face(
    grid: &Grid,
    patches: &mut [Vec<i8>; 3],
    face: &OrientedFace,
    remaining: &mut usize,
) -> Result<(), BooleanError> {
    let axis = face
        .normal
        .iter()
        .position(|&component| component != 0.)
        .ok_or(BooleanError::Invalid)?;
    let sign = match face.normal[axis] {
        -1. => -1,
        1. => 1,
        _ => return Err(BooleanError::Invalid),
    };
    if face
        .normal
        .iter()
        .enumerate()
        .any(|(i, &component)| i != axis && component != 0.)
    {
        return Err(BooleanError::Invalid);
    }
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    let first = face
        .rings
        .first()
        .and_then(|ring| ring.first())
        .ok_or(BooleanError::Invalid)?;
    let plane = grid.plane(axis, first[axis]);
    let mut low = [usize::MAX; 2];
    let mut high = [0; 2];
    let mut rings = Vec::with_capacity(face.rings.len());
    for ring in &face.rings {
        if ring.len() < 4 {
            return Err(BooleanError::Invalid);
        }
        let mut indices = Vec::with_capacity(ring.len());
        for point in ring {
            if grid.plane(axis, point[axis]) != plane {
                return Err(BooleanError::Invalid);
            }
            let at = [grid.plane(u, point[u]), grid.plane(v, point[v])];
            for i in 0..2 {
                low[i] = low[i].min(at[i]);
                high[i] = high[i].max(at[i]);
            }
            indices.push(at);
        }
        rings.push(indices);
    }
    let [width, height] = std::array::from_fn(|i| high[i] - low[i] + 1);
    let points = width * height;
    *remaining = remaining.checked_sub(points).ok_or(BooleanError::Limit)?;
    let index = |[x, y]: [usize; 2]| (x - low[0]) + (y - low[1]) * width;
    let mut along_u = vec![0; points];
    let mut along_v = vec![0; points];
    // Grid edge intervals encode signed crossings. The face normal sets the
    // expected winding, so an interior hole contributes opposite crossings.
    for ring in &rings {
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            match (a[0] == b[0], a[1] == b[1]) {
                (true, false) => {
                    let change = if b[1] > a[1] { -sign } else { sign };
                    for y in a[1].min(b[1])..a[1].max(b[1]) {
                        crossing(&mut along_u[index([a[0], y])], change)?;
                    }
                }
                (false, true) => {
                    let change = if b[0] > a[0] { sign } else { -sign };
                    for x in a[0].min(b[0])..a[0].max(b[0]) {
                        crossing(&mut along_v[index([x, a[1]])], change)?;
                    }
                }
                _ => return Err(BooleanError::Invalid),
            }
        }
    }
    let mut previous = vec![0; width];
    // The two planar scan directions must agree on the same face patch.
    for y in low[1]..=high[1] {
        let mut inside = 0;
        for (column, x) in (low[0]..=high[0]).enumerate() {
            let index = index([x, y]);
            inside += along_u[index];
            if !matches!(inside, 0 | 1) || inside - previous[column] != along_v[index] {
                return Err(BooleanError::Unresolved);
            }
            previous[column] = inside;
            if inside == 1 {
                let mut at = [0; 3];
                at[axis] = plane;
                at[u] = x;
                at[v] = y;
                crossing(&mut patches[axis][grid.index(at)], sign)?;
            }
        }
    }
    Ok(())
}

fn crossing(slot: &mut i8, change: i8) -> Result<(), BooleanError> {
    if *slot != 0 {
        return Err(BooleanError::Unresolved);
    }
    *slot = change;
    Ok(())
}

#[cfg(test)]
mod tests;
