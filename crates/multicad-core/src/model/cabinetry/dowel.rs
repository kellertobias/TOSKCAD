//! Paired circular drilling uses the same exact panel contact, placement and
//! blind-depth checks as loose tenons. A zero-length rounded slot is a circle;
//! the returned cuts explicitly carry circles, not polygon approximations.
use super::*;
use crate::{model::common::number, model::length, parameters::EvaluatedParameters};

pub fn resolve_dowels(
    document: &Document,
    parameters: &EvaluatedParameters,
    spec: &DowelSpec,
    pair: &PanelJointPair,
) -> Result<(PanelJointCorner, LooseTenonLayout), DocumentError> {
    spec.validate()?;
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let diameter = mm(&spec.diameter)?;
    let fit = mm(&spec.fit)?;
    let face_depth = mm(&spec.face_depth)?;
    let edge_depth = mm(&spec.edge_depth)?;
    if diameter <= 0. || fit < 0. || face_depth <= 0. || edge_depth <= 0. {
        return Err(invalid(
            "Dowel diameter and receiving depths must be positive; fit cannot be negative",
        ));
    }
    let circular = LooseTenonSpec {
        thickness: length(diameter),
        width: length(diameter + fit),
        length: length(face_depth + edge_depth),
        count: spec.count,
        inset: spec.inset.clone(),
        offset: Some(spec.offset.clone()),
        face_depth: length(face_depth),
        edge_depth: length(edge_depth),
        face_play: length(0.),
        edge_play: length(0.),
        fit: length(fit),
    };
    let (mut cuts, layout) = resolve_loose_tenon(document, parameters, &circular, pair)?;
    let radius = (diameter + fit) / 2.;
    for hole in &layout.tenons {
        let (point, _) = layout.edge_frame.local(hole.center);
        if (0..2).any(|i| {
            point[i] - radius < MIN_WALL_MM - 1e-6
                || point[i] + radius > layout.edge_frame.size[i] - MIN_WALL_MM + 1e-6
        }) {
            return Err(invalid(format!(
                "Dowel holes must leave at least {MIN_WALL_MM} mm around the receiving panel edge; reduce diameter/fit or move their centers"
            )));
        }
    }
    for cut in cuts.cut_a.iter_mut().chain(&mut cuts.cut_b) {
        let (a, b, r) = cut
            .slot
            .take()
            .ok_or_else(|| invalid("Dowel contact did not yield a circular drilling tool"))?;
        if (a[0] - b[0]).hypot(a[1] - b[1]) > 1e-6 {
            return Err(invalid("Dowel receiving tool must be circular"));
        }
        cut.circle = Some((a, r));
    }
    Ok((cuts, layout))
}
