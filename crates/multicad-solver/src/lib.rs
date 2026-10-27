use sketch_solver::{Along, Constraint, Pair, Sketch, Solution, Status};

mod adapter;
pub use adapter::*;
mod transaction;
pub use transaction::*;

/// Solve a perturbed rectangle. Solver handles stay private to this adapter.
pub fn rectangle(width: f64, depth: f64) -> Result<[f64; 2], String> {
    let (solution, corner) = solve(width, depth, None)?;
    if solution.status != Status::Solved || solution.dof != 0 {
        return Err(format!(
            "Sketch {:?}; {} free coordinates",
            solution.status, solution.dof
        ));
    }
    let [x, y] = solution.point(corner).ok_or("Missing solved corner")?;
    if (x - width).abs() > 1e-7 || (y - depth).abs() > 1e-7 {
        return Err("Solver residual exceeds modeling handoff tolerance".into());
    }
    Ok([x, y])
}

fn solve(
    width: f64,
    depth: f64,
    duplicate_width: Option<f64>,
) -> Result<(Solution, sketch_solver::EntityId), String> {
    if ![width, depth]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.01 && *v <= 100_000.0)
    {
        return Err("Sketch dimensions must be finite and within 0.01–100000 mm".into());
    }
    let mut s = Sketch::new();
    let o = s.point(Some([0.0, 0.0])).map_err(|e| e.to_string())?;
    let a = s
        .point(Some([width * 0.98, 0.1]))
        .map_err(|e| e.to_string())?;
    let b = s
        .point(Some([width * 0.99, depth * 0.98]))
        .map_err(|e| e.to_string())?;
    let c = s
        .point(Some([0.1, depth * 1.01]))
        .map_err(|e| e.to_string())?;
    let bottom = s.line(o, a).map_err(|e| e.to_string())?;
    let right = s.line(a, b).map_err(|e| e.to_string())?;
    let top = s.line(b, c).map_err(|e| e.to_string())?;
    let left = s.line(c, o).map_err(|e| e.to_string())?;
    for constraint in [
        Constraint::Fix {
            entity: o,
            at: None,
        },
        Constraint::Horizontal(Pair::Line(bottom)),
        Constraint::Vertical(Pair::Line(right)),
        Constraint::Horizontal(Pair::Line(top)),
        Constraint::Vertical(Pair::Line(left)),
        Constraint::Distance {
            a: o,
            b: a,
            value: width,
            along: Along::X,
        },
        Constraint::Distance {
            a,
            b,
            value: depth,
            along: Along::Y,
        },
    ] {
        s.add(constraint).map_err(|e| e.to_string())?;
    }
    if let Some(value) = duplicate_width {
        s.add(Constraint::Distance {
            a: o,
            b: a,
            value,
            along: Along::X,
        })
        .map_err(|e| e.to_string())?;
    }
    Ok((s.solve(), b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fully_constrained_rectangle_and_conflict_diagnostics() {
        assert_eq!(rectangle(100.0, 60.0).unwrap(), [100.0, 60.0]);
        let (conflict, _) = solve(100.0, 60.0, Some(101.0)).unwrap();
        assert_ne!(conflict.status, Status::Solved);
        assert!(!conflict.conflicts.is_empty());
        assert!(rectangle(f64::NAN, 60.0).is_err());
        assert!(rectangle(-1.0, 60.0).is_err());
    }
}
