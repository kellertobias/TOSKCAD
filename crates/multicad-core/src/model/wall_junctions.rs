//! Explicit perpendicular flush-butt T junctions. The host remains continuous;
//! branch material ends meet its physical face, without implied layer wrapping.
use super::{ResolvedWall, WallDefinition, WallFace, WallPlan, source::invalid};
use crate::{
    document::{Document, DocumentError, EntityData, EntityId},
    parameters::EvaluatedParameters,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const WALL_JUNCTION_CAPABILITY: &str = "architecture.wall_junctions.1";
const EPS: f64 = 1e-6;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallEnd {
    Start,
    End,
}
impl WallEnd {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallJunctionStyle {
    #[default]
    FlushButt,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallJunction {
    pub end: WallEnd,
    pub host_wall: EntityId,
    pub host_segment: usize,
    pub host_point_count: usize,
    pub host_closed: bool,
    #[serde(default)]
    pub style: WallJunctionStyle,
}
impl WallJunction {
    pub(super) fn validate_all(junctions: &[Self]) -> Result<(), DocumentError> {
        let mut ends = BTreeSet::new();
        if junctions.len() > 2 {
            return Err(invalid("A wall has at most two joined ends"));
        }
        for junction in junctions {
            if junction.host_wall.is_nil()
                || junction.host_point_count < if junction.host_closed { 3 } else { 2 }
                || junction.host_point_count > super::MAX_WALL_POINTS
                || junction.host_segment
                    >= if junction.host_closed {
                        junction.host_point_count
                    } else {
                        junction.host_point_count.saturating_sub(1)
                    }
                || !ends.insert(junction.end)
            {
                return Err(invalid(
                    "Joined wall ends need distinct ends and a valid host wall/segment",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedWallJunction {
    pub end: WallEnd,
    pub host_wall: EntityId,
    pub host_segment: usize,
    pub host_face: WallFace,
    pub origin_mm: [f64; 2],
    /// Positive half-plane is the retained branch side.
    pub inward_normal: [f64; 2],
    /// Signed extra length beyond the authored path end; negative means trim.
    pub end_extension_mm: f64,
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn add_scaled(a: [f64; 2], b: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] + b[0] * s, a[1] + b[1] * s]
}

pub(super) fn resolve_junctions(
    document: &Document,
    parameters: &EvaluatedParameters,
    source: &WallDefinition,
    branch: &ResolvedWall,
    depth: usize,
    remaining: &mut usize,
) -> Result<Vec<ResolvedWallJunction>, DocumentError> {
    let junctions = source
        .layer_placement
        .as_ref()
        .map(|p| p.junctions.as_slice())
        .unwrap_or_default();
    if junctions.is_empty() {
        return Ok(vec![]);
    }
    if depth >= 64 {
        return Err(invalid(
            "Wall junction host chain exceeds 64; simplify the host relationships",
        ));
    }
    if branch.closed || branch.layers.is_none() {
        return Err(invalid(
            "Flush-butt T junctions require an open layered branch wall",
        ));
    }
    let branch_feature = match document.get(source.output.body).map(|e| &e.data) {
        Some(EntityData::Part { features, .. }) if features.len() == 1 => features[0],
        _ => {
            return Err(invalid(
                "Joined branch must be a direct architectural wall; downstream generic geometry requires explicit repair",
            ));
        }
    };
    let branch_segments = branch.segments()?;
    let mut output = vec![];
    for junction in junctions {
        if junction.host_wall == branch_feature {
            return Err(invalid("A wall cannot join its own path"));
        }
        let host_entity = document
            .get(junction.host_wall)
            .ok_or(DocumentError::Missing(junction.host_wall))?;
        if !matches!(&host_entity.data,EntityData::Feature {definition,suppressed:false,..} if definition.type_id==super::WALL_TYPE)
        {
            return Err(invalid("Join host must be an active wall feature"));
        }
        let host_source = super::read_wall(document, junction.host_wall)?;
        if host_source.path.len() != junction.host_point_count
            || host_source.closed != junction.host_closed
        {
            return Err(invalid(
                "Join host path topology changed; explicitly reattach and align that end",
            ));
        }
        if !matches!(document.get(host_source.output.body).map(|e|&e.data),Some(EntityData::Part {features,..}) if features.as_slice()==[junction.host_wall])
        {
            return Err(invalid(
                "Join host has downstream generic geometry; choose an unmodified architectural wall or repair the junction",
            ));
        }
        let host = super::wall::resolve_wall_at_depth(
            document,
            parameters,
            &host_source,
            depth + 1,
            remaining,
        )?;
        let host_segments = host.segments()?;
        let segment = host_segments
            .get(junction.host_segment)
            .ok_or_else(|| invalid("Join host segment was removed; repair the junction"))?;
        let (endpoint, interior, branch_segment) = match junction.end {
            WallEnd::Start => (branch.points[0], branch.points[1], branch_segments[0]),
            WallEnd::End => {
                let n = branch.points.len();
                (
                    branch.points[n - 1],
                    branch.points[n - 2],
                    branch_segments[n - 2],
                )
            }
        };
        if dot(branch_segment.direction, segment.direction).abs() > 1e-8 {
            return Err(invalid(
                "Only perpendicular flush-butt T junctions are supported; align the branch end or remove the join",
            ));
        }
        if dot(sub(endpoint, segment.start), segment.normal).abs() > EPS {
            return Err(invalid(
                "Joined endpoint must lie on the host path; align it before previewing",
            ));
        }
        if branch.base_z_mm < host.base_z_mm - EPS
            || branch.base_z_mm + branch.height_mm > host.base_z_mm + host.height_mm + EPS
        {
            return Err(invalid(
                "Flush-butt T junction needs host material through the branch's full height; repair levels or heights",
            ));
        }
        let side = dot(sub(interior, segment.start), segment.normal);
        if side.abs() <= EPS {
            return Err(invalid("Branch has no approach side at the host wall"));
        }
        let sign = side.signum();
        let face = if sign > 0. {
            WallFace::Left
        } else {
            WallFace::Right
        };
        let (left, right) = host.physical_face_offsets()?;
        let offset = if sign > 0. { left } else { right };
        let datum = add_scaled(
            segment.start,
            segment.direction,
            dot(sub(endpoint, segment.start), segment.direction),
        );
        let origin = add_scaled(datum, segment.normal, offset);
        let normal = segment.normal.map(|v| v * sign);
        if dot(sub(interior, origin), normal) <= EPS {
            return Err(invalid(
                "Joined branch segment is too short to reach the host physical face",
            ));
        }
        let physical = host.plan_region(left, right)?;
        let role = format!(
            "wall.seg.{}.{}",
            junction.host_segment,
            if sign > 0. { "left" } else { "right" }
        );
        let host_edge = std::iter::once(&physical.outer)
            .chain(&physical.holes)
            .find_map(|loop_| {
                loop_
                    .roles
                    .iter()
                    .position(|r| r == &role)
                    .map(|i| (loop_.points[i], loop_.points[(i + 1) % loop_.points.len()]))
            })
            .ok_or_else(|| invalid("Host join face is unavailable; repair the junction"))?;
        let face_range = [
            dot(sub(host_edge.0, segment.start), segment.direction),
            dot(sub(host_edge.1, segment.start), segment.direction),
        ];
        let (branch_left, branch_right) = branch.physical_face_offsets()?;
        let along = dot(sub(endpoint, segment.start), segment.direction);
        let cross = dot(branch_segment.normal, segment.direction);
        let contact = [along + cross * branch_left, along + cross * branch_right];
        let (low, high) = (contact[0].min(contact[1]), contact[0].max(contact[1]));
        if low <= face_range[0].min(face_range[1]) + EPS
            || high >= face_range[0].max(face_range[1]) - EPS
        {
            return Err(invalid(
                "Flush-butt T contact must stay inside one host segment, away from corners and ends",
            ));
        }
        if host.openings.iter().any(|o| {
            o.segment == junction.host_segment
                && high > o.offset_mm + EPS
                && low < o.offset_mm + o.width_mm - EPS
                && branch.base_z_mm < host.base_z_mm + o.sill_mm + o.height_mm - EPS
                && branch.base_z_mm + branch.height_mm > host.base_z_mm + o.sill_mm + EPS
        }) {
            return Err(invalid(
                "T junction crosses a host door/window void; move or repair the join",
            ));
        }
        if host.cutouts.iter().any(|c| {
            c.segment == junction.host_segment
                && (c.through || c.face == face)
                && high > c.offset_mm + EPS
                && low < c.offset_mm + c.width_mm - EPS
        }) {
            return Err(invalid(
                "T junction crosses a host chase/cutout on that face; move or repair the join",
            ));
        }
        output.push(ResolvedWallJunction {
            end: junction.end,
            host_wall: junction.host_wall,
            host_segment: junction.host_segment,
            host_face: face,
            origin_mm: origin,
            inward_normal: normal,
            end_extension_mm: dot(sub(endpoint, origin), normal),
        });
    }
    Ok(output)
}

impl ResolvedWallJunction {
    /// Replace just the semantic end cap, preserving every side/corner role.
    /// This supports a small extension through a host's exterior air cavity as
    /// well as trimming an endpoint drawn to an interior reference datum.
    pub(super) fn terminate(&self, plan: &mut WallPlan) -> Result<(), DocumentError> {
        if !plan.holes.is_empty() {
            return Err(invalid("Closed branch walls cannot have joined ends"));
        }
        let loop_ = &mut plan.outer;
        let n = loop_.points.len();
        let cap = loop_
            .roles
            .iter()
            .position(|r| r == &format!("wall.{}", self.end.label()))
            .ok_or_else(|| invalid("Wall end topology is unavailable for the junction"))?;
        let a = cap;
        let b = (cap + 1) % n;
        let before = (cap + n - 1) % n;
        let after = (cap + 2) % n;
        let distance = |p| dot(sub(p, self.origin_mm), self.inward_normal);
        if loop_
            .points
            .iter()
            .enumerate()
            .any(|(i, p)| i != a && i != b && distance(*p) <= EPS)
        {
            return Err(invalid(
                "Join termination would cross another branch corner/segment; shorten or simplify the path",
            ));
        }
        let intersect = |interior: [f64; 2], end: [f64; 2]| -> Result<[f64; 2], DocumentError> {
            let inside = distance(interior);
            let outside = distance(end);
            let denominator = inside - outside;
            if !denominator.is_finite() || denominator.abs() < EPS {
                return Err(invalid("Join end cannot intersect the host face"));
            }
            let t = inside / denominator;
            let p = add_scaled(interior, sub(end, interior), t);
            if p.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
                return Err(invalid(
                    "Join termination exceeds the supported coordinate range",
                ));
            }
            Ok(p)
        };
        let new_a = intersect(loop_.points[before], loop_.points[a])?;
        let new_b = intersect(loop_.points[after], loop_.points[b])?;
        loop_.points[a] = new_a;
        loop_.points[b] = new_b;
        Ok(())
    }
}

impl WallJunction {
    /// Read-only suggestion for an explicit Align action. The caller replaces
    /// only the selected endpoint in its local draft; this is not a transaction
    /// or a native geometry proof.
    pub fn aligned_endpoint(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        path: &[[crate::constraint_sketch::Dimension; 2]],
        closed: bool,
    ) -> Result<[f64; 2], DocumentError> {
        if closed || path.len() < 2 {
            return Err(invalid(
                "Align requires an open branch path with at least two points",
            ));
        }
        let host = super::read_wall(document, self.host_wall)?;
        let host = super::resolve_wall(document, parameters, &host)?;
        let segments = host.segments()?;
        let segment = segments
            .get(self.host_segment)
            .ok_or_else(|| invalid("Host segment was removed; choose another join host"))?;
        let index = match self.end {
            WallEnd::Start => 1,
            WallEnd::End => path.len() - 2,
        };
        let point = [
            super::common::number(
                &path[index][0],
                crate::parameters::Unit::Millimeter,
                parameters,
            )?,
            super::common::number(
                &path[index][1],
                crate::parameters::Unit::Millimeter,
                parameters,
            )?,
        ];
        let along = dot(sub(point, segment.start), segment.direction);
        if along <= EPS || along >= segment.length_mm - EPS {
            return Err(invalid(
                "Perpendicular endpoint projection is outside the host segment",
            ));
        }
        Ok(add_scaled(segment.start, segment.direction, along))
    }
}
