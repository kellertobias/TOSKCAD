//! Pair authored circular and straight axial cutters in the drawer chart.
use super::*;
use multicad_rust_kernel::warped_union::{AxialCut, ConvexRecess};

const ANGULAR_TOLERANCE: f64 = 1e-10;
const MAX_RECESS_SIDES: usize = 64;

pub(super) struct Classified {
    pub(super) planar: Vec<usize>,
    pub(super) cuts: Vec<AxialCut>,
}

#[derive(Clone, Copy)]
struct Round {
    frame: PlaneFrame,
    axis: Point,
    length: f64,
    radius: f64,
}

impl Round {
    fn read(solid: &Solid) -> Option<Self> {
        let (cylinder, frame) = solid.axial_cylinder_frame()?;
        Some(Self {
            frame,
            axis: frame.normal(),
            length: cylinder.height().as_f64(),
            radius: cylinder.circle().radius().as_f64(),
        })
    }

    fn at(self, level: f64) -> Point {
        std::array::from_fn(|i| self.frame.origin[i] + self.axis[i] * level)
    }

    fn level(self, point: Point) -> f64 {
        dot(subtract(point, self.frame.origin), self.axis)
    }

    fn section(self, point: Point) -> [f64; 2] {
        let offset = subtract(point, self.frame.origin);
        [dot(offset, self.frame.u), dot(offset, self.frame.v)]
    }

    fn coaxial_boss(self, boss: &AuthoredBoss, tolerance: f64) -> bool {
        let [a, b] = boss.ends.map(|point| self.level(point));
        self.radius < boss.radius - tolerance
            && a.min(b).max(0.) + tolerance < a.max(b).min(self.length)
            && boss.ends.iter().all(|&point| {
                norm(cross(subtract(point, self.frame.origin), self.axis)) <= tolerance
            })
    }

    fn cut(self, source: usize, recess: Option<ConvexRecess>) -> AxialCut {
        AxialCut {
            start_mm: self.frame.origin,
            end_mm: self.at(self.length),
            radius_mm: self.radius,
            source,
            wall_source_face: 2,
            cap_source_faces: [0, 1],
            recess,
        }
    }
}

fn intersects(a: Bounds, b: Bounds, tolerance: f64) -> bool {
    (0..3).all(|i| a.min[i].max(b.min[i]) <= a.max[i].min(b.max[i]) + tolerance)
}

fn ambiguous() -> GeometryError {
    error(
        "quarter-round Boolean",
        GeometryErrorKind::Unsupported,
        "An axial cutter has more than one matching boss or widening profile",
    )
}

/// Classify all circular tools and pair any unique exact widening. Source
/// indices refer to the input tools, for the chart to convert to provenance.
pub(super) fn classify(
    tools: &[Solid],
    bosses: &[AuthoredBoss],
    tolerance: f64,
) -> Result<Classified, GeometryError> {
    let rounds: Vec<_> = tools.iter().map(Round::read).collect();
    let mut used: Vec<_> = rounds.iter().map(Option::is_some).collect();
    let mut cuts = Vec::new();
    for (source, round) in rounds.into_iter().enumerate() {
        let Some(round) = round else { continue };
        let mut matching = bosses
            .iter()
            .filter(|boss| round.coaxial_boss(boss, tolerance));
        let boss = matching.next();
        if matching.next().is_some() {
            return Err(ambiguous());
        }
        let recess = if let Some(boss) = boss {
            let region = boss.bounds();
            let mut recess = None;
            for (index, tool) in tools.iter().enumerate() {
                if used[index] || !intersects(region, tool.bounds()?, tolerance) {
                    continue;
                }
                if let Some(candidate) = profile(tool, index, round, boss, tolerance)?
                    && recess.replace(candidate).is_some()
                {
                    return Err(ambiguous());
                }
            }
            if let Some(recess) = &recess {
                used[recess.source] = true;
            }
            recess
        } else {
            None
        };
        cuts.push(round.cut(source, recess));
    }
    let planar = used
        .iter()
        .enumerate()
        .filter_map(|(index, used)| (!used).then_some(index))
        .collect();
    Ok(Classified { planar, cuts })
}

#[derive(Clone, Copy)]
struct FacePlane {
    ordinal: u32,
    point: Point,
    normal: Point,
}

fn profile(
    solid: &Solid,
    source: usize,
    round: Round,
    boss: &AuthoredBoss,
    tolerance: f64,
) -> Result<Option<ConvexRecess>, GeometryError> {
    let faces = solid.faces()?;
    if !(5..=MAX_RECESS_SIDES + 2).contains(&faces.len()) {
        return Ok(None);
    }
    let Some(planes) = faces
        .iter()
        .map(|face| {
            let (point, normal) = face.plane?;
            Some(FacePlane {
                ordinal: face.index,
                point,
                normal,
            })
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    let mut caps = planes
        .iter()
        .copied()
        .filter(|face| norm(cross(round.axis, face.normal)) <= ANGULAR_TOLERANCE);
    let (Some(start), Some(end), None) = (caps.next(), caps.next(), caps.next()) else {
        return Ok(None);
    };
    let levels = [round.level(start.point), round.level(end.point)];
    let [low, high] = [levels[0].min(levels[1]), levels[0].max(levels[1])];
    let [a, b] = boss.ends.map(|point| round.level(point));
    // The recess crosses one boss cap and terminates inside its axial span.
    // The final shell independently certifies which cap is exposed.
    let contains = |level| low + tolerance < level && level < high - tolerance;
    if contains(a) == contains(b)
        || low.max(0.) + tolerance >= high.min(round.length)
        || high - low <= tolerance
    {
        return Ok(None);
    }
    let travel = subtract(round.at(levels[1]), round.at(levels[0]));
    if dot(start.normal, travel) >= -tolerance || dot(end.normal, travel) <= tolerance {
        return Ok(None);
    }

    let topology = match solid.rust_topology() {
        Some(topology) => topology?,
        None => return Ok(None),
    };
    let lines = topology
        .elements
        .iter()
        .filter_map(|element| {
            let Some(AnalyticGeometry::Line { a_mm, b_mm }) = element.geometry else {
                return None;
            };
            Some((element.key.ordinal, [a_mm, b_mm]))
        })
        .collect();
    let [Some(rim), Some(end_rim)] =
        [start, end].map(|cap| cap_ring(solid, cap.ordinal, round.axis, &lines, tolerance))
    else {
        return Ok(None);
    };
    if rim.len() + 2 != faces.len()
        || !same_section(&rim, &end_rim, travel, tolerance)
        || !fits(&rim, round, boss.radius, tolerance)
    {
        return Ok(None);
    }
    let mut wall_source_faces = Vec::with_capacity(rim.len());
    for (&a, &b) in rim.iter().zip(rim.iter().cycle().skip(1)) {
        let outward = cross(subtract(b, a), round.axis);
        let mut side = planes.iter().filter(|face| {
            dot(round.axis, face.normal).abs() <= ANGULAR_TOLERANCE
                && dot(outward, face.normal) > 0.
                && [a, b].into_iter().all(|endpoint| {
                    dot(face.normal, subtract(endpoint, face.point)).abs() <= tolerance
                })
        });
        let Some(face) = side.next() else {
            return Ok(None);
        };
        let ordinal = face.ordinal as usize;
        if side.next().is_some() || wall_source_faces.contains(&ordinal) {
            return Ok(None);
        }
        wall_source_faces.push(ordinal);
    }
    Ok(Some(ConvexRecess {
        rim_mm: rim,
        travel_mm: travel,
        source,
        wall_source_faces,
        cap_source_faces: [start.ordinal as usize, end.ordinal as usize],
    }))
}

fn cap_ring(
    solid: &Solid,
    face: u32,
    axis: Point,
    lines: &std::collections::BTreeMap<u32, [Point; 2]>,
    tolerance: f64,
) -> Option<Vec<Point>> {
    let loops = crate::rust_display::planar_loops(solid, face);
    let [ring] = loops.as_slice() else {
        return None;
    };
    let segments: Vec<_> = ring
        .iter()
        .map(|&(id, reversed)| {
            let [a, b] = *lines.get(&id)?;
            Some(if reversed { [b, a] } else { [a, b] })
        })
        .collect::<Option<_>>()?;
    if segments.len() < 3
        || segments
            .iter()
            .zip(segments.iter().cycle().skip(1))
            .any(|(a, b)| norm(subtract(a[1], b[0])) > tolerance)
    {
        return None;
    }
    let mut points: Vec<_> = segments.into_iter().map(|[start, _]| start).collect();
    if winding(&points, axis) < 0. {
        points.reverse();
    }
    Some(points)
}

fn same_section(base: &[Point], top: &[Point], travel: Point, tolerance: f64) -> bool {
    if base.len() != top.len() {
        return false;
    }
    let translated = |p: Point| std::array::from_fn(|i| p[i] + travel[i]);
    let near = |a, b| norm(subtract(a, b)) <= tolerance;
    top.iter().enumerate().any(|(offset, &point)| {
        near(translated(base[0]), point)
            && base
                .iter()
                .zip(top.iter().cycle().skip(offset))
                .all(|(&a, &b)| near(translated(a), b))
    })
}

fn fits(rim: &[Point], round: Round, boss_radius: f64, tolerance: f64) -> bool {
    let points: Vec<_> = rim.iter().map(|&point| round.section(point)).collect();
    let determinant = |[a, b]: [f64; 2], [c, d]: [f64; 2]| a * d - b * c;
    points
        .iter()
        .all(|&[x, y]| x.hypot(y) < boss_radius - tolerance)
        && points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .all(|(&a, &b)| {
                let edge = [b[0] - a[0], b[1] - a[1]];
                let length = edge[0].hypot(edge[1]);
                length > tolerance
                    && determinant(a, b) > (round.radius + tolerance) * length
                    && points.iter().all(|p| {
                        determinant(edge, [p[0] - a[0], p[1] - a[1]]) >= -tolerance * length
                    })
            })
}
