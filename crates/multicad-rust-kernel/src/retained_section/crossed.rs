use super::*;

/// Project bodies swept along either cardinal screen axis. Bodies in
/// `along_v` use transposed (V, U, depth) coordinates throughout, including
/// stock, cuts and positive profiles. The returned segments use (U, V).
pub fn project_crossed(
    along_u: &[Body],
    along_v: &[Body],
    options: Options,
) -> Result<Vec<Segment>> {
    let mut work = WORK_LIMIT;
    let a = Scene::new(along_u, options, &mut work)?;
    let b = Scene::new(
        along_v,
        Options {
            windows_mm: [options.windows_mm[1], options.windows_mm[0]],
            ..options
        },
        &mut work,
    )?;
    let mut u = Captured::default();
    let mut v = Captured::default();
    a.scan(&mut u, &mut work)?;
    b.scan(&mut v, &mut work)?;
    let mut output = Output::new(options.tolerance_mm);
    clip(u.edges, &b, &v.front, false, &mut output, &mut work)?;
    clip(v.edges, &a, &u.front, true, &mut output, &mut work)?;
    output.finish(options.segment_limit)
}

#[derive(Default)]
struct Captured {
    edges: Vec<Candidate>,
    front: Vec<Vec<Option<Height>>>,
}

impl ScanOutput for Captured {
    fn edge(&mut self, edge: Candidate) {
        self.edges.push(edge);
    }

    fn band(&mut self, v_events: &[f64], band: &Band) {
        self.front.push(
            v_events
                .windows(2)
                .zip(band)
                .map(|(v, heights)| {
                    let sample = v[0].midpoint(v[1]);
                    heights
                        .iter()
                        .copied()
                        .flatten()
                        .max_by(|a, b| a.at(sample).total_cmp(&b.at(sample)))
                })
                .collect(),
        );
    }
}

/// Open intervals incident to the query point. The nearest support of the
/// two sides counts at a screen seam, as in the single-axis scan.
fn incident(events: &[f64], coordinate: f64) -> [Option<usize>; 2] {
    let split = events.partition_point(|&event| event < coordinate && !equal(event, coordinate));
    if events
        .get(split)
        .is_some_and(|&event| equal(event, coordinate))
    {
        [
            split.checked_sub(1),
            (split + 1 < events.len()).then_some(split),
        ]
    } else {
        [
            (split > 0 && split < events.len()).then_some(split.saturating_sub(1)),
            None,
        ]
    }
}

fn opposing_front(
    scene: &Scene<'_>,
    front: &[Vec<Option<Height>>],
    [u, v]: Point,
) -> Option<Height> {
    let along = incident(&scene.u, u);
    let across = incident(&scene.v, v);
    along
        .into_iter()
        .flatten()
        .flat_map(|i| {
            across
                .into_iter()
                .flatten()
                .filter_map(move |j| front[i][j])
        })
        .max_by(|a, b| a.at(v).total_cmp(&b.at(v)))
}

fn clip(
    candidates: Vec<Candidate>,
    other: &Scene<'_>,
    front: &[Vec<Option<Height>>],
    transposed: bool,
    output: &mut Output,
    work: &mut usize,
) -> Result<()> {
    for edge in candidates {
        let Candidate {
            axis,
            fixed,
            span,
            height,
            ..
        } = edge;
        // Transposing interchanges which coordinate varies. A vertical edge
        // meets the other group's U bands and a horizontal edge meets its V
        // events. Only one of the two depths can vary along either segment.
        let events = if axis == 0 { &other.u } else { &other.v };
        let start = events.partition_point(|&event| event <= span[0]);
        let end = events.partition_point(|&event| event < span[1]);
        let breaks = std::iter::once(span[0])
            .chain(events[start..end].iter().copied())
            .chain(std::iter::once(span[1]));
        let next = breaks.clone().skip(1);
        for (lo, hi) in breaks.zip(next) {
            charge(work, 1)?;
            let sample = lo.midpoint(hi);
            let [u, v] = if axis == 0 {
                [sample, fixed]
            } else {
                [fixed, sample]
            };
            let axis_out = if transposed { 1 - axis } else { axis };
            let mut write = |span| {
                output.edge(Candidate {
                    axis: axis_out,
                    span,
                    ..edge
                })
            };
            let Some(other_height) = opposing_front(other, front, [u, v]) else {
                write([lo, hi]);
                continue;
            };
            let occluder = if axis == 0 {
                Height::flat(other_height.at(v))
            } else {
                other_height
            };
            let mut cuts = vec![lo, hi];
            height.intersections(occluder, [lo, hi], &mut cuts);
            sort_unique(&mut cuts);
            for pair in cuts.windows(2) {
                let sample = pair[0].midpoint(pair[1]);
                let [a, b] = [height, occluder].map(|h| h.at(sample));
                if a >= b || equal(a, b) {
                    write([pair[0], pair[1]]);
                }
            }
        }
    }
    Ok(())
}
