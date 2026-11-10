//! Paper-space queries on the same bounding hierarchy used for visibility rays.
use super::{BoundedFace, Bounds, Children, Node, Point2};

pub(crate) struct PaperIndex(Option<Node<Entry>>);

struct Entry {
    index: usize,
    bounds: Bounds,
}

impl BoundedFace for Entry {
    fn bounds(&self) -> Bounds {
        self.bounds
    }
}

fn paper_bounds([[u0, v0], [u1, v1]]: [Point2; 2]) -> Bounds {
    Bounds {
        min: [u0, v0, 0.],
        max: [u1, v1, 0.],
    }
}

impl PaperIndex {
    pub(crate) fn build<E>(
        bounds: impl ExactSizeIterator<Item = [Point2; 2]>,
        charge: &impl Fn(usize) -> Result<(), E>,
    ) -> Result<Self, E> {
        let count = bounds.len();
        charge(count.saturating_mul(count.max(1).ilog2() as usize + 1))?;
        Ok(Self(Node::build(
            bounds
                .enumerate()
                .map(|(index, bounds)| Entry {
                    index,
                    bounds: paper_bounds(bounds),
                })
                .collect(),
        )))
    }

    /// Visit the overlapping entries until one returns true.
    pub(crate) fn any<E>(
        &self,
        bounds: [Point2; 2],
        tolerance: f64,
        charge: &impl Fn(usize) -> Result<(), E>,
        mut visit: impl FnMut(usize) -> Result<bool, E>,
    ) -> Result<bool, E> {
        match &self.0 {
            Some(root) => root.any_paper(paper_bounds(bounds), tolerance, charge, &mut visit),
            None => Ok(false),
        }
    }
}

impl Node<Entry> {
    fn any_paper<E>(
        &self,
        bounds: Bounds,
        tolerance: f64,
        charge: &impl Fn(usize) -> Result<(), E>,
        visit: &mut impl FnMut(usize) -> Result<bool, E>,
    ) -> Result<bool, E> {
        charge(1)?;
        if !self.bounds.overlaps_paper(bounds, tolerance) {
            return Ok(false);
        }
        match &self.children {
            Children::Leaves(entries) => {
                charge(entries.len())?;
                for entry in entries {
                    if entry.bounds.overlaps_paper(bounds, tolerance) && visit(entry.index)? {
                        return Ok(true);
                    }
                }
            }
            Children::Branches(children) => {
                for child in children {
                    if child.any_paper(bounds, tolerance, charge, visit)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_query_keeps_touching_entries_within_the_requested_tolerance() {
        use std::cell::Cell;

        let maximum = Cell::new(usize::MAX);
        let charge = |cost| {
            if cost <= maximum.get() {
                Ok(())
            } else {
                Err(())
            }
        };
        let visited = Cell::new(None);
        let visit = |entry| {
            visited.set(Some(entry));
            Ok(true)
        };
        let entries = [[[0., 0.], [1., 1.]], [[4., 4.], [5., 5.]]];
        let empty = PaperIndex::build(entries[..0].iter().copied(), &charge).unwrap();
        assert!(!empty.any([[0.; 2]; 2], 0., &charge, visit).unwrap());
        assert_eq!(visited.get(), None);

        let index = PaperIndex::build(entries.iter().copied(), &charge).unwrap();
        maximum.set(1);
        assert!(PaperIndex::build(entries.iter().copied(), &charge).is_err());
        assert_eq!(index.any([[0.; 2]; 2], 0., &charge, visit), Err(()));
        assert_eq!(visited.get(), None);

        maximum.set(usize::MAX);
        let query = [[1.125, 0.5]; 2];
        assert!(!index.any(query, 0., &charge, visit).unwrap());
        assert_eq!(visited.get(), None);
        assert!(index.any(query, 0.125, &charge, visit).unwrap());
        assert_eq!(visited.get(), Some(0));
    }
}
