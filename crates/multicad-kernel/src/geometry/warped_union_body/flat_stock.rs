//! Promote rectilinear stock for mixed sloped and axial cuts.
use super::*;
use multicad_rust_kernel::orthogonal::Box3;

pub(super) fn chart(
    cells: &[Box3],
    topology: TopologySnapshot,
    policy: GeometryPolicy,
) -> Result<Chart, GeometryError> {
    let mut cells = cells.to_vec();
    for axis in [2, 1, 0] {
        cells = merge_axis(cells, axis);
    }
    let mut chart = Chart::new(None);
    chart.topologies.push(Rc::new(topology));
    for cell in cells {
        let piece = ConvexPiece::from_box(cell.min, cell.max, policy.linear_tolerance_mm)
            .map_err(source_error)?;
        chart.pieces.push(piece);
        chart.sources.push(SourcePart {
            operand: 0,
            faces: Vec::new(),
        });
    }
    chart.stock_count = chart.pieces.len();
    Ok(chart)
}

// Merge only face-adjacent cells with exactly equal transverse intervals.
// This restores continuous cheek faces across artificial shelf-height seams
// without crossing a material boundary or moving any authored coordinate.
fn merge_axis(mut cells: Vec<Box3>, axis: usize) -> Vec<Box3> {
    let [a, b] = [(axis + 1) % 3, (axis + 2) % 3];
    let transverse = |cell: &Box3| [cell.min[a], cell.max[a], cell.min[b], cell.max[b]];
    cells.sort_unstable_by(|a, b| {
        transverse(a)
            .into_iter()
            .zip(transverse(b))
            .map(|(a, b)| a.total_cmp(&b))
            .find(|order| !order.is_eq())
            .unwrap_or_else(|| a.min[axis].total_cmp(&b.min[axis]))
    });
    let mut merged = Vec::<Box3>::with_capacity(cells.len());
    for cell in cells {
        if let Some(last) = merged.last_mut()
            && transverse(last) == transverse(&cell)
            && last.max[axis] == cell.min[axis]
        {
            last.max[axis] = cell.max[axis];
        } else {
            merged.push(cell);
        }
    }
    merged
}
