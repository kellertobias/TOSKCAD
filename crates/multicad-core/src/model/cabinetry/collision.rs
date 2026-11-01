//! Exact circular/capsule footprints and blind depth intervals for receiving
//! cuts. Check generated connections against each other and existing drilling
//! before a native candidate can be applied. General pockets remain subject
//! to their existing native panel validation.
use super::*;
use crate::parameters::EvaluatedParameters;
use std::collections::BTreeMap;
const EPS: f64 = 1e-6;

struct Cut {
    operation: EntityId,
    generated: bool,
    a: [f64; 3],
    b: [f64; 3],
    axis: [f64; 3],
    radius: f64,
    depth: f64,
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn delta(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = delta(b, a);
    let v = delta(p, a);
    let length = d[0] * d[0] + d[1] * d[1];
    let t = if length > 1e-18 {
        ((v[0] * d[0] + v[1] * d[1]) / length).clamp(0., 1.)
    } else {
        0.
    };
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}
fn segment_distance(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    let (r, s) = (delta(b, a), delta(d, c));
    let den = cross(r, s);
    if den.abs() > 1e-12 {
        let q = delta(c, a);
        let (t, u) = (cross(q, s) / den, cross(q, r) / den);
        if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&u) {
            return 0.;
        }
    }
    [
        point_segment(a, c, d),
        point_segment(b, c, d),
        point_segment(c, a, b),
        point_segment(d, a, b),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}
fn overlaps(a: &Cut, b: &Cut) -> bool {
    let cosine = dot(a.axis, b.axis);
    // All cabinet-generated receiving cuts on one panel have parallel or
    // opposed axes. Keep general differently oriented machining on its native
    // validation path instead of guessing from a bounding box.
    if cosine.abs() < 1. - 1e-9 {
        return false;
    }
    let offset = dot(sub(b.a, a.a), a.axis);
    let (lo, hi) = if cosine > 0. {
        (offset, offset + b.depth)
    } else {
        (offset - b.depth, offset)
    };
    if a.depth.min(hi) <= 0_f64.max(lo) + EPS {
        return false;
    }
    let reference = if a.axis[0].abs() < 0.9 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    let mut u = super::resolve::cross(a.axis, reference);
    let length = norm(u);
    u = u.map(|v| v / length);
    let v = super::resolve::cross(a.axis, u);
    let project = |p| [dot(p, u), dot(p, v)];
    segment_distance(project(a.a), project(a.b), project(b.a), project(b.b))
        < a.radius + b.radius - EPS
}
fn from_prism(prism: &CutPrism, operation: EntityId, generated: bool) -> Option<Cut> {
    let (a, b, radius) = match (prism.circle, prism.slot) {
        (Some((c, r)), _) => (c, c, r),
        (None, Some((a, b, r))) => (a, b, r),
        _ => return None,
    };
    let length = norm(prism.sweep);
    if length <= EPS {
        return None;
    }
    let axis = prism.sweep.map(|v| v / length);
    let at = |p: [f64; 2]| {
        std::array::from_fn(|i| {
            prism.origin[i]
                + prism.u[i] * p[0]
                + prism.v[i] * p[1]
                + axis[i] * super::machining::MARGIN_MM
        })
    };
    Some(Cut {
        operation,
        generated,
        a: at(a),
        b: at(b),
        axis,
        radius,
        depth: length - super::machining::MARGIN_MM,
    })
}

pub fn check_cabinet_connections(
    document: &Document,
    parameters: &EvaluatedParameters,
    owner: EntityId,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), DocumentError> {
    if read_carcass(document, owner)?.cabinet.is_none() {
        return Ok(());
    }
    let parts: BTreeSet<_> = carcass_panels(document, owner)?
        .into_iter()
        .map(|(_, _, p)| p)
        .collect();
    let active = |entity: &Entity| {
        matches!(
            entity.data,
            EntityData::Feature {
                suppressed: false,
                ..
            }
        )
    };
    let mut joint_cuts = BTreeSet::new();
    for entity in panel_joint_cuts(document).filter(|e| active(e)) {
        let cut = read_panel_joint_cut(document, entity.id)?;
        joint_cuts.insert((
            cut.joint,
            cut.pair,
            match cut.side {
                PanelJointSide::A => 0_u8,
                PanelJointSide::B => 1,
            },
        ));
    }
    let mut machining_cuts_active = BTreeSet::new();
    for entity in machining_cuts(document).filter(|e| active(e)) {
        let cut = read_machining_cut(document, entity.id)?;
        machining_cuts_active.insert((cut.operation, cut.output.body));
    }
    let mut by_part: BTreeMap<EntityId, Vec<Cut>> = BTreeMap::new();
    for entity in panel_joints(document) {
        let joint = read_panel_joint(document, entity.id)?;
        if !matches!(
            joint.kind,
            PanelJointKind::Dowels(_) | PanelJointKind::LooseTenon(_)
        ) {
            continue;
        }
        for (i, pair) in joint.pairs.iter().enumerate() {
            if interrupt.is_some_and(|f| f()) {
                return Err(invalid("Cabinet connection evaluation superseded"));
            }
            let a_active =
                parts.contains(&pair.a) && joint_cuts.contains(&(entity.id, i as u32, 0));
            let b_active =
                parts.contains(&pair.b) && joint_cuts.contains(&(entity.id, i as u32, 1));
            if !a_active && !b_active {
                continue;
            }
            let cuts = resolve_panel_joint_pair(document, parameters, &joint, i)?;
            for (part, prisms, enabled) in [
                (pair.a, cuts.cut_a, a_active),
                (pair.b, cuts.cut_b, b_active),
            ] {
                if enabled {
                    by_part
                        .entry(part)
                        .or_default()
                        .extend(prisms.iter().filter_map(|p| {
                            from_prism(p, entity.id, joint.generated_by == Some(owner))
                        }));
                }
            }
        }
    }
    for entity in machinings(document) {
        let operation = read_machining(document, entity.id)?;
        for part in operation
            .targets
            .iter()
            .filter(|p| parts.contains(p) && machining_cuts_active.contains(&(entity.id, **p)))
        {
            if interrupt.is_some_and(|f| f()) {
                return Err(invalid("Cabinet connection evaluation superseded"));
            }
            let resolved = resolve_machining(document, parameters, &operation, *part)?;
            by_part
                .entry(*part)
                .or_default()
                .extend(resolved.holes.into_iter().map(|hole| Cut {
                    operation: entity.id,
                    generated: operation.generated_by == Some(owner),
                    a: hole.center,
                    b: hole.center,
                    axis: hole.axis,
                    radius: hole.diameter_mm / 2.,
                    depth: hole.depth_mm,
                }));
        }
    }
    for (part, cuts) in by_part {
        for (i, a) in cuts.iter().enumerate() {
            if interrupt.is_some_and(|f| f()) {
                return Err(invalid("Cabinet connection evaluation superseded"));
            }
            for b in &cuts[i + 1..] {
                if a.operation != b.operation && (a.generated || b.generated) && overlaps(a, b) {
                    let name = |id| {
                        document
                            .get(id)
                            .map_or("unnamed operation", |e| e.name.as_str())
                    };
                    return Err(invalid(format!(
                        "Cabinet receiving cuts of {} and {} overlap on {}; change connection spacing/depths or shelf-pin rows before Apply",
                        name(a.operation),
                        name(b.operation),
                        name(part)
                    )));
                }
            }
        }
    }
    Ok(())
}
