//! Exact planar cells. Splits are derived, never authored edits.
use super::*;

const MAX_SEGMENTS: usize = 8192;

fn fail(message: &str) -> QueryError {
    QueryError {
        message: message.into(),
    }
}

/// Extract bounded cells from exact line segments, including shared junctions
/// and interior crossings. Input IDs are retained on every derived segment.
/// Construction curves must be removed by the caller. Dangling edges do not
/// form boundaries. Overlaps and tolerance-ambiguous junctions require repair.
/// This deliberately refuses curved geometry rather than approximating it.
pub fn extract_line_cells(
    edges: &[Edge],
    tolerance_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Extraction, QueryError> {
    if edges
        .iter()
        .any(|e| !matches!(e.curve, ExactCurve::Line { .. }))
    {
        return Err(fail("Line cell extraction requires exact straight edges"));
    }
    extract_cells(edges, tolerance_mm, &AnalyticQueries, interrupt)
}

/// Exact arrangement with a worker-owned query service for curved boundaries.
/// No tessellated coordinates participate in splitting, area or containment.
pub fn extract_cells(
    edges: &[Edge],
    tolerance_mm: f64,
    queries: &dyn RegionQueries,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Extraction, QueryError> {
    if !tolerance_mm.is_finite() || !(1e-10..=1e-3).contains(&tolerance_mm) {
        return Err(fail("Invalid arrangement tolerance"));
    }
    if edges.len() > queries.maximum_curves(edges.iter().any(|e| e.curve.is_spline())) {
        return Err(fail("Arrangement exceeds the exact curve budget"));
    }
    let mut ids = BTreeSet::new();
    let mut cuts = Vec::with_capacity(edges.len());
    for edge in edges {
        check_interrupt(interrupt)?;
        let (a, b) = edge.curve.ends();
        if !ids.insert(edge.authored)
            || a.into_iter().chain(b).any(|v| !v.is_finite())
            || (!edge.curve.closed(tolerance_mm) && distance(a, b) <= tolerance_mm)
        {
            return Err(fail("Invalid, duplicate or degenerate arrangement edge"));
        }
        let hits = queries.self_intersections(&edge.curve, tolerance_mm)?;
        if hits.overlap || !hits.points.is_empty() {
            return Err(fail("Self-intersecting arrangement edge requires repair"));
        }
        cuts.push(vec![(0., a), (1., b)]);
    }
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            check_interrupt(interrupt)?;
            let hits = queries.intersections(&edges[i].curve, &edges[j].curve, tolerance_mm)?;
            if hits.overlap {
                return Err(fail("Overlapping curves require explicit repair"));
            }
            for point in hits.points {
                for index in [i, j] {
                    let curve = &edges[index].curve;
                    let (a, b) = curve.ends();
                    let t = if distance(point, a) <= tolerance_mm {
                        0.
                    } else if distance(point, b) <= tolerance_mm {
                        1.
                    } else if let ExactCurve::Line { a, b } = curve {
                        dot(sub(point, *a), sub(*b, *a)) / dot(sub(*b, *a), sub(*b, *a))
                    } else {
                        let projections: Vec<_> = queries
                            .project_parameters(curve, point, tolerance_mm)?
                            .into_iter()
                            .filter(|p| p.distance_mm <= tolerance_mm)
                            .collect();
                        if projections.len() != 1 {
                            return Err(fail("Ambiguous intersection parameter requires repair"));
                        }
                        projections[0].parameter
                    };
                    if !t.is_finite() || !(0. ..=1.).contains(&t) {
                        return Err(fail("Invalid intersection parameter"));
                    }
                    cuts[index].push((t, point));
                }
            }
        }
    }
    let mut nodes: Vec<Point> = vec![];
    let mut segments: Vec<(Edge, [usize; 2])> = vec![];
    let mut tangents: Vec<[[f64; 2]; 2]> = vec![];
    let mut spans = vec![];
    for (edge, points) in edges.iter().zip(&mut cuts) {
        check_interrupt(interrupt)?;
        points.sort_by(|p, q| p.0.total_cmp(&q.0));
        points.dedup_by(|p, q| (p.0 - q.0).abs() <= 1e-12);
        let mut node_ids = vec![];
        for (_, p) in points.iter() {
            let matches: Vec<_> = nodes
                .iter()
                .enumerate()
                .filter(|(_, q)| distance(*p, **q) <= tolerance_mm)
                .map(|(i, _)| i)
                .collect();
            if matches.len() > 1 {
                return Err(fail(
                    "Junction is within tolerance of multiple distinct nodes",
                ));
            }
            let id = matches.first().copied().unwrap_or_else(|| {
                nodes.push(*p);
                nodes.len() - 1
            });
            node_ids.push(id);
        }
        for (index, pair) in node_ids.windows(2).enumerate() {
            let from = points[index].0;
            let to = points[index + 1].0;
            let curve = if let ExactCurve::Line { .. } = edge.curve {
                if pair[0] == pair[1] {
                    continue;
                }
                ExactCurve::Line {
                    a: nodes[pair[0]],
                    b: nodes[pair[1]],
                }
            } else if from == 0. && to == 1. {
                edge.curve.clone()
            } else {
                queries.curve_segment(&edge.curve, from, to, tolerance_mm)?
            };
            if segments.len() == MAX_SEGMENTS {
                return Err(fail("Arrangement exceeds its derived-segment budget"));
            }
            tangents.push([
                tangent(&curve, false, queries, tolerance_mm)?,
                tangent(&curve, true, queries, tolerance_mm)?.map(|v| -v),
            ]);
            spans.push((index, points.len() - 1));
            segments.push((
                Edge {
                    authored: edge.authored,
                    reversed: false,
                    fragment: None,
                    curve,
                },
                [pair[0], pair[1]],
            ));
        }
    }
    // Half-edge rotation system: each face keeps material on its left.
    let mut outgoing = vec![vec![]; nodes.len()];
    for (i, (_, [a, b])) in segments.iter().enumerate() {
        outgoing[*a].push(2 * i);
        outgoing[*b].push(2 * i + 1);
    }
    let junctions: Vec<BTreeSet<_>> = outgoing
        .iter()
        .map(|halves| halves.iter().map(|h| segments[h / 2].0.authored).collect())
        .collect();
    for ((edge, [a, b]), span) in segments.iter_mut().zip(spans) {
        if span.1 > 1 {
            let bytes =
                serde_json::to_vec(&(span, &junctions[*a], &junctions[*b])).map_err(|e| {
                    QueryError {
                        message: e.to_string(),
                    }
                })?;
            edge.fragment = Some(crate::project_file::ContentHash::of(&bytes));
        }
    }
    let ends = |half: usize| {
        let pair = segments[half / 2].1;
        if half.is_multiple_of(2) {
            pair
        } else {
            [pair[1], pair[0]]
        }
    };
    for halves in &mut outgoing {
        halves.sort_by(|a, b| {
            let angle = |h: usize| {
                let d = tangents[h / 2][h % 2];
                d[1].atan2(d[0])
            };
            angle(*a).total_cmp(&angle(*b))
        });
    }
    // Remove graph bridges before tracing faces, so a stroke connecting two
    // closed loops leaves two continuous wires, not a disconnected face walk.
    let mut entered = vec![None; nodes.len()];
    let mut low = vec![0; nodes.len()];
    let mut bridges = BTreeSet::new();
    let mut clock = 0;
    for root in 0..nodes.len() {
        if entered[root].is_some() {
            continue;
        }
        entered[root] = Some(clock);
        low[root] = clock;
        clock += 1;
        let mut stack = vec![(root, None::<usize>, 0usize)];
        while let Some((node, parent, cursor)) = stack.last_mut() {
            check_interrupt(interrupt)?;
            if *cursor < outgoing[*node].len() {
                let h = outgoing[*node][*cursor];
                *cursor += 1;
                if *parent == Some(h / 2) {
                    continue;
                }
                let target = ends(h)[1];
                if let Some(time) = entered[target] {
                    low[*node] = low[*node].min(time);
                } else {
                    entered[target] = Some(clock);
                    low[target] = clock;
                    clock += 1;
                    stack.push((target, Some(h / 2), 0));
                }
            } else {
                let (node, parent, _) = stack.pop().expect("active node");
                if let Some((ancestor, _, _)) = stack.last() {
                    if low[node] > entered[*ancestor].expect("visited ancestor") {
                        bridges.insert(parent.expect("child edge"));
                    }
                    low[*ancestor] = low[*ancestor].min(low[node]);
                }
            }
        }
    }
    for halves in &mut outgoing {
        halves.retain(|h| !bridges.contains(&(h / 2)));
    }
    let mut next = vec![0; segments.len() * 2];
    for (half, successor) in next.iter_mut().enumerate() {
        if bridges.contains(&(half / 2)) {
            continue;
        }
        let at = &outgoing[ends(half)[1]];
        let reverse = at
            .iter()
            .position(|h| *h == (half ^ 1))
            .expect("reverse half edge");
        *successor = at[(reverse + at.len() - 1) % at.len()];
    }
    let mut visited: Vec<_> = (0..next.len())
        .map(|h| bridges.contains(&(h / 2)))
        .collect();
    let mut boundaries = vec![];
    for seed in 0..next.len() {
        check_interrupt(interrupt)?;
        if visited[seed] {
            continue;
        }
        let mut halves = vec![];
        let mut half = seed;
        while !visited[half] {
            visited[half] = true;
            halves.push(half);
            half = next[half];
        }
        // A bridge occurs twice in one face walk. Remove both traversals;
        // an isolated stroke or dangling branch cannot become manufacturing
        // geometry, including a branch projecting into an otherwise valid cell.
        let set: BTreeSet<_> = halves.iter().copied().collect();
        halves.retain(|h| !set.contains(&(h ^ 1)));
        let mut wire: Vec<_> = halves
            .into_iter()
            .map(|h| {
                let mut edge = segments[h / 2].0.clone();
                if h % 2 == 1 {
                    edge.reversed = !edge.reversed;
                    edge.curve = edge.curve.reversed();
                }
                edge
            })
            .collect();
        merge_arc_seams(&mut wire, tolerance_mm)?;
        if wire.is_empty() {
            continue;
        }
        let area = if wire.len() == 1
            && matches!(wire[0].curve, ExactCurve::Circle { .. })
            && wire[0].reversed
        {
            -queries.signed_area(&wire, tolerance_mm)?.abs()
        } else {
            queries.signed_area(&wire, tolerance_mm)?
        };
        // The unbounded exterior has clockwise winding. Zero-area strokes
        // also disappear; positive cells alone are selectable.
        if area <= tolerance_mm * tolerance_mm {
            continue;
        }
        boundaries.push(Boundary {
            intent: BoundaryIntent {
                curves: wire.iter().map(|e| e.authored).collect(),
            },
            edges: wire,
            signed_area_mm2: area,
        });
    }
    let mut parents = vec![None; boundaries.len()];
    for (i, child) in boundaries.iter().enumerate() {
        let mut candidates = vec![];
        for (j, parent) in boundaries.iter().enumerate() {
            check_interrupt(interrupt)?;
            if i == j || parent.signed_area_mm2 <= child.signed_area_mm2 {
                continue;
            }
            let mut inside = false;
            let mut outside = false;
            for edge in &child.edges {
                let a = edge.curve.ends().0;
                let middle = midpoint(&edge.curve, queries, tolerance_mm)?;
                for point in [a, middle] {
                    match queries.classify_boundary(&parent.edges, point, tolerance_mm)? {
                        PointLocation::Inside => inside = true,
                        PointLocation::Outside => outside = true,
                        PointLocation::Boundary => {}
                    }
                }
            }
            if inside && !outside {
                candidates.push(j);
            }
        }
        parents[i] = candidates.into_iter().min_by(|a, b| {
            boundaries[*a]
                .signed_area_mm2
                .total_cmp(&boundaries[*b].signed_area_mm2)
        });
    }
    let regions: Vec<_> = boundaries
        .iter()
        .enumerate()
        .map(|(i, outer)| {
            let holes: Vec<_> = boundaries
                .iter()
                .enumerate()
                .filter(|(j, _)| parents[*j] == Some(i))
                .map(|(_, b)| b.clone())
                .collect();
            Region {
                area_mm2: outer.signed_area_mm2
                    - holes.iter().map(|h| h.signed_area_mm2).sum::<f64>(),
                outer: outer.clone(),
                holes,
            }
        })
        .collect();
    Ok(Extraction {
        regions,
        diagnostics: vec![],
    })
}

// A periodic source circle can be split at its parameter seam as well as at
// real junctions. Join those consecutive arcs again before making a native
// profile; the seam is not an authored corner or an overlapping boundary.
fn merge_arc_seams(wire: &mut Vec<Edge>, tolerance: f64) -> Result<(), QueryError> {
    loop {
        let mut merged = false;
        for i in 0..wire.len() {
            if wire.len() < 2 {
                break;
            }
            let j = (i + 1) % wire.len();
            let first = &wire[i];
            let second = &wire[j];
            if first.authored != second.authored {
                continue;
            }
            let (
                ExactCurve::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                },
                ExactCurve::Arc {
                    center: c,
                    a: d,
                    b: e,
                    clockwise: w,
                },
            ) = (&first.curve, &second.curve)
            else {
                continue;
            };
            if clockwise != w || distance(*center, *c) > tolerance || distance(*b, *d) > tolerance {
                continue;
            }
            let sweep =
                first.curve.arc().expect("arc").1.abs() + second.curve.arc().expect("arc").1.abs();
            if sweep >= TAU - 1e-10 {
                continue;
            }
            let curve = ExactCurve::Arc {
                center: *center,
                a: *a,
                b: *e,
                clockwise: *clockwise,
            };
            let labels: BTreeSet<_> = [first.fragment, second.fragment]
                .into_iter()
                .flatten()
                .map(|h| h.to_string())
                .collect();
            let fragment = Some(crate::project_file::ContentHash::of(
                &serde_json::to_vec(&labels).map_err(|e| QueryError {
                    message: e.to_string(),
                })?,
            ));
            wire[i].curve = curve;
            wire[i].fragment = fragment;
            wire.remove(j);
            merged = true;
            break;
        }
        if !merged {
            return Ok(());
        }
    }
}

fn midpoint(
    curve: &ExactCurve,
    queries: &dyn RegionQueries,
    tolerance: f64,
) -> Result<Point, QueryError> {
    match curve {
        ExactCurve::Line { a, b } => Ok(along(*a, sub(*b, *a), 0.5)),
        _ => Ok(queries.curve_segment(curve, 0., 0.5, tolerance)?.ends().1),
    }
}

pub(super) fn interior_point(
    region: &Region,
    queries: &dyn RegionQueries,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Point, QueryError> {
    let tolerance = queries.tolerance_mm();
    for edge in &region.outer.edges {
        check_interrupt(interrupt)?;
        let point = midpoint(&edge.curve, queries, tolerance)?;
        let direction = if matches!(edge.curve, ExactCurve::Line { .. }) {
            tangent(&edge.curve, false, queries, tolerance)?
        } else {
            tangent(
                &queries.curve_segment(&edge.curve, 0., 0.5, tolerance)?,
                true,
                queries,
                tolerance,
            )?
        };
        let length = direction[0].hypot(direction[1]);
        if !length.is_finite() || length <= tolerance {
            continue;
        }
        let bounds = edge.curve.bounds();
        let mut offset = distance(bounds[0], bounds[1]).mul_add(1e-3, 0.).min(0.1);
        while offset > tolerance * 4. {
            let candidate = [
                point[0] - direction[1] / length * offset,
                point[1] + direction[0] / length * offset,
            ];
            if region.classify_with_queries_and_interrupt(candidate, queries, interrupt)?
                == PointLocation::Inside
            {
                return Ok(candidate);
            }
            offset *= 0.5;
        }
    }
    Err(fail(
        "Bounded cell has no unambiguous interior above the geometry tolerance",
    ))
}
fn tangent(
    curve: &ExactCurve,
    end: bool,
    queries: &dyn RegionQueries,
    tolerance: f64,
) -> Result<Point, QueryError> {
    let (a, b) = curve.ends();
    let p = if end { b } else { a };
    match curve {
        ExactCurve::Line { a, b } => Ok(sub(*b, *a)),
        ExactCurve::Arc {
            center, clockwise, ..
        } => {
            let r = sub(p, *center);
            let sign = if *clockwise { -1. } else { 1. };
            Ok([-r[1] * sign, r[0] * sign])
        }
        ExactCurve::Circle { center, .. } => {
            let r = sub(p, *center);
            Ok([-r[1], r[0]])
        }
        ExactCurve::BSpline { .. } => {
            let projection = queries
                .project_parameters(curve, p, tolerance)?
                .into_iter()
                .find(|p| {
                    (p.parameter - if end { 1. } else { 0. }).abs() <= 1e-10
                        && p.distance_mm <= tolerance
                })
                .ok_or_else(|| fail("Exact endpoint tangent unavailable"))?;
            Ok(projection.tangent)
        }
    }
}
