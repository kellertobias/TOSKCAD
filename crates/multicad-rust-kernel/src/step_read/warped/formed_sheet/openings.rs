//! Match each interior flange loop with the opposing loop through its walls.
use super::*;

pub(super) struct Opening {
    walls: [usize; 2],
    origin: Point,
    u: Point,
    v: Point,
    through: Point,
    outline: Vec<(Edge, usize)>,
}

impl Opening {
    pub(super) fn spec(&self) -> SheetFlangeOpening<'_> {
        SheetFlangeOpening {
            walls: self.walls,
            origin_mm: self.origin,
            u: self.u,
            v: self.v,
            through_mm: self.through,
            outline: &self.outline,
        }
    }
}

pub(super) fn read(
    wire: &Wire<'_, '_, '_>,
    incidence: &Incidence,
    chart: Chart,
    section: &[(Edge, usize)],
) -> Result<Vec<Opening>> {
    let (_, sides) = section.split_last().ok_or(ReadError::Invalid)?;
    let hosts = sides
        .iter()
        .enumerate()
        .map(|(i, &(_, face))| (face, i))
        .collect::<BTreeMap<_, _>>();
    let mut openings = Vec::new();
    let mut used = BTreeSet::new();
    for (index, &(section_edge, face)) in sides.iter().enumerate() {
        for (hole, ring) in wire.boundary.faces[face].holes.iter().enumerate() {
            if !used.insert((face, hole)) {
                continue;
            }
            let [first, ..] =
                <[_; 4]>::try_from(ring.as_slice()).map_err(|_| ReadError::Unsupported)?;
            if !matches!(section_edge, Edge::Line { .. }) {
                return Err(ReadError::Unsupported);
            }
            let exit = other_host(wire, incidence, &hosts, face, first.0)?;
            let exit_hole = exit.hole.ok_or(ReadError::Invalid)?;
            if !used.insert((exit.face, exit_hole)) {
                return Err(ReadError::Invalid);
            }
            let end = &wire.boundary.faces[exit.face].holes[exit_hole];
            let normal = wire.normal(face)?;
            if dot(normal, wire.normal(exit.face)?) > -1. + 1e-10 || end.len() != ring.len() {
                return Err(ReadError::Unsupported);
            }
            let [a, b] = wire.points(first);
            let depth = dot(sub(wire.points(end[0])[0], a), normal);
            if depth >= -wire.reader.tolerance {
                return Err(ReadError::Invalid);
            }
            let through = normal.map(|n| n * depth);
            if !paired_outline(wire, ring, end, through) {
                return Err(ReadError::Unsupported);
            }
            let circular = match wire.boundary.curves[&first.0].1 {
                TrimCurve::Line => None,
                TrimCurve::Arc(arc) => {
                    let ellipse = arc.ellipse();
                    Some((ellipse.center_mm(), ellipse.semiaxes_mm()[0]))
                }
            };
            let (origin, direction) =
                circular.map_or((a, sub(b, a)), |(center, _)| (center, sub(a, center)));
            let v = unit(cross(normal, direction))?;
            let u = cross(v, normal);
            let outline = if let Some((_, radius_mm)) = circular {
                vec![(
                    Edge::Circle {
                        center: [0.; 2],
                        radius_mm,
                        clockwise: false,
                    },
                    0,
                )]
            } else {
                reverse(ring)
                    .map(|usage| {
                        wire.outline(usage, [origin, u, v], std::convert::identity)
                            .map(|edge| (edge, 0))
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            openings.push(Opening {
                walls: [index, hosts[&exit.face]],
                origin: chart.local(origin),
                u: chart.vector(u),
                v: chart.vector(v),
                through: chart.vector(through),
                outline,
            });
        }
    }
    Ok(openings)
}

// One rim edge suffices to derive a candidate tunnel. The final replay
// verifies every other wall and both complete, directed host wires.
fn other_host(
    wire: &Wire<'_, '_, '_>,
    incidence: &Incidence,
    hosts: &BTreeMap<usize, usize>,
    entry: usize,
    edge: u32,
) -> Result<axial_cuts::Use> {
    let wall = incidence.adjacent(edge, entry)?;
    for &(edge, _) in &wire.boundary.faces[wall.face].loop_ {
        let next = incidence.adjacent(edge, wall.face)?;
        if next.face != entry && next.hole.is_some() && hosts.contains_key(&next.face) {
            return Ok(next);
        }
    }
    Err(ReadError::Unsupported)
}
