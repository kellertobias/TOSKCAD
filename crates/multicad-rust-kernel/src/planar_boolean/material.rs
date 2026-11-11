use super::*;

mod certified;
mod intersection;

/// Positive cells independently checked against a certified planar boundary.
#[derive(Clone, Debug)]
pub struct CutMaterial {
    pub(super) boundary: Union,
    pub(super) pieces: Vec<ConvexPiece>,
}

impl CutMaterial {
    /// Certify supplied positive cells against an existing authoritative
    /// boundary. Preserve every original boundary face and source label;
    /// cell partition walls never become published ancestry.
    pub fn certify_existing(
        boundary: &Union,
        pieces: &[ConvexPiece],
        tolerance: f64,
    ) -> Result<Self, Error> {
        Self::certify_existing_with_interrupt(boundary, pieces, tolerance, None)
    }

    pub fn boundary(&self) -> &Union {
        &self.boundary
    }

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
        Self::recover_with_interrupt(boundary, stock, tools, tolerance, None)
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

fn subtract_cell_with_interrupt(
    piece: ConvexPiece,
    tool: &ConvexPiece,
    tolerance: f64,
    budget: &mut usize,
    output: &mut Vec<ConvexPiece>,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), Error> {
    checkpoint(interrupt)?;
    if !clipping::bounds_overlap(piece.bounds, tool.bounds, tolerance) {
        output.push(piece);
        return Ok(());
    }
    let mut interior = piece.clone();
    let mut outside = Vec::new();
    for &plane in &tool.planes {
        checkpoint(interrupt)?;
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
    verify_cells_with_interrupt(boundary, pieces, tolerance, budget, None)
}

fn verify_cells_with_interrupt(
    boundary: &Union,
    pieces: &[ConvexPiece],
    tolerance: f64,
    budget: &mut usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), Error> {
    validate_cells(pieces, tolerance)?;
    let rebuilt = Union::from_boundary(clipping::exterior_with_interrupt(
        pieces, tolerance, budget, interrupt,
    )?)?;
    checkpoint(interrupt)?;
    covers_with_interrupt(boundary, &rebuilt, tolerance, budget, interrupt)?;
    covers_with_interrupt(&rebuilt, boundary, tolerance, budget, interrupt)
}

fn validate_cells(pieces: &[ConvexPiece], tolerance: f64) -> Result<(), Error> {
    validate_tolerance(tolerance)?;
    if pieces.is_empty() {
        return Err(Error::Invalid);
    }
    if pieces.len() > MAX_PIECES {
        return Err(Error::Limit);
    }
    Ok(())
}

fn covers_with_interrupt(
    a: &Union,
    b: &Union,
    tolerance: f64,
    budget: &mut usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), Error> {
    let a_charts = convex_charts(a, budget, interrupt)?;
    let b_charts = convex_charts(b, budget, interrupt)?;
    let precision = precision(a.bounds.into_iter().chain(b.bounds), tolerance);
    for face in &a_charts {
        checkpoint(interrupt)?;
        let mut fragments = vec![face.ring.clone()];
        for patch in &b_charts {
            checkpoint(interrupt)?;
            charge(budget, patch.ring.len())?;
            if !face.support.same_direction(patch.support, precision) {
                continue;
            }
            if !clipping::bounds_overlap(face.bounds, patch.bounds, precision)
                || patch
                    .ring
                    .iter()
                    .any(|&point| face.support.distance(point).abs() > precision)
            {
                continue;
            }
            let planes: Vec<_> = patch
                .ring
                .iter()
                .zip(patch.ring.iter().cycle().skip(1))
                .map(|(&point, &next)| {
                    let normal = cross(sub(next, point), patch.support.normal);
                    Plane {
                        point,
                        normal: scale(normal, norm(normal).recip()),
                    }
                })
                .collect();
            let mut remaining = Vec::new();
            let mut fragment_budget = MAX_INCIDENCES;
            for fragment in fragments {
                checkpoint(interrupt)?;
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

struct ConvexChart {
    ring: Vec<Point>,
    support: Plane,
    bounds: [Point; 2],
    ordinal: usize,
}

/// Exact planar subdivisions for clipping/coverage only. No new boundary
/// samples or ancestry are introduced; all charts retain the authored face.
fn convex_charts(
    boundary: &Union,
    budget: &mut usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<ConvexChart>, Error> {
    let mut charts = Vec::new();
    for (ordinal, face) in boundary.faces.iter().enumerate() {
        checkpoint(interrupt)?;
        let count = face.vertices.len();
        charge(budget, count)?;
        let rings: Vec<Vec<Point>> = if shell::is_concave(&boundary.vertices, face) {
            charge(budget, count.checked_mul(count).ok_or(Error::Limit)?)?;
            shell::triangulate_face(&boundary.vertices, face)
                .map_err(|_| Error::Unresolved)?
                .into_iter()
                .map(|triangle| triangle.map(|i| boundary.vertices[i]).to_vec())
                .collect()
        } else {
            vec![
                face.vertices
                    .iter()
                    .map(|&i| boundary.vertices[i])
                    .collect(),
            ]
        };
        for ring in rings {
            charge(budget, ring.len())?;
            let support = Plane {
                normal: face.normal,
                point: ring[0],
            };
            charts.push(ConvexChart {
                bounds: bounds(ring.iter().copied()),
                ring,
                support,
                ordinal,
            });
        }
    }
    Ok(charts)
}

#[cfg(test)]
mod tests;
