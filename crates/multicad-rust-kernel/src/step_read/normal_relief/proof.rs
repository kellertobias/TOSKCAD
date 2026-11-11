use super::*;

fn near(a: Point, b: Point, epsilon: f64) -> bool {
    length(sub(a, b)) <= epsilon
}
fn normal(a: Point, b: Point, extent: f64, epsilon: f64) -> bool {
    dot(a, b) > 0. && length(cross(a, b)) * extent.max(1.) <= epsilon
}
pub(super) fn vertices(
    shell: &Shell<'_, '_>,
    graph: &Boundary,
    frame: Frame,
    epsilon: f64,
    work: &mut Work,
) -> Result<Vec<u32>> {
    require(shell.points.len() == graph.vertices.len())?;
    let mut points = shell
        .points
        .iter()
        .map(|(&id, &point)| (point, id))
        .collect::<Vec<_>>();
    points.sort_unstable_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    let mut used = BTreeSet::new();
    graph
        .vertices
        .iter()
        .map(|&point| {
            work.charge(1)?;
            let point = frame.world(point);
            let start = points.partition_point(|(p, _)| p[0] < point[0] - epsilon);
            let end = points.partition_point(|(p, _)| p[0] <= point[0] + epsilon);
            work.charge(end - start)?;
            let mut candidates = points[start..end]
                .iter()
                .filter(|(p, _)| near(*p, point, epsilon));
            let &(_, id) = candidates.next().ok_or(ReadError::Invalid)?;
            require(candidates.next().is_none() && used.insert(id))?;
            Ok(id)
        })
        .collect()
}
pub(super) fn circle(curve: Curve) -> Option<(Point, Point, f64, f64)> {
    match curve {
        Curve::Circle(c) => Some((c.center_mm, c.normal, c.radius_mm, std::f64::consts::TAU)),
        Curve::Semicircle(c) => Some((c.center_mm, c.normal, c.radius_mm, std::f64::consts::PI)),
        Curve::Arc(c) => Some((c.center_mm, c.normal, c.radius_mm, c.sweep_rad)),
        Curve::Line => None,
    }
}
fn curve_matches(
    actual: Curve,
    expected: ReliefCurve,
    forward: bool,
    frame: Frame,
    epsilon: f64,
) -> bool {
    let ReliefCurve::Profile { edge, z_mm } = expected else {
        return matches!(actual, Curve::Line);
    };
    let Some(radius) = edge.radius_mm() else {
        return matches!(actual, Curve::Line);
    };
    let Some((center, axis, r, sweep)) = circle(actual) else {
        return false;
    };
    let (xy, clockwise) = match edge {
        ProfileEdge::Circle {
            center, clockwise, ..
        }
        | ProfileEdge::Arc {
            center, clockwise, ..
        } => (center, clockwise),
        ProfileEdge::Line { .. } => unreachable!("radius implies circular edge"),
    };
    let axis = axis.map(|v| if forward { v } else { -v });
    let expected_axis = frame.axes[2].map(|v| if clockwise { -v } else { v });
    near(center, frame.world([xy[0], xy[1], z_mm]), epsilon)
        && (r - radius).abs() <= epsilon
        && normal(axis, expected_axis, radius, epsilon)
        && (sweep - edge.signed_sweep_rad().expect("circular sweep").abs()).abs() * radius
            <= epsilon
        && matches!(
            (actual, edge),
            (Curve::Circle(_), ProfileEdge::Circle { .. })
                | (
                    Curve::Arc(_) | Curve::Semicircle(_),
                    ProfileEdge::Arc { .. }
                )
        )
}
pub(super) fn edges(
    shell: &Shell<'_, '_>,
    graph: &Boundary,
    vertices: &[u32],
    frame: Frame,
    epsilon: f64,
    work: &mut Work,
) -> Result<Vec<(u32, bool)>> {
    require(shell.edges.len() == graph.edges.len())?;
    let mut by_ends: BTreeMap<[u32; 2], Vec<u32>> = BTreeMap::new();
    for (&id, edge) in &shell.edges {
        work.charge(1)?;
        let mut ends = edge.vertices;
        ends.sort_unstable();
        by_ends.entry(ends).or_default().push(id);
    }
    let mut used = BTreeSet::new();
    graph
        .edges
        .iter()
        .map(|expected| {
            let ends = expected.vertices.map(|v| vertices[v]);
            let mut key = ends;
            key.sort_unstable();
            let candidates = by_ends.get(&key).ok_or(ReadError::Invalid)?;
            work.charge(candidates.len())?;
            let mut matches = Vec::new();
            for &id in candidates {
                let edge = &shell.edges[&id];
                for forward in [true, false] {
                    if (if forward {
                        edge.vertices
                    } else {
                        [edge.vertices[1], edge.vertices[0]]
                    }) == ends
                        && curve_matches(edge.curve, expected.curve, forward, frame, epsilon)
                    {
                        matches.push((id, forward));
                    }
                }
            }
            let [(id, forward)] = matches[..] else {
                return Err(ReadError::Invalid);
            };
            require(used.insert(id))?;
            Ok((id, forward))
        })
        .collect()
}
fn support(
    shell: &Shell<'_, '_>,
    id: u32,
    actual: &Face,
    expected: Surface,
    frame: Frame,
    epsilon: f64,
    extent: f64,
) -> Result<()> {
    let [Value::String(_), _, surface, forward] = shell.args(id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    match (actual, expected) {
        (
            Face::Planar {
                origin,
                normal: axis,
                ..
            },
            Surface::Plane { origin_mm, u, v },
        ) => {
            let expected_axis = unit(frame.vector(cross(u, v)))?;
            require(
                normal(*axis, expected_axis, extent, epsilon)
                    && dot(sub(frame.world(origin_mm), *origin), *axis).abs() <= epsilon,
            )
        }
        (
            _,
            Surface::Cylinder {
                origin_mm,
                axis,
                u: _,
                radius_mm,
                outward,
            },
        ) => {
            let [Value::String(_), placement, Value::Number(radius)] =
                shell.args(reference(surface)?, "CYLINDRICAL_SURFACE")?
            else {
                return Err(ReadError::Invalid);
            };
            let plane = shell.placement(reference(placement)?)?;
            let expected_axis = unit(frame.vector(axis))?;
            require(
                sense(forward)? == outward
                    && (radius - radius_mm).abs() <= epsilon
                    && length(cross(plane.normal, expected_axis)) * extent.max(radius_mm)
                        <= epsilon
                    && length(cross(
                        sub(frame.world(origin_mm), plane.origin),
                        plane.normal,
                    )) <= epsilon,
            )
        }
        _ => Err(ReadError::Invalid),
    }
}
pub(super) fn verify(
    shell: &Shell<'_, '_>,
    ids: &[u32],
    faces: &[Face],
    readback: &Readback,
    relief: &Relief,
    frame: Frame,
    work: &mut Work,
) -> Result<()> {
    let graph = relief.boundary();
    require(ids.len() == graph.faces.len())?;
    let epsilon = shell.tolerance / 16.;
    let vertices = vertices(shell, graph, frame, epsilon, work)?;
    let edges = edges(shell, graph, &vertices, frame, epsilon, work)?;
    let mut actual = BTreeMap::new();
    for (index, &id) in ids.iter().enumerate() {
        let mut wires = loops(shell, id, work)?
            .into_iter()
            .map(cyclic)
            .collect::<Vec<_>>();
        wires.sort_unstable();
        require(actual.insert(wires, index).is_none())?;
    }
    let bounds = relief.bounds();
    let extent = length(sub(bounds[1], bounds[0]));
    for face in &graph.faces {
        let mut wires = face
            .loops
            .iter()
            .map(|wire| {
                wire.iter()
                    .map(|use_| {
                        let (id, forward) = edges[use_.edge];
                        (id, forward == use_.forward)
                    })
                    .collect::<Wire>()
            })
            .map(cyclic)
            .collect::<Vec<_>>();
        work.charge(wires.iter().map(Vec::len).sum())?;
        wires.sort_unstable();
        let index = actual.remove(&wires).ok_or(ReadError::Invalid)?;
        support(
            shell,
            ids[index],
            &faces[index],
            face.surface,
            frame,
            epsilon,
            extent,
        )?;
    }
    require(actual.is_empty())?;
    let volume_error = shell.tolerance * relief.surface_area_mm2().max(1.);
    require((readback.volume_mm3 - relief.volume_mm3()).abs() <= volume_error)
}
