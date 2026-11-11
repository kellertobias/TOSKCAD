//! Bounded analytic cap validation for a large collection of relief motifs.
use super::*;
use crate::normal_sweep::CapError;

const WORK: usize = 24_000_000;
struct Budget<'a> {
    left: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
}
impl Budget<'_> {
    fn spend(&mut self, work: usize) -> Result<(), CapError> {
        if self.interrupt.is_some_and(|f| f()) {
            return Err(CapError::Cancelled);
        }
        self.left = self.left.checked_sub(work).ok_or(CapError::Limit)?;
        Ok(())
    }
}
struct Item {
    ring: usize,
    index: usize,
    edge: Edge,
    bounds: Bounds,
}
struct Node {
    bounds: Bounds,
    children: Option<[usize; 2]>,
    item: usize,
}
fn merged(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: std::array::from_fn(|i| a.min[i].min(b.min[i])),
        max: std::array::from_fn(|i| a.max[i].max(b.max[i])),
    }
}
fn tree(
    ids: &mut [usize],
    items: &[Item],
    nodes: &mut Vec<Node>,
    budget: &mut Budget<'_>,
) -> Result<usize, CapError> {
    budget.spend(ids.len())?;
    let bounds = ids
        .iter()
        .map(|&i| items[i].bounds)
        .reduce(merged)
        .expect("nonempty source partition");
    let id = nodes.len();
    nodes.push(Node {
        bounds,
        children: None,
        item: ids[0],
    });
    if ids.len() > 1 {
        // Long parallel edges can have equal X centers despite spanning a
        // wide box. Split their centers, not their physical edge lengths.
        let centers = ids
            .iter()
            .map(|&i| {
                let b = items[i].bounds;
                let p = std::array::from_fn(|j| b.min[j].midpoint(b.max[j]));
                Bounds { min: p, max: p }
            })
            .reduce(merged)
            .expect("nonempty source centers");
        let axis = usize::from(centers.max[1] - centers.min[1] > centers.max[0] - centers.min[0]);
        budget.spend(ids.len() * ((usize::BITS - ids.len().leading_zeros()) as usize))?;
        ids.sort_unstable_by(|&a, &b| {
            items[a].bounds.min[axis]
                .midpoint(items[a].bounds.max[axis])
                .total_cmp(&items[b].bounds.min[axis].midpoint(items[b].bounds.max[axis]))
        });
        let middle = ids.len() / 2;
        let (a, b) = ids.split_at_mut(middle);
        nodes[id].children = Some([
            tree(a, items, nodes, budget)?,
            tree(b, items, nodes, budget)?,
        ]);
    }
    Ok(id)
}
fn within(b: Bounds, p: Point, slack: f64) -> bool {
    (0..2).all(|i| p[i] >= b.min[i] - slack && p[i] <= b.max[i] + slack)
}

pub(in crate::normal_sweep) fn validate(
    rings: &mut [Vec<Edge>],
    tolerance: f64,
    minimum: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), CapError> {
    let mut budget = Budget {
        left: WORK,
        interrupt,
    };
    budget.spend(rings.iter().map(Vec::len).sum())?;
    let (slack, roundoff) = normalize_all(rings, tolerance, minimum).map_err(CapError::Geometry)?;
    let items = rings
        .iter()
        .enumerate()
        .flat_map(|(ring, edges)| {
            edges.iter().enumerate().map(move |(index, &edge)| Item {
                ring,
                index,
                edge,
                bounds: Bounds::of(edge),
            })
        })
        .collect::<Vec<_>>();
    let mut nodes = Vec::with_capacity(2 * items.len());
    let mut ids = (0..items.len()).collect::<Vec<_>>();
    tree(&mut ids, &items, &mut nodes, &mut budget)?;
    for (index, item) in items.iter().enumerate() {
        let mut queue = vec![0];
        while let Some(node) = queue.pop() {
            budget.spend(1)?;
            let node = &nodes[node];
            if !item.bounds.overlaps(node.bounds, slack) {
                continue;
            }
            if let Some(children) = node.children {
                queue.extend(children);
                continue;
            }
            if node.item >= index {
                continue;
            }
            let other = &items[node.item];
            let mut joins = [[0.; 2]; 2];
            let mut count = 0;
            if item.ring == other.ring {
                if other.index + 1 == item.index {
                    joins[count] = item.edge.start();
                    count += 1;
                }
                if other.index == 0 && item.index + 1 == rings[item.ring].len() {
                    joins[count] = item.edge.end();
                    count += 1;
                }
            }
            if !clear_pair(item.edge, other.edge, &joins[..count], slack, roundoff) {
                return Err(CapError::Geometry(Error::InvalidProfile));
            }
        }
    }
    let bounds = rings
        .iter()
        .map(|ring| {
            ring.iter()
                .copied()
                .map(Bounds::of)
                .reduce(merged)
                .expect("normalized nonempty ring")
        })
        .collect::<Vec<_>>();
    for (i, ring) in rings.iter().enumerate().skip(1) {
        budget.spend(rings[0].len())?;
        if !inside(&rings[0], ring[0].start()) {
            return Err(CapError::Geometry(Error::InvalidProfile));
        }
        for (j, other) in rings[1..i].iter().enumerate() {
            budget.spend(1)?;
            let a = within(bounds[j + 1], ring[0].start(), slack);
            let b = within(bounds[i], other[0].start(), slack);
            budget.spend(if a { other.len() } else { 0 } + if b { ring.len() } else { 0 })?;
            if a && inside(other, ring[0].start()) || b && inside(ring, other[0].start()) {
                return Err(CapError::Geometry(Error::InvalidProfile));
            }
        }
    }
    Ok(())
}
