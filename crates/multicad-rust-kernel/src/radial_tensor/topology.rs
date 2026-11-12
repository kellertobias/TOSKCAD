//! Oriented manifold ownership; this proof alone does not imply material.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone)]
pub(super) struct Topology {
    pub edge_faces: Vec<[usize; 2]>,
    pub vertex_edges: Vec<Vec<usize>>,
}
pub(super) fn certify(
    vertex_count: usize,
    edges: &[[usize; 2]],
    loops: &[Vec<(usize, bool)>],
    work: &mut Work<'_>,
) -> Result<Topology, Error> {
    if vertex_count > 80 || edges.len() > 112 || loops.len() > 34 {
        return Err(Error::Limit);
    }
    if vertex_count < 4 || edges.len() < 6 || loops.len() < 4 {
        return Err(Error::Invalid);
    }
    let mut uses = vec![Vec::<(usize, bool)>::new(); edges.len()];
    let mut vertex_edges = vec![Vec::new(); vertex_count];
    for (i, &[a, b]) in edges.iter().enumerate() {
        work.step()?;
        if a >= vertex_count || b >= vertex_count || a == b {
            return Err(Error::Invalid);
        }
        vertex_edges[a].push(i);
        vertex_edges[b].push(i);
    }
    let mut links = vec![Vec::<(usize, usize)>::new(); vertex_count];
    for (face, ring) in loops.iter().enumerate() {
        work.step()?;
        if ring.len() > 80 {
            return Err(Error::Limit);
        }
        if ring.len() < 3 {
            return Err(Error::Invalid);
        }
        let mut vertices = BTreeSet::new();
        for (i, &(edge, forward)) in ring.iter().enumerate() {
            work.step()?;
            let [a, b] = *edges.get(edge).ok_or(Error::Invalid)?;
            let [from, to] = if forward { [a, b] } else { [b, a] };
            if !vertices.insert(from) {
                return Err(Error::Invalid);
            }
            let (next, sense) = ring[(i + 1) % ring.len()];
            let next_ends = edges.get(next).ok_or(Error::Invalid)?;
            if to != next_ends[usize::from(!sense)] {
                return Err(Error::Invalid);
            }
            uses[edge].push((face, forward));
            links[to].push((edge, next));
        }
    }
    let edge_faces = uses
        .into_iter()
        .map(|use_| match use_.as_slice() {
            [(a, sa), (b, sb)] if a != b && sa != sb => Ok([*a, *b]),
            _ => Err(Error::Invalid),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut neighbors = vec![Vec::new(); loops.len()];
    for &[a, b] in &edge_faces {
        neighbors[a].push(b);
        neighbors[b].push(a);
    }
    connected(&neighbors, work)?;
    for (v, incident) in vertex_edges.iter().enumerate() {
        work.step()?;
        if incident.len() < 2 {
            return Err(Error::Invalid);
        }
        let mut fan = vec![Vec::new(); incident.len()];
        for &(a, b) in &links[v] {
            let a = incident
                .iter()
                .position(|i| *i == a)
                .ok_or(Error::Invalid)?;
            let b = incident
                .iter()
                .position(|i| *i == b)
                .ok_or(Error::Invalid)?;
            fan[a].push(b);
            fan[b].push(a);
        }
        // A simple face visits this vertex once. Each incident edge has
        // exactly two distinct opposite face uses, so it already has two
        // link incidences. Connectedness proves that link is one cycle.
        connected(&fan, work)?;
    }
    if vertex_count + loops.len() != edges.len() + 2 {
        return Err(Error::Invalid);
    }
    Ok(Topology {
        edge_faces,
        vertex_edges,
    })
}
fn connected(neighbors: &[Vec<usize>], work: &mut Work<'_>) -> Result<(), Error> {
    let mut seen = vec![false; neighbors.len()];
    let mut pending = vec![0];
    while let Some(i) = pending.pop() {
        work.step()?;
        if seen[i] {
            continue;
        }
        seen[i] = true;
        pending.extend(neighbors[i].iter().copied().filter(|&j| !seen[j]));
    }
    if seen.into_iter().all(|v| v) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type Cube = (Vec<[usize; 2]>, Vec<Vec<(usize, bool)>>);
    fn cube() -> Cube {
        let edges = vec![
            [0, 1],
            [1, 2],
            [2, 3],
            [3, 0],
            [4, 5],
            [5, 6],
            [6, 7],
            [7, 4],
            [0, 4],
            [1, 5],
            [2, 6],
            [3, 7],
        ];
        let loops = vec![
            vec![(3, false), (2, false), (1, false), (0, false)],
            vec![(4, true), (5, true), (6, true), (7, true)],
            vec![(0, true), (9, true), (4, false), (8, false)],
            vec![(1, true), (10, true), (5, false), (9, false)],
            vec![(2, true), (11, true), (6, false), (10, false)],
            vec![(3, true), (8, true), (7, false), (11, false)],
        ];
        (edges, loops)
    }
    fn work() -> Work<'static> {
        Work {
            remaining: 10000,
            interrupt: None,
        }
    }
    #[test]
    fn closed_oriented_sphere_ownership_is_checked_before_material() {
        let (edges, loops) = cube();
        let t = certify(8, &edges, &loops, &mut work()).unwrap();
        assert_eq!(t.edge_faces.len(), 12);
        assert!(t.vertex_edges.iter().all(|e| e.len() == 3));
        let mut reversed = loops.clone();
        reversed[0].reverse();
        for e in &mut reversed[0] {
            e.1 = !e.1;
        }
        assert!(certify(8, &edges, &reversed, &mut work()).is_err());
        let mut open = loops.clone();
        open.pop();
        assert!(certify(8, &edges, &open, &mut work()).is_err());
        let mut dangling = edges.clone();
        dangling.push([0, 6]);
        assert!(certify(8, &dangling, &loops, &mut work()).is_err());
        let mut bad = edges.clone();
        bad[0][0] = 99;
        assert!(certify(8, &bad, &loops, &mut work()).is_err());
        let mut repeated = loops.clone();
        repeated[0].extend_from_within(..);
        assert!(certify(8, &edges, &repeated, &mut work()).is_err());
        let mut no_work = Work {
            remaining: 0,
            interrupt: None,
        };
        assert!(matches!(
            certify(8, &edges, &loops, &mut no_work),
            Err(Error::Limit)
        ));
        let mut cancelled = Work {
            remaining: 10000,
            interrupt: Some(&|| true),
        };
        assert!(matches!(
            certify(8, &edges, &loops, &mut cancelled),
            Err(Error::Cancelled)
        ));
        certify(8, &edges, &loops, &mut work()).unwrap();
    }

    #[test]
    fn ownership_rejects_multiple_shells_pinched_vertices_and_positive_genus() {
        let (edges, loops) = cube();
        assert!(matches!(
            certify(81, &edges, &loops, &mut work()),
            Err(Error::Limit)
        ));
        assert!(matches!(
            certify(3, &edges, &loops, &mut work()),
            Err(Error::Invalid)
        ));
        let mut long = loops.clone();
        long[0].resize(81, (0, true));
        assert!(matches!(
            certify(8, &edges, &long, &mut work()),
            Err(Error::Limit)
        ));
        let mut short = loops.clone();
        short[0].truncate(2);
        assert!(matches!(
            certify(8, &edges, &short, &mut work()),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            certify(9, &edges, &loops, &mut work()),
            Err(Error::Invalid)
        ));
        let mut two = edges.clone();
        two.extend(edges.iter().map(|e| e.map(|v| v + 8)));
        let mut shells = loops.clone();
        shells.extend(
            loops
                .iter()
                .map(|f| f.iter().map(|&(e, d)| (e + 12, d)).collect()),
        );
        assert!(matches!(
            certify(16, &two, &shells, &mut work()),
            Err(Error::Invalid)
        ));
        let pinched = edges
            .iter()
            .map(|e| {
                e.map(|v| {
                    if v == 6 {
                        0
                    } else if v == 7 {
                        6
                    } else {
                        v
                    }
                })
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            certify(7, &pinched, &loops, &mut work()),
            Err(Error::Invalid)
        ));
        let index = |i: usize, j: usize| (i % 3) * 3 + j % 3;
        let mut torus = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                torus.push([index(i, j), index(i + 1, j)]);
            }
        }
        for i in 0..3 {
            for j in 0..3 {
                torus.push([index(i, j), index(i, j + 1)]);
            }
        }
        let mut faces = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                faces.push(vec![
                    (index(i, j), true),
                    (9 + index(i + 1, j), true),
                    (index(i, j + 1), false),
                    (9 + index(i, j), false),
                ]);
            }
        }
        assert!(matches!(
            certify(9, &torus, &faces, &mut work()),
            Err(Error::Invalid)
        ));
        assert!(certify(8, &edges, &loops, &mut work()).is_ok());
    }
}
