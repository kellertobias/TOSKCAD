//! Exact source-role and periodic-chart ownership, independently of edge IDs.
use super::*;
fn half(body: &Chamfer, ends: [PhaseEndpoint; 2]) -> ConeHalf {
    let breaks = body.tool.profile_breaks();
    let valley = body
        .tool
        .root_circle()
        .map_or(breaks[2].midpoint(breaks[3]), |r| r.phase);
    if ends[0].nominal >= valley {
        ConeHalf::Rising
    } else {
        ConeHalf::Falling
    }
}
pub(super) fn expected_turn(body: &Chamfer, role: Face, curve: SourceCurve) -> Option<i32> {
    let n = body.tool.spec().starts;
    let turn = if body.tool.spec().left_hand { 1 } else { -1 };
    match (role, curve) {
        (
            Face::ThreadBand {
                start: owner,
                band: wall,
            },
            SourceCurve::Intersection { start, band, .. }
            | SourceCurve::EndProfile { start, band, .. },
        ) => {
            if body.wall_face_index(start, band) != Some(2 + 4 * owner + wall) {
                return None;
            }
            Some(if start == 0 && band == 0 { turn } else { 0 })
        }
        (
            Face::ThreadBand {
                start: owner,
                band: wall,
            },
            SourceCurve::WallRail { start, phase, .. },
        ) => {
            let PhaseConstraint::ProfileKnot(index @ 1..=4) = phase.constraint else {
                return None;
            };
            let face = 2 + 4 * owner + wall;
            if ![index - 1, index]
                .into_iter()
                .any(|b| body.wall_face_index(start, b) == Some(face))
            {
                return None;
            }
            Some(if owner + 1 == n && wall == 3 && start == 0 && index == 1 {
                turn
            } else {
                0
            })
        }
        (
            Face::ConeHalf {
                start: owner,
                half: side,
            },
            SourceCurve::Intersection {
                start,
                phase,
                active: true,
                ..
            }
            | SourceCurve::EntryArc { start, phase },
        ) => (owner == start && side == half(body, phase)).then_some(0),
        (
            Face::ConeHalf {
                start: owner,
                half: side,
            },
            SourceCurve::ConeRail { start, phase, .. },
        ) => {
            let breaks = body.tool.profile_breaks();
            let valley = body
                .tool
                .root_circle()
                .map_or(breaks[2].midpoint(breaks[3]), |r| r.phase);
            if phase.nominal == valley {
                return (owner == start).then_some(0);
            }
            match phase.constraint {
                PhaseConstraint::ProfileKnot(0) => {
                    if owner == start && side == ConeHalf::Falling {
                        Some(0)
                    } else if owner == (start + n - 1) % n && side == ConeHalf::Rising {
                        Some(if start == 0 { turn } else { 0 })
                    } else {
                        None
                    }
                }
                PhaseConstraint::ProfileKnot(5) => {
                    if owner == start && side == ConeHalf::Rising {
                        Some(0)
                    } else if owner == (start + 1) % n && side == ConeHalf::Falling {
                        Some(if start + 1 == n { -turn } else { 0 })
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        (
            Face::EntryCap,
            SourceCurve::Intersection { active: false, .. } | SourceCurve::EntryArc { .. },
        ) => Some(0),
        (Face::EndCap, SourceCurve::EndProfile { .. }) => Some(0),
        (Face::EndCap, SourceCurve::Circle { radius_mm, z_mm }) => body
            .terminal
            .filter(|r| radius_mm == r.neck_radius_mm() && z_mm == body.material_length_mm())
            .map(|_| 0),
        (Face::NeckWall, SourceCurve::Circle { radius_mm, z_mm }) => body
            .terminal
            .filter(|r| {
                radius_mm == r.neck_radius_mm()
                    && (z_mm == body.material_length_mm() || z_mm == body.full_length_mm())
            })
            .map(|_| 0),
        (Face::FarCap, SourceCurve::Circle { radius_mm, z_mm }) => body
            .terminal
            .filter(|r| radius_mm == r.neck_radius_mm() && z_mm == body.full_length_mm())
            .map(|_| 0),
        _ => None,
    }
}
pub(super) fn check(
    body: &Chamfer,
    role: Face,
    edge: &BoundaryEdge,
    usage: EdgeUse,
) -> Result<(), Error> {
    // The cylinder's two seam coedges retain its two actual angular branches.
    if let (
        Face::NeckWall,
        SourceCurve::NeckSeam {
            radius_mm,
            stations_mm,
        },
    ) = (role, edge.source)
    {
        if body
            .terminal
            .is_some_and(|r| radius_mm == r.neck_radius_mm())
            && stations_mm == [body.material_length_mm(), body.full_length_mm()]
            && usage.angular_turns == i32::from(usage.forward)
        {
            return Ok(());
        }
        return Err(Error::Unresolved);
    }
    let turn = expected_turn(body, role, edge.source);
    let forward = expected_forward(body, role, edge.source);
    if turn == Some(usage.angular_turns) && forward == Some(usage.forward) {
        Ok(())
    } else {
        Err(Error::Unresolved)
    }
}
/// Bind every carrier to an actual semantic source segment/endpoint before
/// interpreting face ordinals or periodic chart lifts. This is authored source
/// construction, not an expected-header test over decoded foreign geometry.
enum Span {
    Range([PhaseEndpoint; 2]),
    Point(PhaseEndpoint),
}
struct PhaseRule {
    start: u32,
    band: Option<usize>,
    span: Span,
    active: Option<bool>,
    noncontact: bool,
}
pub(super) fn source(body: &Chamfer, curve: SourceCurve, work: &mut Work<'_>) -> Result<(), Error> {
    work.step()?;
    let rule = match curve {
        SourceCurve::Intersection {
            start,
            band,
            phase,
            active,
        } => PhaseRule {
            start,
            band: Some(band),
            span: Span::Range(phase),
            active: Some(active),
            noncontact: false,
        },
        SourceCurve::EntryArc { start, phase } => PhaseRule {
            start,
            band: None,
            span: Span::Range(phase),
            active: Some(true),
            noncontact: false,
        },
        SourceCurve::EndProfile { start, band, phase } => PhaseRule {
            start,
            band: Some(band),
            span: Span::Range(phase),
            active: None,
            noncontact: false,
        },
        SourceCurve::WallRail {
            start,
            band,
            phase,
            active,
        } => PhaseRule {
            start,
            band: Some(band),
            span: Span::Point(phase),
            active: Some(active),
            noncontact: false,
        },
        SourceCurve::ConeRail { start, band, phase } => PhaseRule {
            start,
            band: Some(band),
            span: Span::Point(phase),
            active: Some(true),
            noncontact: true,
        },
        SourceCurve::Circle { radius_mm, z_mm } => {
            return if body
                .terminal
                .is_some_and(|r| radius_mm == r.neck_radius_mm())
                && (z_mm == body.material_length_mm() || z_mm == body.full_length_mm())
            {
                Ok(())
            } else {
                Err(Error::Unresolved)
            };
        }
        SourceCurve::NeckSeam {
            radius_mm,
            stations_mm,
        } => {
            return if body
                .terminal
                .is_some_and(|r| radius_mm == r.neck_radius_mm())
                && stations_mm == [body.material_length_mm(), body.full_length_mm()]
            {
                Ok(())
            } else {
                Err(Error::Unresolved)
            };
        }
    };
    for segment in body.phase_segments()? {
        work.step()?;
        let ends = segment.ends.map(|k| builder::endpoint(body, segment, k));
        let span = match rule.span {
            Span::Range(p) => p == ends,
            Span::Point(p) => ends.contains(&p) && (!rule.noncontact || !p.contact),
        };
        if rule.start < body.tool.spec().starts
            && rule.band.is_none_or(|b| b == segment.band)
            && rule.active.is_none_or(|a| a == segment.active)
            && span
        {
            return Ok(());
        }
    }
    Err(Error::Unresolved)
}

/// The q,z charts have radial normal sign -sign(rate). Cap polar orientation
/// follows theta=(z-q)/rate; these independent boundary signs prevent a
/// consistently connected but entirely inward shell from being certified.
fn expected_forward(body: &Chamfer, role: Face, curve: SourceCurve) -> Option<bool> {
    let left = body.tool.spec().left_hand;
    match (role, curve) {
        (Face::ThreadBand { .. }, SourceCurve::Intersection { .. }) => Some(left),
        (Face::ThreadBand { .. }, SourceCurve::EndProfile { .. }) => Some(!left),
        (
            Face::ThreadBand {
                start: owner,
                band: wall,
            },
            SourceCurve::WallRail { start, phase, .. },
        ) => {
            let PhaseConstraint::ProfileKnot(index @ 1..=4) = phase.constraint else {
                return None;
            };
            Some(
                if body.wall_face_index(start, index) == Some(2 + 4 * owner + wall) {
                    !left
                } else {
                    left
                },
            )
        }
        (Face::ConeHalf { .. }, SourceCurve::Intersection { .. }) => Some(!left),
        (Face::ConeHalf { .. }, SourceCurve::EntryArc { .. }) => Some(left),
        (Face::ConeHalf { half, .. }, SourceCurve::ConeRail { phase, .. }) => {
            let breaks = body.tool.profile_breaks();
            let valley = body
                .tool
                .root_circle()
                .map_or(breaks[2].midpoint(breaks[3]), |r| r.phase);
            let lower = if phase.nominal == valley {
                half == ConeHalf::Rising
            } else {
                half == ConeHalf::Falling
            };
            Some(if lower { !left } else { left })
        }
        (Face::EntryCap, _) => Some(!left),
        (Face::EndCap, SourceCurve::EndProfile { .. }) => Some(left),
        (Face::EndCap, SourceCurve::Circle { .. }) => Some(false),
        (Face::NeckWall, SourceCurve::Circle { z_mm, .. }) => {
            Some(z_mm == body.material_length_mm())
        }
        (Face::FarCap, SourceCurve::Circle { .. }) => Some(true),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn work() -> Work<'static> {
        Work::new(QueryOptions::default(), None).unwrap()
    }
    #[test]
    fn all_role_orientation_and_periodic_branch_combinations_match_only_actual_boundary_uses() {
        for left in [false, true] {
            let body = Chamfer::new(
                super::super::super::tests::tool(left, 2, true),
                0.5,
                Some(TerminalIntent {
                    width_mm: 0.6,
                    radial_depth_mm: 0.1,
                }),
            )
            .unwrap();
            let topology = body.boundary_topology(QueryOptions::default()).unwrap();
            for (id, edge) in topology.edges().iter().enumerate() {
                for face in topology.faces() {
                    let role = face.role();
                    for forward in [false, true] {
                        for angular_turns in [-1, 0, 1] {
                            let usage = EdgeUse {
                                edge: id,
                                forward,
                                angular_turns,
                            };
                            let expected = face.loops().iter().flatten().any(|u| *u == usage);
                            assert_eq!(
                                check(&body, role, edge, usage).is_ok(),
                                expected,
                                "{role:?} {:?} {usage:?}",
                                edge.source
                            );
                        }
                    }
                }
            }
            // Authentic internal strip rails are not exterior semantic edges.
            for segment in body.phase_segments().unwrap() {
                for knot in segment.ends {
                    let end = builder::endpoint(&body, segment, knot);
                    if !matches!(end.constraint, PhaseConstraint::ProfileKnot(_)) {
                        let curve = SourceCurve::WallRail {
                            start: 0,
                            band: segment.band,
                            phase: end,
                            active: segment.active,
                        };
                        source(&body, curve, &mut work()).unwrap();
                        let role = body
                            .face(body.wall_face_index(0, segment.band).unwrap())
                            .unwrap();
                        assert_eq!(expected_turn(&body, role, curve), None);
                        assert_eq!(expected_forward(&body, role, curve), None);
                    }
                }
            }
            let segment = body.phase_segments().unwrap()[0];
            let end = builder::endpoint(&body, segment, segment.ends[1]);
            let inner = SourceCurve::ConeRail {
                start: 0,
                band: segment.band,
                phase: end,
            };
            source(&body, inner, &mut work()).unwrap();
            assert_eq!(
                expected_turn(
                    &body,
                    Face::ConeHalf {
                        start: 0,
                        half: ConeHalf::Falling
                    },
                    inner
                ),
                None
            );
            for segment in body.phase_segments().unwrap() {
                for knot in segment.ends {
                    let end = builder::endpoint(&body, segment, knot);
                    if end.contact {
                        let zero = SourceCurve::ConeRail {
                            start: 0,
                            band: segment.band,
                            phase: end,
                        };
                        assert_eq!(source(&body, zero, &mut work()), Err(Error::Unresolved));
                    }
                }
            }
        }
    }
    #[test]
    fn alternate_true_periodic_carriers_keep_their_actual_parameter_lifts() {
        for left in [false, true] {
            let body =
                Chamfer::new(super::super::super::tests::tool(left, 2, false), 0.3, None).unwrap();
            let segments = body.phase_segments().unwrap();
            let first = segments[0];
            let last = *segments.last().unwrap();
            let turn = if left { 1 } else { -1 };
            let origin = SourceCurve::ConeRail {
                start: 1,
                band: first.band,
                phase: builder::endpoint(&body, first, first.ends[0]),
            };
            source(&body, origin, &mut work()).unwrap();
            assert_eq!(
                expected_turn(
                    &body,
                    Face::ConeHalf {
                        start: 0,
                        half: ConeHalf::Rising
                    },
                    origin
                ),
                Some(0)
            );
            let wrapped = SourceCurve::ConeRail {
                start: 1,
                band: last.band,
                phase: builder::endpoint(&body, last, last.ends[1]),
            };
            source(&body, wrapped, &mut work()).unwrap();
            assert_eq!(
                expected_turn(
                    &body,
                    Face::ConeHalf {
                        start: 0,
                        half: ConeHalf::Falling
                    },
                    wrapped
                ),
                Some(-turn)
            );
            let wall = SourceCurve::WallRail {
                start: 0,
                band: first.band,
                phase: builder::endpoint(&body, first, first.ends[0]),
                active: true,
            };
            assert_eq!(
                curve::rail_binding_error(&body, wall, origin, 0),
                Err(Error::Invalid)
            );
            let circle = SourceCurve::Circle {
                radius_mm: 1.,
                z_mm: 0.,
            };
            assert_eq!(
                curve::rail_binding_error(&body, wall, circle, 0),
                Err(Error::Invalid)
            );
        }
    }
}
