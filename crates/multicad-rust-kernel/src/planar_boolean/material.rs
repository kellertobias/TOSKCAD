use super::*;

/// Positive cells independently checked against the authored boundary of a cut.
#[derive(Clone, Debug)]
pub struct CutMaterial {
    pub(super) boundary: Union,
    pub(super) pieces: Vec<ConvexPiece>,
}

impl CutMaterial {
    pub fn pieces(&self) -> &[ConvexPiece] {
        &self.pieces
    }

    /// Partition the original stock into convex pieces outside every cutter.
    /// Accept the cells only if their regularized boundary covers the cut
    /// boundary in both directions, with outward-oriented supporting planes.
    pub fn recover(
        boundary: &Union,
        stock: &[ConvexPiece],
        tools: &[ConvexPiece],
        tolerance: f64,
    ) -> Result<Self, Error> {
        validate_tolerance(tolerance)?;
        if stock.is_empty() || tools.is_empty() {
            return Err(Error::Invalid);
        }
        if stock.len().saturating_add(tools.len()) > MAX_PIECES {
            return Err(Error::Limit);
        }
        let mut budget = MAX_WORK;
        let mut pieces = stock.to_vec();
        for tool in tools {
            let mut remaining = Vec::new();
            for piece in pieces {
                subtract_cell(piece, tool, tolerance, &mut budget, &mut remaining)?;
                if remaining.len() > MAX_PIECES {
                    return Err(Error::Limit);
                }
            }
            pieces = remaining;
        }
        verify_cells(boundary, &pieces, tolerance, &mut budget)?;
        Ok(Self {
            boundary: boundary.clone(),
            pieces,
        })
    }

    /// Carry both certified representations through a placement.
    pub fn transformed(
        &self,
        point: &dyn Fn(Point) -> Point,
        tolerance: f64,
    ) -> Result<Self, Error> {
        let mut transformed = shell::Boundary::new(tolerance);
        for (i, face) in self.boundary.faces.iter().enumerate() {
            let ring: Vec<_> = face
                .vertices
                .iter()
                .map(|&vertex| point(self.boundary.vertices[vertex]))
                .collect();
            if ring
                .iter()
                .flatten()
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE)
            {
                return Err(Error::Invalid);
            }
            let plane = supporting_plane(&ring, tolerance)?;
            transformed.add(&ring, plane.normal, 0, i)?;
        }
        let pieces = self
            .pieces
            .iter()
            .map(|piece| piece.transformed(point, tolerance))
            .collect::<Result<Vec<_>, _>>()?;
        let boundary = Union::from_boundary(transformed)?;
        let mut budget = MAX_WORK;
        verify_cells(&boundary, &pieces, tolerance, &mut budget)?;
        Ok(Self { boundary, pieces })
    }
}

fn subtract_cell(
    piece: ConvexPiece,
    tool: &ConvexPiece,
    tolerance: f64,
    budget: &mut usize,
    output: &mut Vec<ConvexPiece>,
) -> Result<(), Error> {
    if !clipping::bounds_overlap(piece.bounds, tool.bounds, tolerance) {
        output.push(piece);
        return Ok(());
    }
    let mut interior = piece.clone();
    let mut outside = Vec::new();
    for &plane in &tool.planes {
        charge(
            budget,
            interior.faces.iter().map(Vec::len).sum::<usize>() * interior.faces.len(),
        )?;
        match split_cell(interior, plane, tolerance)? {
            Split::Inside(cell) => interior = cell,
            Split::Outside => {
                // No volume lies in every tool half-space. Retain the whole
                // original cell, including contact along a face or an edge.
                output.push(piece);
                return Ok(());
            }
            Split::Both(inside, cell) => {
                interior = inside;
                outside.push(cell);
            }
        }
    }
    output.extend(outside);
    Ok(())
}

enum Split {
    Inside(ConvexPiece),
    Outside,
    Both(ConvexPiece, ConvexPiece),
}

fn precision(points: impl Iterator<Item = Point>, tolerance: f64) -> f64 {
    let magnitude = points.flatten().map(f64::abs).fold(1., f64::max);
    (magnitude * 8. * f64::EPSILON)
        .max(tolerance * 1e-3)
        .min(tolerance)
}

fn split_cell(piece: ConvexPiece, plane: Plane, tolerance: f64) -> Result<Split, Error> {
    let precision = precision(piece.bounds.into_iter().chain([plane.point]), tolerance);
    let signed = |point| {
        let d = plane.distance(point);
        if d.abs() <= precision { 0. } else { d }
    };
    let mut inside = false;
    let mut outside = false;
    for point in piece.faces.iter().flatten().copied() {
        let d = signed(point);
        inside |= d < 0.;
        outside |= d > 0.;
    }
    if !inside {
        return Ok(Split::Outside);
    }
    if !outside {
        return Ok(Split::Inside(piece));
    }

    let mut faces = [Vec::new(), Vec::new()];
    let mut cap = Vec::new();
    for ring in &piece.faces {
        let mut halves = [Vec::new(), Vec::new()];
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let (da, db) = (signed(a), signed(b));
            if da <= 0. {
                halves[0].push(a);
            }
            if da >= 0. {
                halves[1].push(a);
            }
            if da == 0. {
                add_cap(&mut cap, a, precision);
            }
            if da * db < 0. {
                // Always use the same oriented edge so both incident faces
                // compute precisely the same intersection.
                let (start, end, d, next) = if da < 0. {
                    (a, b, da, db)
                } else {
                    (b, a, db, da)
                };
                let crossing = add(start, scale(sub(end, start), d / (d - next)));
                for half in &mut halves {
                    half.push(crossing);
                }
                add_cap(&mut cap, crossing, precision);
            }
        }
        for (side, half) in halves.into_iter().enumerate() {
            if half.len() >= 3 {
                faces[side].push(half);
            }
        }
    }
    if cap.len() < 3 {
        return Err(Error::Unresolved);
    }
    let center = scale(
        cap.iter().copied().fold([0.; 3], add),
        1. / cap.len() as f64,
    );
    let u = sub(cap[0], center);
    let v = cross(plane.normal, u);
    let angle = |point| {
        let offset = sub(point, center);
        dot(v, offset).atan2(dot(u, offset))
    };
    cap.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)));
    faces[0].push(cap.clone());
    cap.reverse();
    faces[1].push(cap);
    let [inside, outside] = faces;
    Ok(Split::Both(
        ConvexPiece::new(inside, tolerance)?,
        ConvexPiece::new(outside, tolerance)?,
    ))
}

fn add_cap(cap: &mut Vec<Point>, point: Point, precision: f64) {
    if !cap
        .iter()
        .any(|&other| norm(sub(point, other)) <= precision)
    {
        cap.push(point);
    }
}

fn verify_cells(
    boundary: &Union,
    pieces: &[ConvexPiece],
    tolerance: f64,
    budget: &mut usize,
) -> Result<(), Error> {
    let rebuilt = Union::new(pieces, tolerance)?;
    covers(boundary, &rebuilt, tolerance, budget)?;
    covers(&rebuilt, boundary, tolerance, budget)
}

fn covers(a: &Union, b: &Union, tolerance: f64, budget: &mut usize) -> Result<(), Error> {
    let precision = precision(a.bounds.into_iter().chain(b.bounds), tolerance);
    for face in &a.faces {
        let ring: Vec<_> = face.vertices.iter().map(|&i| a.vertices[i]).collect();
        let bounds_a = bounds(ring.iter().copied());
        let support = Plane {
            normal: face.normal,
            point: ring[0],
        };
        let mut fragments = vec![ring];
        for patch in &b.faces {
            charge(budget, patch.vertices.len())?;
            if !support.same_direction(
                Plane {
                    normal: patch.normal,
                    point: b.vertices[patch.vertices[0]],
                },
                precision,
            ) {
                continue;
            }
            let patch_ring: Vec<_> = patch.vertices.iter().map(|&i| b.vertices[i]).collect();
            if !clipping::bounds_overlap(bounds_a, bounds(patch_ring.iter().copied()), precision)
                || patch_ring
                    .iter()
                    .any(|&point| support.distance(point).abs() > precision)
            {
                continue;
            }
            let planes: Vec<_> = patch_ring
                .iter()
                .zip(patch_ring.iter().cycle().skip(1))
                .map(|(&point, &next)| {
                    let normal = cross(sub(next, point), patch.normal);
                    Plane {
                        point,
                        normal: scale(normal, norm(normal).recip()),
                    }
                })
                .collect();
            let mut remaining = Vec::new();
            let mut fragment_budget = MAX_INCIDENCES;
            for fragment in fragments {
                clipping::partition(
                    fragment,
                    &planes,
                    precision,
                    &mut remaining,
                    budget,
                    &mut fragment_budget,
                )?;
            }
            fragments = remaining;
            if fragments.is_empty() {
                break;
            }
        }
        if !fragments.is_empty() {
            return Err(Error::Unresolved);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
