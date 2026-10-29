//! Member connections: paired miters, square butt joints and copes between
//! two explicit members. A connection is an extension naming two member
//! Parts; the cuts are derived from both members' current resolved geometry
//! whenever either member is built, so path and section edits update both
//! ends. Each member keeps its own Part.
//!
//! The joint point is where the two member paths meet. Members whose paths
//! are parallel or pass each other without meeting are not connected.
use super::*;
use crate::{model::common::number, parameters::EvaluatedParameters};
use resolve::vec3::*;

pub const CONNECTION_KIND: &str = "weldment.connection";
pub const CONNECTION_CAPABILITY: &str = "weldment.connection.1";
/// Smallest angle between two connected member axes.
pub const MIN_CONNECTION_ANGLE_DEG: f64 = 15.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// Both members end at the joint and are cut on the plane bisecting
    /// them. Their sections must be mirror images across that plane.
    Miter,
    /// The first member ends square against a flat face of the second,
    /// which continues through. When the second also ends at the joint, its
    /// end is cut square to cover the first.
    Butt,
    /// The first member (branch) is fitted to the actual outside of the
    /// second (receiving) member, plus the gap.
    Cope,
}
impl ConnectionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Miter => "miter",
            Self::Butt => "butt",
            Self::Cope => "cope",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDefinition {
    pub kind: ConnectionKind,
    /// Member Parts. Miter: either order. Butt: terminating, then
    /// continuing. Cope: branch, then receiving.
    pub members: [EntityId; 2],
    /// Clearance between the cut end and the face it meets (a miter gap is
    /// shared equally by both members).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<Dimension>,
}
impl ConnectionDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.members.iter().any(EntityId::is_nil) || self.members[0] == self.members[1] {
            return Err(invalid("A connection needs two different members"));
        }
        if let Some(g) = &self.gap {
            check_dimension(g, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(self.gap.iter().collect());
        links.push(Link::new(self.members[0], "member/0"));
        links.push(Link::new(self.members[1], "member/1"));
        links
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(CONNECTION_KIND, self)?,
            inputs: self.inputs(),
        })
    }
    pub fn gap_mm(&self, parameters: &EvaluatedParameters) -> Result<f64, DocumentError> {
        let g = self
            .gap
            .as_ref()
            .map_or(Ok(0.), |g| number(g, Unit::Millimeter, parameters))?;
        if g < 0. {
            return Err(invalid("A connection gap cannot be negative"));
        }
        Ok(g)
    }
}
fn is_connection(entity: &Entity) -> bool {
    matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == CONNECTION_KIND)
}
pub fn connections(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| is_connection(e))
}
pub fn read_connection(
    document: &Document,
    id: EntityId,
) -> Result<ConnectionDefinition, DocumentError> {
    let definition = match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. }) if definition.type_id == CONNECTION_KIND => {
            definition
        }
        Some(_) => return Err(invalid(format!("{id} is not a weldment connection"))),
        None => return Err(DocumentError::Missing(id)),
    };
    let value: ConnectionDefinition = parse(definition, CONNECTION_KIND)?;
    value.validate()?;
    Ok(value)
}
/// Connections naming a member Part, in document order.
pub fn member_connections(
    document: &Document,
    part: EntityId,
) -> Result<Vec<(EntityId, ConnectionDefinition)>, DocumentError> {
    let mut out = vec![];
    for e in connections(document) {
        let c = read_connection(document, e.id)?;
        if c.members.contains(&part) {
            out.push((e.id, c));
        }
    }
    Ok(out)
}
pub(super) fn validate_connections(document: &Document) -> Result<(), DocumentError> {
    let mut pairs = BTreeSet::new();
    for e in connections(document) {
        let c = read_connection(document, e.id)?;
        for part in c.members {
            if document.get(part).is_some() && member_feature(document, part).is_none() {
                return Err(invalid(format!(
                    "Connection {} names a Part that is not a weldment member",
                    e.name
                )));
            }
        }
        let mut key = c.members;
        key.sort();
        if !pairs.insert(key) {
            return Err(invalid(format!(
                "Two connections join the same members; edit {} instead",
                e.name
            )));
        }
    }
    Ok(())
}

/// Which end of a member a cut applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberEnd {
    Start,
    End,
}
/// How a member end is cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EndCut {
    /// Keep the material on the side of the plane `normal` points to.
    Plane { point: [f64; 3], normal: [f64; 3] },
    /// Remove the other member's outside envelope grown by `grow_mm`, and
    /// the material beyond the plane through the receiving section center
    /// (keep the side `normal` points to).
    Cope {
        grow_mm: f64,
        point: [f64; 3],
        normal: [f64; 3],
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndTreatment {
    pub end: MemberEnd,
    pub kind: ConnectionKind,
    pub cut: EndCut,
    /// Plane cuts: the angle of the cut face from square to the member
    /// axis (0 = square, 45 = a right-angle miter). Copes: the angle
    /// between the branch and receiving axes.
    pub angle_deg: f64,
    /// Whether this member terminates at the joint (false for a continuing
    /// member that is cut square to cover its partner).
    pub terminates: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedConnection {
    pub kind: ConnectionKind,
    /// Where the member paths meet.
    pub point: [f64; 3],
    /// Angle between the member axes, each pointing away from the joint
    /// (or along the member for one that passes through), in degrees.
    pub angle_deg: f64,
    pub gap_mm: f64,
    /// Treatments of the first and second member.
    pub treatments: [Option<EndTreatment>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Position {
    At(MemberEnd),
    Interior,
}
fn position(m: &ResolvedMember, p: [f64; 3]) -> Result<Position, DocumentError> {
    if distance(p, m.path.start) <= PATH_TOLERANCE_MM {
        return Ok(Position::At(MemberEnd::Start));
    }
    if distance(p, m.path.end) <= PATH_TOLERANCE_MM {
        return Ok(Position::At(MemberEnd::End));
    }
    let s = m.axial(p);
    if s > 0. && s < m.path_length_mm {
        Ok(Position::Interior)
    } else {
        Err(invalid(
            "The members' paths meet beyond the end of one member; extend its path to the joint",
        ))
    }
}
/// Unit direction from the joint into the member body.
fn away(m: &ResolvedMember, end: MemberEnd) -> [f64; 3] {
    match end {
        MemberEnd::Start => m.frame.axis,
        MemberEnd::End => scale(m.frame.axis, -1.),
    }
}
fn at_end(m: &ResolvedMember, p: [f64; 3], role: &str) -> Result<MemberEnd, DocumentError> {
    match position(m, p)? {
        Position::At(end) => Ok(end),
        Position::Interior => Err(invalid(format!(
            "The {role} member must end at the joint; its path passes through it"
        ))),
    }
}
fn angle_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    dot(a, b).clamp(-1., 1.).acos().to_degrees()
}
/// The extent of a member's placed outline beyond `p` along `direction`,
/// which must be square to the member axis.
fn extent(m: &ResolvedMember, p: [f64; 3], direction: [f64; 3]) -> f64 {
    let s = m.axial(p);
    if m.section.shape == crate::materials::ProfileShape::Round {
        let center = m.point([m.section.width_mm / 2., m.section.height_mm / 2.], s);
        return dot(sub(center, p), direction) + m.section.width_mm / 2.;
    }
    m.outline_world(s)
        .into_iter()
        .map(|q| dot(sub(q, p), direction))
        .fold(f64::NEG_INFINITY, f64::max)
}
/// Where two member paths meet: the closest points of the two axis lines
/// must coincide.
fn meeting_point(a: &ResolvedMember, b: &ResolvedMember) -> Result<[f64; 3], DocumentError> {
    let (da, db) = (a.frame.axis, b.frame.axis);
    if norm(cross(da, db)) < 1e-9 {
        return Err(invalid(
            "The members are parallel; connect members whose paths meet at an angle",
        ));
    }
    // Closest points p = a0 + t·da, q = b0 + s·db.
    let w = sub(a.path.start, b.path.start);
    let (b_, d, e) = (dot(da, db), dot(da, w), dot(db, w));
    let denom = 1. - b_ * b_;
    let t = (b_ * e - d) / denom;
    let s = (e - b_ * d) / denom;
    let p = add(a.path.start, scale(da, t));
    let q = add(b.path.start, scale(db, s));
    let gap = distance(p, q);
    if gap > PATH_TOLERANCE_MM {
        return Err(invalid(format!(
            "The member paths do not meet; they pass {gap:.3} mm apart. Non-coplanar connections are not supported"
        )));
    }
    Ok(scale(add(p, q), 0.5))
}
fn reflect(x: [f64; 3], p: [f64; 3], n: [f64; 3]) -> [f64; 3] {
    sub(x, scale(n, 2. * dot(sub(x, p), n)))
}
/// Whether the sections of `a` and `b` are mirror images across the plane
/// through `p` with normal `n`, at the joint.
fn mirrored(a: &ResolvedMember, b: &ResolvedMember, p: [f64; 3], n: [f64; 3]) -> bool {
    use crate::materials::ProfileShape;
    let (sa, sb) = (a.axial(p), b.axial(p));
    match (a.section.shape, b.section.shape) {
        (ProfileShape::Round, ProfileShape::Round) => {
            let ca = reflect(
                a.point([a.section.width_mm / 2., a.section.height_mm / 2.], sa),
                p,
                n,
            );
            let cb = b.point([b.section.width_mm / 2., b.section.height_mm / 2.], sb);
            (a.section.width_mm - b.section.width_mm).abs() <= PATH_TOLERANCE_MM
                && distance(ca, cb) <= PATH_TOLERANCE_MM
        }
        (x, y) if x == y => {
            let pa: Vec<_> = a
                .outline_world(sa)
                .into_iter()
                .map(|q| reflect(q, p, n))
                .collect();
            let pb = b.outline_world(sb);
            pa.len() == pb.len()
                && pa
                    .iter()
                    .all(|q| pb.iter().any(|r| distance(*q, *r) <= PATH_TOLERANCE_MM))
        }
        _ => false,
    }
}
fn largest_extent(m: &ResolvedMember, p: [f64; 3], direction: [f64; 3]) -> f64 {
    let s = m.axial(p);
    let values: Vec<f64> = m
        .outline_world(s)
        .into_iter()
        .map(|q| dot(q, direction))
        .collect();
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let round = matches!(m.section.shape, crate::materials::ProfileShape::Round);
    // Round outline points are quadrant points; the true width is the
    // diameter in every direction square to the axis.
    if round { m.section.width_mm } else { max - min }
}

/// Derive both members' end treatments from their current geometry.
pub fn resolve_connection(
    kind: ConnectionKind,
    a: &ResolvedMember,
    b: &ResolvedMember,
    gap_mm: f64,
) -> Result<ResolvedConnection, DocumentError> {
    let p = meeting_point(a, b)?;
    let min = MIN_CONNECTION_ANGLE_DEG;
    match kind {
        ConnectionKind::Miter => {
            let (ea, eb) = (at_end(a, p, "first")?, at_end(b, p, "second")?);
            let (ua, ub) = (away(a, ea), away(b, eb));
            let angle_deg = angle_between(ua, ub);
            if angle_deg < min {
                return Err(invalid(format!(
                    "The members meet at {angle_deg:.1}°; miters need at least {min}°"
                )));
            }
            let n = unit(sub(ua, ub)).ok_or_else(|| invalid("Degenerate miter"))?;
            if !mirrored(a, b, p, n) {
                return Err(invalid(
                    "A miter needs sections that mirror each other across the miter plane (same section, size, anchor and orientation); use a butt joint or cope for different sections",
                ));
            }
            let cut_angle = angle_between(n, ua);
            let half = gap_mm / 2.;
            Ok(ResolvedConnection {
                kind,
                point: p,
                angle_deg,
                gap_mm,
                treatments: [
                    Some(EndTreatment {
                        end: ea,
                        kind,
                        cut: EndCut::Plane {
                            point: add(p, scale(n, half)),
                            normal: n,
                        },
                        angle_deg: cut_angle,
                        terminates: true,
                    }),
                    Some(EndTreatment {
                        end: eb,
                        kind,
                        cut: EndCut::Plane {
                            point: sub(p, scale(n, half)),
                            normal: scale(n, -1.),
                        },
                        angle_deg: cut_angle,
                        terminates: true,
                    }),
                ],
            })
        }
        ConnectionKind::Butt => {
            let et = at_end(a, p, "terminating")?;
            let ut = away(a, et);
            if b.section.shape != crate::materials::ProfileShape::Rectangular {
                return Err(invalid(
                    "Butt joints need a rectangular continuing member with flat faces; use a cope against round or angle members",
                ));
            }
            let face = [b.frame.x, b.frame.y]
                .into_iter()
                .any(|f| dot(f, ut).abs() > 1. - 1e-9);
            if !face {
                return Err(invalid(
                    "A butt joint needs the terminating member square to a flat face of the continuing member; use a miter or cope for other angles",
                ));
            }
            let h = extent(b, p, ut);
            let mut treatments = [
                Some(EndTreatment {
                    end: et,
                    kind,
                    cut: EndCut::Plane {
                        point: add(p, scale(ut, h + gap_mm)),
                        normal: ut,
                    },
                    angle_deg: 0.,
                    terminates: true,
                }),
                None,
            ];
            let mut angle_deg = 90.;
            if let Position::At(ec) = position(b, p)? {
                // A corner: the continuing member is cut square to cover
                // the terminating member's far side.
                let uc = away(b, ec);
                angle_deg = angle_between(ut, uc);
                let e = extent(a, p, scale(uc, -1.));
                treatments[1] = Some(EndTreatment {
                    end: ec,
                    kind,
                    cut: EndCut::Plane {
                        point: sub(p, scale(uc, e)),
                        normal: uc,
                    },
                    angle_deg: 0.,
                    terminates: false,
                });
            }
            Ok(ResolvedConnection {
                kind,
                point: p,
                angle_deg,
                gap_mm,
                treatments,
            })
        }
        ConnectionKind::Cope => {
            let eb = at_end(a, p, "branch")?;
            position(b, p)?;
            let ub = away(a, eb);
            let axis = b.frame.axis;
            let angle_deg = angle_between(ub, axis).min(angle_between(ub, scale(axis, -1.)));
            if angle_deg < min {
                return Err(invalid(format!(
                    "The branch meets the receiving member at {angle_deg:.1}°; copes need at least {min}°"
                )));
            }
            let w = unit(sub(ub, scale(axis, dot(ub, axis))))
                .ok_or_else(|| invalid("Degenerate cope"))?;
            // Across both axes, the branch may not be wider than the
            // receiving member it saddles.
            let v = unit(cross(axis, ub)).ok_or_else(|| invalid("Degenerate cope"))?;
            let (wb, wr) = (largest_extent(a, p, v), largest_extent(b, p, v));
            if wb > wr + 2. * gap_mm + PATH_TOLERANCE_MM {
                return Err(invalid(format!(
                    "The branch is {wb:.1} mm wide across the receiving member, which is only {wr:.1} mm; use a butt joint"
                )));
            }
            let s = b.axial(p);
            let center = b.point([b.section.width_mm / 2., b.section.height_mm / 2.], s);
            Ok(ResolvedConnection {
                kind,
                point: p,
                angle_deg,
                gap_mm,
                treatments: [
                    Some(EndTreatment {
                        end: eb,
                        kind,
                        cut: EndCut::Cope {
                            grow_mm: gap_mm,
                            point: center,
                            normal: w,
                        },
                        angle_deg,
                        terminates: true,
                    }),
                    None,
                ],
            })
        }
    }
}
