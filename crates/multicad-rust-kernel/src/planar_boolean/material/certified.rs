//! Boundary-aware Boolean operations on reusable occupied-material certificates.
use super::*;
use std::cell::Cell;
use std::collections::BTreeSet;

mod separation;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Union,
    Intersect,
    Subtract,
}

struct Operand<'a> {
    boundary: &'a Union,
    pieces: &'a [ConvexPiece],
}

struct Work<'a> {
    remaining: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
    uncertain: Cell<bool>,
}
impl<'a> Work<'a> {
    fn new(interrupt: Option<&'a dyn Fn() -> bool>) -> Self {
        Self {
            remaining: MAX_WORK,
            interrupt,
            uncertain: Cell::new(false),
        }
    }
    fn check(&self) -> Result<(), Error> {
        checkpoint(self.interrupt)
    }
    fn charge(&mut self, cost: usize) -> Result<(), Error> {
        self.check()?;
        charge(&mut self.remaining, cost)
    }
}

impl CutMaterial {
    /// Cooperative cancellation is checked before/after certification. The
    /// supplied authoritative boundary and both occupied sets stay untouched.
    pub fn certify_existing_with_interrupt(
        boundary: &Union,
        pieces: &[ConvexPiece],
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        validate_cells(pieces, tolerance)?;
        let mut work = Work::new(interrupt);
        work.charge(
            boundary.vertices.len()
                + boundary
                    .faces
                    .iter()
                    .map(|f| f.vertices.len())
                    .sum::<usize>(),
        )?;
        charge_cells(pieces, &mut work)?;
        verify_cells_with_interrupt(boundary, pieces, tolerance, &mut work.remaining, interrupt)?;
        work.check()?;
        Ok(Self {
            boundary: boundary.clone(),
            pieces: pieces.to_vec(),
        })
    }

    /// Recover a legacy cut's cells with one work budget and cancellation
    /// checkpoints inside splitting, regularization and coverage proof.
    pub fn recover_with_interrupt(
        boundary: &Union,
        stock: &[ConvexPiece],
        tools: &[ConvexPiece],
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        validate_cells(stock, tolerance)?;
        if tools.is_empty() {
            return Err(Error::Invalid);
        }
        if stock.len().saturating_add(tools.len()) > MAX_PIECES {
            return Err(Error::Limit);
        }
        let mut work = Work::new(interrupt);
        charge_cells(stock, &mut work)?;
        let mut pieces = stock.to_vec();
        for tool in tools {
            let mut remaining = Vec::new();
            for piece in pieces {
                subtract_cell_with_interrupt(
                    piece,
                    tool,
                    tolerance,
                    &mut work.remaining,
                    &mut remaining,
                    interrupt,
                )?;
                if remaining.len() > MAX_PIECES {
                    return Err(Error::Limit);
                }
            }
            pieces = remaining;
        }
        verify_cells_with_interrupt(boundary, &pieces, tolerance, &mut work.remaining, interrupt)?;
        work.check()?;
        Ok(Self {
            boundary: boundary.clone(),
            pieces,
        })
    }

    /// Intersect all original operands. Labels are (original operand index,
    /// original exposed boundary face ordinal), never intermediate cell faces.
    pub fn intersect_certified(operands: &[&Self], tolerance: f64) -> Result<Option<Self>, Error> {
        Self::intersect_certified_with_interrupt(operands, tolerance, None)
    }
    pub fn intersect_certified_with_interrupt(
        operands: &[&Self],
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Option<Self>, Error> {
        operation(
            operands,
            tolerance,
            Operation::Intersect,
            &mut Work::new(interrupt),
        )
    }

    /// Union original authoritative boundaries, retaining the earliest operand
    /// on coincident outward patches. Internal cell partitions are not ancestry.
    pub fn union_certified(operands: &[&Self], tolerance: f64) -> Result<Self, Error> {
        Self::union_certified_with_interrupt(operands, tolerance, None)
    }
    pub fn union_certified_with_interrupt(
        operands: &[&Self],
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        operation(
            operands,
            tolerance,
            Operation::Union,
            &mut Work::new(interrupt),
        )?
        .ok_or(Error::Unresolved)
    }

    /// Subtract the regularized union of operands 1.. from stock operand 0.
    /// Tool boundary normals are reversed; their original labels are retained.
    pub fn subtract_certified(operands: &[&Self], tolerance: f64) -> Result<Option<Self>, Error> {
        Self::subtract_certified_with_interrupt(operands, tolerance, None)
    }
    pub fn subtract_certified_with_interrupt(
        operands: &[&Self],
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Option<Self>, Error> {
        operation(
            operands,
            tolerance,
            Operation::Subtract,
            &mut Work::new(interrupt),
        )
    }
}

fn charge_cells(pieces: &[ConvexPiece], work: &mut Work<'_>) -> Result<(), Error> {
    for piece in pieces {
        work.charge(piece.faces.iter().map(Vec::len).sum::<usize>() + 1)?;
    }
    Ok(())
}

fn emit_boundary(
    operands: &[Operand<'_>],
    tolerance: f64,
    op: Operation,
    work: &mut Work<'_>,
) -> Result<Option<Union>, Error> {
    validate_tolerance(tolerance)?;
    validate_operand_len(operands.len())?;
    // Every certificate already has at most MAX_PIECES cells. Bound the
    // operand count before summing, so this preflight cannot overflow usize.
    let count: usize = operands.iter().map(|operand| operand.pieces.len()).sum();
    if count > MAX_PIECES {
        return Err(Error::Limit);
    }
    work.check()?;
    if op == Operation::Intersect
        && (0..3).any(|axis| {
            let lower = operands
                .iter()
                .map(|m| m.boundary.bounds[0][axis])
                .fold(f64::NEG_INFINITY, f64::max);
            let upper = operands
                .iter()
                .map(|m| m.boundary.bounds[1][axis])
                .fold(f64::INFINITY, f64::min);
            lower >= upper
        })
    {
        return Ok(None);
    }
    if op == Operation::Intersect && separation::empty(operands, tolerance, work)? {
        return Ok(None);
    }
    let charts = operands
        .iter()
        .map(|operand| {
            work.check()?;
            convex_charts(operand.boundary, &mut work.remaining, work.interrupt)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let mut boundary = shell::Boundary::new(tolerance);
    for (source, source_charts) in charts.iter().enumerate() {
        work.check()?;
        for chart in source_charts {
            let precision = precision(chart.ring.iter().copied(), tolerance);
            let mut fragments = vec![chart.ring.clone()];
            for (other_index, other) in operands.iter().enumerate().filter(|&(i, _)| i != source) {
                work.check()?;
                fragments = match op {
                    Operation::Intersect => {
                        let covered = cover(
                            fragments,
                            chart,
                            chart.support,
                            other.pieces,
                            [false, true],
                            precision,
                            work,
                        )?;
                        if other_index < source {
                            remove_owned_patches(
                                covered,
                                chart,
                                &charts[other_index],
                                precision,
                                work,
                            )?
                        } else {
                            covered
                        }
                    }
                    Operation::Union => outside(
                        fragments,
                        chart,
                        other.pieces,
                        [other_index > source, false],
                        precision,
                        work,
                    )?,
                    Operation::Subtract if source == 0 => outside(
                        fragments,
                        chart,
                        other.pieces,
                        [false, true],
                        precision,
                        work,
                    )?,
                    Operation::Subtract if other_index != 0 => outside(
                        fragments,
                        chart,
                        other.pieces,
                        [other_index > source, false],
                        precision,
                        work,
                    )?,
                    Operation::Subtract => {
                        let covered = cover(
                            fragments,
                            chart,
                            chart.support,
                            other.pieces,
                            [true, false],
                            precision,
                            work,
                        )?;
                        let opposite = Plane {
                            normal: scale(chart.support.normal, -1.),
                            ..chart.support
                        };
                        cover(
                            covered,
                            chart,
                            opposite,
                            other.pieces,
                            [true, false],
                            precision,
                            work,
                        )?
                    }
                };
                if fragments.is_empty() {
                    break;
                }
            }
            for mut ring in fragments {
                work.charge(ring.len())?;
                let mut normal = chart.support.normal;
                if op == Operation::Subtract && source != 0 {
                    ring.reverse();
                    normal = scale(normal, -1.);
                }
                boundary.add(&ring, normal, source, chart.ordinal)?;
            }
        }
    }
    work.check()?;
    if boundary.faces.is_empty() {
        // A nonzero classification snapped to contact cannot prove that no
        // material remains. Refuse sub-resolution slivers instead of turning
        // a potentially positive result into a successful empty Boolean.
        if work.uncertain.get() {
            return Err(Error::Unresolved);
        }
        return Ok(None);
    }
    boundary.split_edges(&mut work.remaining)?;
    let boundary = Union::from_boundary(boundary)?;
    work.check()?;
    Ok(Some(boundary))
}

fn operation(
    operands: &[&CutMaterial],
    tolerance: f64,
    op: Operation,
    work: &mut Work<'_>,
) -> Result<Option<CutMaterial>, Error> {
    validate_operand_len(operands.len())?;
    let views: Vec<_> = operands
        .iter()
        .map(|operand| Operand {
            boundary: &operand.boundary,
            pieces: &operand.pieces,
        })
        .collect();
    let Some(boundary) = emit_boundary(&views, tolerance, op, work)? else {
        return Ok(None);
    };
    let pieces = recover(operands, op, tolerance, work)?;
    verify_cells_with_interrupt(
        &boundary,
        &pieces,
        tolerance,
        &mut work.remaining,
        work.interrupt,
    )?;
    work.check()?;
    Ok(Some(CutMaterial { boundary, pieces }))
}

fn validate_operand_len(count: usize) -> Result<(), Error> {
    if count < 2 {
        return Err(Error::Invalid);
    }
    if count > MAX_PIECES {
        return Err(Error::Limit);
    }
    Ok(())
}

pub(super) fn raw_intersection(
    left: &[ConvexPiece],
    right: &[ConvexPiece],
    tolerance: f64,
    budget: usize,
) -> Result<Option<Union>, Error> {
    validate_cells(left, tolerance)?;
    validate_cells(right, tolerance)?;
    if left.len() + right.len() > MAX_PIECES {
        return Err(Error::Limit);
    }
    let mut work = Work::new(None);
    work.remaining = budget;
    let mut make = |pieces: &[ConvexPiece]| -> Result<Union, Error> {
        charge_cells(pieces, &mut work)?;
        let exterior =
            clipping::exterior_with_interrupt(pieces, tolerance, &mut work.remaining, None)?;
        Union::from_boundary(exterior)
    };
    let a = make(left)?;
    let b = make(right)?;
    // Construct source views from precisely these raw cells; do not invent
    // material certificates or impose output-cell recovery on boundary queries.
    let views = [
        Operand {
            boundary: &a,
            pieces: left,
        },
        Operand {
            boundary: &b,
            pieces: right,
        },
    ];
    let Some(mut result) = emit_boundary(&views, tolerance, Operation::Intersect, &mut work)?
    else {
        return Ok(None);
    };
    let inputs = [&a, &b];
    for face in &mut result.faces {
        let operand = face.source;
        let original = &inputs[operand].faces[face.source_face];
        face.source = original.source + if operand == 0 { 0 } else { left.len() };
        face.source_face = original.source_face;
    }
    Ok(Some(result))
}

/// Coincident ownership is a property of original EXPOSED finite patches.
/// Recovery-cell planes can be hidden internal partitions and cannot own a
/// new clipping face. Remove only the area actually published by earlier input.
fn remove_owned_patches(
    mut fragments: Vec<Vec<Point>>,
    chart: &ConvexChart,
    earlier: &[ConvexChart],
    tolerance: f64,
    work: &mut Work<'_>,
) -> Result<Vec<Vec<Point>>, Error> {
    for patch in earlier {
        work.charge(patch.ring.len() + 1)?;
        if !coincident(
            chart,
            chart.support,
            patch.support,
            tolerance,
            &work.uncertain,
        ) || !clipping::bounds_overlap(chart.bounds, patch.bounds, tolerance)
        {
            continue;
        }
        let planes: Vec<_> = patch
            .ring
            .iter()
            .zip(patch.ring.iter().cycle().skip(1))
            .map(|(&point, &next)| {
                let normal = cross(sub(next, point), patch.support.normal);
                Plane {
                    point,
                    normal: scale(normal, norm(normal).recip()),
                }
            })
            .collect();
        let mut remaining = Vec::new();
        let mut fragment_budget = MAX_INCIDENCES;
        for ring in fragments {
            work.check()?;
            clipping::partition_tracked(
                ring,
                &planes,
                tolerance,
                &mut remaining,
                &mut work.remaining,
                &mut fragment_budget,
                Some(&work.uncertain),
            )?;
        }
        fragments = remaining;
        if fragments.is_empty() {
            break;
        }
    }
    Ok(fragments)
}

fn coincident(
    chart: &ConvexChart,
    support: Plane,
    plane: Plane,
    tolerance: f64,
    uncertain: &Cell<bool>,
) -> bool {
    support.same_direction(plane, tolerance)
        && chart.ring.iter().all(|&p| {
            let distance = plane.distance(p);
            clipping::observe_snap(distance, tolerance, Some(uncertain));
            distance.abs() <= tolerance
        })
}

fn cover(
    fragments: Vec<Vec<Point>>,
    chart: &ConvexChart,
    support: Plane,
    pieces: &[ConvexPiece],
    [reject_same, reject_opposite]: [bool; 2],
    tolerance: f64,
    work: &mut Work<'_>,
) -> Result<Vec<Vec<Point>>, Error> {
    for piece in pieces {
        work.charge(piece.planes.len() * chart.ring.len() + 1)?;
    }
    let opposite = Plane {
        normal: scale(support.normal, -1.),
        ..support
    };
    clipping::covered_with_control(
        fragments,
        pieces,
        chart.bounds,
        tolerance,
        &mut work.remaining,
        |piece| {
            !piece.planes.iter().any(|&plane| {
                reject_same && coincident(chart, support, plane, tolerance, &work.uncertain)
                    || reject_opposite
                        && coincident(chart, opposite, plane, tolerance, &work.uncertain)
            })
        },
        clipping::Control {
            interrupt: work.interrupt,
            uncertain: Some(&work.uncertain),
        },
    )
}

fn outside(
    mut fragments: Vec<Vec<Point>>,
    chart: &ConvexChart,
    pieces: &[ConvexPiece],
    [reject_same, reject_opposite]: [bool; 2],
    tolerance: f64,
    work: &mut Work<'_>,
) -> Result<Vec<Vec<Point>>, Error> {
    let opposite = Plane {
        normal: scale(chart.support.normal, -1.),
        ..chart.support
    };
    for piece in pieces {
        work.charge(piece.planes.len() * chart.ring.len() + 1)?;
        if !clipping::bounds_overlap(chart.bounds, piece.bounds, tolerance)
            || piece.planes.iter().any(|&plane| {
                reject_same && coincident(chart, chart.support, plane, tolerance, &work.uncertain)
                    || reject_opposite
                        && coincident(chart, opposite, plane, tolerance, &work.uncertain)
            })
        {
            continue;
        }
        let mut remaining = Vec::new();
        let mut fragments_budget = MAX_INCIDENCES;
        for ring in fragments {
            work.check()?;
            clipping::partition_tracked(
                ring,
                &piece.planes,
                tolerance,
                &mut remaining,
                &mut work.remaining,
                &mut fragments_budget,
                Some(&work.uncertain),
            )?;
        }
        fragments = remaining;
        if fragments.is_empty() {
            break;
        }
    }
    Ok(fragments)
}

fn recover(
    operands: &[&CutMaterial],
    op: Operation,
    tolerance: f64,
    work: &mut Work<'_>,
) -> Result<Vec<ConvexPiece>, Error> {
    let mut pieces = Vec::new();
    for operand in if op == Operation::Union {
        operands
    } else {
        &operands[..1]
    } {
        charge_cells(&operand.pieces, work)?;
        pieces.extend(operand.pieces.iter().cloned());
    }
    if op == Operation::Union {
        return Ok(pieces);
    }
    for operand in &operands[1..] {
        let mut remaining = Vec::new();
        let mut identities = BTreeSet::new();
        if op == Operation::Subtract {
            for tool in &operand.pieces {
                for piece in pieces {
                    work.check()?;
                    subtract_cell_with_interrupt(
                        piece,
                        tool,
                        tolerance,
                        &mut work.remaining,
                        &mut remaining,
                        work.interrupt,
                    )?;
                    if remaining.len() > MAX_PIECES {
                        return Err(Error::Limit);
                    }
                }
                pieces = std::mem::take(&mut remaining);
            }
        } else {
            for piece in &pieces {
                for other in &operand.pieces {
                    work.charge(1)?;
                    if !clipping::bounds_overlap(piece.bounds, other.bounds, tolerance) {
                        continue;
                    }
                    charge_cells(std::slice::from_ref(piece), work)?;
                    if let Some(piece) = intersection::common_cell_with_interrupt(
                        piece.clone(),
                        other,
                        tolerance,
                        &mut work.remaining,
                        work.interrupt,
                    )? {
                        charge_cells(std::slice::from_ref(&piece), work)?;
                        if identities.insert(intersection::vertex_identity(&piece)) {
                            if remaining.len() == MAX_PIECES {
                                return Err(Error::Limit);
                            }
                            remaining.push(piece);
                        }
                    }
                }
            }
            pieces = remaining;
        }
    }
    if pieces.is_empty() {
        return Err(Error::Unresolved);
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests;
