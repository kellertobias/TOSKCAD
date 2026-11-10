use super::*;
use crate::normal_sweep::{self, Edge, Sweep};

type Point2 = [f64; 2];
type Crossing = (Point2, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileFace {
    StartCap,
    EndCap,
    Edge(usize),
    Partition,
}

#[derive(Clone, Debug)]
pub struct ProfilePiece {
    pub piece: ConvexPiece,
    pub source_faces: Vec<ProfileFace>,
}

struct SweepFrame {
    origin: Point,
    transverse: Point,
    displacement: Point,
}

impl ConvexPiece {
    /// Extrude a straight simple ring in a vertical plane along its horizontal
    /// normal. Partition at authored transverse coordinates so every added
    /// wall is vertical and stays planar under a nonlinear Z remap.
    pub fn from_vertical_profile_sweep(
        ring_world: &[Point],
        displacement_world: Point,
        tolerance_mm: f64,
    ) -> Result<Vec<ProfilePiece>, Error> {
        validate_tolerance(tolerance_mm)?;
        if ring_world.len() > MAX_FACE_VERTICES {
            return Err(Error::Limit);
        }
        if ring_world.len() < 3
            || ring_world
                .iter()
                .flatten()
                .chain(&displacement_world)
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
        {
            return Err(Error::Invalid);
        }
        let [x, y, z] = displacement_world;
        let height = x.hypot(y);
        let magnitude = ring_world
            .iter()
            .flatten()
            .copied()
            .map(f64::abs)
            .fold(height.max(1.), f64::max);
        let resolution = (8. * f64::EPSILON * magnitude).max(tolerance_mm * 1e-3);
        if height <= 2. * tolerance_mm || z.abs() > resolution {
            return Err(Error::Invalid);
        }
        let normal = [x / height, y / height, 0.];
        let frame = SweepFrame {
            origin: ring_world[0],
            transverse: [-normal[1], normal[0], 0.],
            displacement: [x, y, 0.],
        };
        let mut profile = Vec::with_capacity(ring_world.len());
        for &point in ring_world {
            let relative = sub(point, frame.origin);
            if dot(relative, normal).abs() > resolution {
                return Err(Error::Invalid);
            }
            profile.push([dot(relative, frame.transverse), relative[2]]);
        }
        let stations = canonical_stations(&mut profile, resolution, tolerance_mm)?;
        let edges: Vec<_> = profile
            .iter()
            .copied()
            .zip(profile.iter().copied().cycle().skip(1))
            .map(|(a, b)| Edge::Line { a, b })
            .collect();
        let mut certified_edges = edges.clone();
        if normal_sweep::signed_area(&certified_edges) < 0. {
            certified_edges = certified_edges
                .into_iter()
                .rev()
                .map(Edge::reversed)
                .collect();
        }
        let certified = Sweep::new(
            vec![certified_edges],
            height,
            tolerance_mm,
            2. * tolerance_mm,
        )
        .map_err(|error| match error {
            normal_sweep::Error::Limit => Error::Limit,
            _ => Error::Invalid,
        })?;

        let mut pieces = Vec::new();
        let mut area = 0.;
        let mut remaining_pieces = MAX_PIECES;
        for band in stations.windows(2) {
            let [low, high] = [band[0], band[1]];
            let mut crossings: Vec<_> = edges
                .iter()
                .enumerate()
                .filter_map(|(source, edge)| {
                    let [mut a, mut b] = [edge.start(), edge.end()];
                    if a[0] > b[0] {
                        std::mem::swap(&mut a, &mut b);
                    }
                    (a[0] <= low && b[0] >= high).then(|| {
                        let z_at = |x| {
                            if x == b[0] {
                                b[1]
                            } else {
                                a[1] + (b[1] - a[1]) * ((x - a[0]) / (b[0] - a[0]))
                            }
                        };
                        ([z_at(low), z_at(high)], source)
                    })
                })
                .collect();
            for &[
                ([lower_left, lower_right], lower_source),
                ([upper_left, upper_right], upper_source),
            ] in order_crossings(&mut crossings)?
            {
                charge(&mut remaining_pieces, 1)?;
                let mut left = [lower_left, upper_left];
                let mut right = [lower_right, upper_right];
                collapse_corner(&mut left, resolution)?;
                collapse_corner(&mut right, resolution)?;
                area += (high - low) * ((left[1] - left[0]) + (right[1] - right[0])) * 0.5;
                let piece = frame.extrude_strip(
                    [
                        [low, left[0]],
                        [high, right[0]],
                        [high, right[1]],
                        [low, left[1]],
                    ],
                    [
                        ProfileFace::Edge(lower_source),
                        vertical_face(&edges, high, right, resolution),
                        ProfileFace::Edge(upper_source),
                        vertical_face(&edges, low, left, resolution),
                    ],
                    tolerance_mm,
                )?;
                pieces.push(piece);
            }
        }
        let perimeter = edges.iter().map(|edge| edge.length_mm()).sum::<f64>();
        verify_area(area, certified.area_mm2(), resolution * perimeter)?;
        Ok(pieces)
    }
}

fn order_crossings(crossings: &mut [Crossing]) -> Result<&[[Crossing; 2]], Error> {
    crossings.sort_unstable_by(|a, b| (a.0[0] + a.0[1]).total_cmp(&(b.0[0] + b.0[1])));
    let (pairs, remainder) = crossings.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(Error::Unresolved);
    }
    Ok(pairs)
}

fn verify_area(actual: f64, expected: f64, tolerance: f64) -> Result<(), Error> {
    if (actual - expected).abs() > tolerance {
        return Err(Error::Unresolved);
    }
    Ok(())
}

fn canonical_stations(
    profile: &mut [Point2],
    resolution: f64,
    tolerance: f64,
) -> Result<Vec<f64>, Error> {
    let mut indices: Vec<_> = (0..profile.len()).collect();
    indices.sort_unstable_by(|&a, &b| profile[a][0].total_cmp(&profile[b][0]));
    let mut stations = Vec::new();
    for index in indices {
        let coordinate = profile[index][0];
        if let Some(&last) = stations.last() {
            let width: f64 = coordinate - last;
            if width <= resolution {
                profile[index][0] = last;
                continue;
            }
            if width <= tolerance {
                return Err(Error::Unresolved);
            }
        }
        stations.push(coordinate);
    }
    Ok(stations)
}

fn collapse_corner([lower, upper]: &mut [f64; 2], resolution: f64) -> Result<(), Error> {
    if *upper < *lower - resolution {
        return Err(Error::Unresolved);
    }
    if (*upper - *lower).abs() <= resolution {
        *upper = *lower;
    }
    Ok(())
}

fn vertical_face(
    edges: &[Edge],
    station: f64,
    [lower, upper]: [f64; 2],
    resolution: f64,
) -> ProfileFace {
    edges
        .iter()
        .position(|edge| {
            let [a, b] = [edge.start(), edge.end()];
            a[0] == station
                && b[0] == station
                && a[1].min(b[1]) <= lower + resolution
                && a[1].max(b[1]) >= upper - resolution
        })
        .map_or(ProfileFace::Partition, ProfileFace::Edge)
}

impl SweepFrame {
    fn extrude_strip(
        &self,
        corners: [Point2; 4],
        sources: [ProfileFace; 4],
        tolerance: f64,
    ) -> Result<ProfilePiece, Error> {
        let point = |[x, z]: Point2| {
            add(
                self.origin,
                [self.transverse[0] * x, self.transverse[1] * x, z],
            )
        };
        let mut start = Vec::with_capacity(4);
        let mut sides = Vec::with_capacity(4);
        for index in 0..4 {
            let [a, b] = [corners[index], corners[(index + 1) % 4]];
            if a != b {
                start.push(point(a));
                sides.push(sources[index]);
            }
        }
        let end: Vec<_> = start
            .iter()
            .copied()
            .map(|p| add(p, self.displacement))
            .collect();
        let mut faces = vec![start.iter().rev().copied().collect(), end.clone()];
        for a in 0..start.len() {
            let b = (a + 1) % start.len();
            faces.push(vec![start[a], start[b], end[b], end[a]]);
        }
        let piece = ConvexPiece::new(faces, tolerance)?;
        let source_faces = [ProfileFace::StartCap, ProfileFace::EndCap]
            .into_iter()
            .chain(sides)
            .collect();
        Ok(ProfilePiece {
            piece,
            source_faces,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_unpaired_crossings_inverted_corners_and_area_mismatch() {
        assert_eq!(
            order_crossings(&mut [([0., 1.], 0)]),
            Err(Error::Unresolved)
        );
        assert_eq!(
            collapse_corner(&mut [1., 0.], 1e-10),
            Err(Error::Unresolved)
        );
        assert_eq!(verify_area(2., 1., 1e-10), Err(Error::Unresolved));
    }

    #[test]
    fn refuse_out_of_range_world_sweep_end() {
        let x = MAX_COORDINATE - 0.5;
        let ring = [[x, 0., 0.], [x, 2., 0.], [x, 2., 2.], [x, 0., 2.]];
        assert_eq!(
            ConvexPiece::from_vertical_profile_sweep(&ring, [1., 0., 0.], 1e-7).unwrap_err(),
            Error::Invalid,
        );
    }
}
