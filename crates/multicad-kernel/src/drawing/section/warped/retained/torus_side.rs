//! An edge-on window face contributes its frontmost cut boundary in the view.
use super::*;
use std::collections::BTreeSet;

type Edge = [[f64; 2]; 2];

pub(super) fn contours(
    section: &[ProjectedVector],
    axis: usize,
    coordinate: f64,
    near: f64,
    tolerance: f64,
    owner: (usize, usize),
    limit: usize,
) -> Result<Vec<ContourCandidate>, GeometryError> {
    let edges: Vec<_> = section
        .iter()
        .flat_map(|vector| vector.preview_mm.windows(2))
        .filter_map(|pair| {
            let [a, b] = [pair[0], pair[1]];
            if (a[0] - b[0]).abs() <= tolerance {
                None
            } else if a[0] < b[0] {
                Some([a, b])
            } else {
                Some([b, a])
            }
        })
        .collect();
    let mut events: Vec<_> = edges
        .iter()
        .enumerate()
        .flat_map(|(id, edge)| [(edge[0][0], id, true), (edge[1][0], id, false)])
        .collect();
    events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut work = limit.saturating_mul(64);
    let mut active = BTreeSet::new();
    let mut output = Vec::new();
    let mut events = events.into_iter().peekable();
    while let Some((lo, id, starts)) = events.next() {
        if starts {
            active.insert(id);
        } else {
            active.remove(&id);
        }
        let Some(&(hi, _, _)) = events.peek() else {
            break;
        };
        if hi - lo <= tolerance || active.is_empty() {
            continue;
        }
        let count = active.len();
        work = work
            .checked_sub(count * (count + 1))
            .ok_or_else(super::limit)?;
        let mut cuts = vec![lo, hi];
        for &a in &active {
            for &b in active.range((std::ops::Bound::Excluded(a), std::ops::Bound::Unbounded)) {
                let before = depth(edges[a], lo) - depth(edges[b], lo);
                let after = depth(edges[a], hi) - depth(edges[b], hi);
                if before * after < 0. {
                    cuts.push((hi - lo).mul_add(before / (before - after), lo));
                }
            }
        }
        cuts.sort_unstable_by(f64::total_cmp);
        for pair in cuts.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            if b - a <= tolerance {
                continue;
            }
            work = work.checked_sub(count).ok_or_else(super::limit)?;
            let mid = a.midpoint(b);
            let front = active
                .iter()
                .map(|&id| edges[id])
                .max_by(|&a, &b| depth(a, mid).total_cmp(&depth(b, mid)))
                .expect("active side cut edges");
            let depths = [depth(front, a), depth(front, b)];
            if depths.iter().all(|&z| (z - near).abs() <= tolerance) {
                continue; // The front cap already publishes this connector.
            }
            if output.len() == limit {
                return Err(super::limit());
            }
            let (source, face) = owner;
            output.push(ContourCandidate {
                face,
                line: LineCandidate {
                    source,
                    points: std::array::from_fn(|i| {
                        let u = [a, b][i];
                        let z = depths[i];
                        if axis == 0 {
                            [coordinate, u, z]
                        } else {
                            [u, coordinate, z]
                        }
                    }),
                },
            });
        }
    }
    Ok(output)
}

fn depth([[a, za], [b, zb]]: Edge, x: f64) -> f64 {
    ((x - a) / (b - a)).mul_add(zb - za, za)
}
