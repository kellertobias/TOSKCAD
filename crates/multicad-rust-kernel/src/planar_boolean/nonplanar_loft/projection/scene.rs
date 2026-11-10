//! Shared orthographic visibility for planar, round and bilinear bodies.
use super::*;
use crate::planar_projection::{Body, Options, ProjectionError, prepare_loft_faces};
use crate::{curved_projection, round_occlusion::RoundOccluder};
use std::cell::Cell;
use visibility::{Candidate, Cap, Wall};

pub enum LoftSceneMember<'a> {
    Loft {
        body: &'a NonplanarLoft,
        axes: [Point; 3],
        offset: Point,
    },
    Planar(&'a Body),
    Round(&'a RoundOccluder),
    RetainedRound {
        body: &'a RoundOccluder,
        edges: &'a [LoftRoundCutEdge],
        near_trimmed: bool,
    },
}

/// An exact cut curve in the cyclic coordinate chart of a clipping plane.
#[derive(Clone, Copy, Debug)]
pub struct LoftRoundCutEdge {
    /// Coordinate fixed by the clipping plane, from paper U, V and depth.
    pub axis: usize,
    pub level: f64,
    /// Coordinates along the next two cyclic axes.
    pub curve: curved_projection::Curve,
}

#[derive(Clone, Copy, Debug)]
pub struct LoftSceneCut {
    pub near: f64,
    pub far: Option<f64>,
    pub windows: [Option<[f64; 2]>; 2],
}

#[derive(Debug, PartialEq)]
pub struct LoftSceneSection {
    pub edges: Vec<(usize, LoftDrawingEdge)>,
    /// Indices of retained loft and round members that have section geometry.
    pub clipped_members: Vec<usize>,
}

const MAX_WORK: usize = 20_000_000;

struct ProjectedLoft<'a> {
    source: usize,
    rings: [Vec<Point>; 2],
    walls: Vec<Wall>,
    clip: Option<clip::Clip<'a>>,
}

pub fn project_loft_scene(
    members: &[LoftSceneMember<'_>],
    options: Options,
) -> Result<Vec<(usize, LoftDrawingEdge)>, Error> {
    project_with_limit(members, options, None, MAX_WORK).map(|result| result.edges)
}

/// The planar members must already be clipped to the same cut. Their
/// retained planar faces and the exact loft trims then share visibility.
pub fn project_loft_scene_section(
    members: &[LoftSceneMember<'_>],
    options: Options,
    cut: LoftSceneCut,
) -> Result<LoftSceneSection, Error> {
    project_with_limit(members, options, Some(cut), MAX_WORK)
}

pub(super) fn project_with_limit(
    members: &[LoftSceneMember<'_>],
    options: Options,
    cut: Option<LoftSceneCut>,
    limit: usize,
) -> Result<LoftSceneSection, Error> {
    if cut.is_some_and(|cut| {
        !cut.near.is_finite()
            || cut
                .far
                .is_some_and(|far| !far.is_finite() || cut.near - far <= options.tolerance)
            || cut
                .windows
                .into_iter()
                .flatten()
                .any(|[lo, hi]| !lo.is_finite() || !hi.is_finite() || hi - lo <= options.tolerance)
    }) {
        return Err(Error::Invalid);
    }
    let planes = cut.map(|cut| {
        [
            cut.windows[0],
            cut.windows[1],
            Some([cut.far.unwrap_or(f64::NEG_INFINITY), cut.near]),
        ]
    });
    let planar: Vec<_> = members
        .iter()
        .filter_map(|member| match member {
            LoftSceneMember::Planar(body) => Some(*body),
            LoftSceneMember::Loft { .. }
            | LoftSceneMember::Round(_)
            | LoftSceneMember::RetainedRound { .. } => None,
        })
        .collect();
    let prepared = prepare_loft_faces(&planar, &options, limit).map_err(|cause| match cause {
        ProjectionError::Invalid => Error::Invalid,
        ProjectionError::Limit => Error::Limit,
    })?;
    let planar_faces = prepared.faces;
    let mut lofts = Vec::new();
    let mut rounds = Vec::new();
    let mut clipped_rounds = Vec::new();
    let mut curves = Vec::new();
    let mut coordinate_scale = 1_f64;
    let mut tangent = prepared.tangent.into_iter();
    for (source, member) in members.iter().enumerate() {
        match *member {
            LoftSceneMember::Loft { body, axes, offset } => {
                if !valid_axes(axes)
                    || offset
                        .iter()
                        .any(|x| !x.is_finite() || x.abs() > MAX_COORDINATE)
                {
                    return Err(Error::Invalid);
                }
                let rings = body.rings().each_ref().map(|ring| {
                    ring.iter()
                        .map(|&point| add(axes.map(|axis| dot(axis, point)), offset))
                        .collect::<Vec<_>>()
                });
                let scale = rings
                    .iter()
                    .flatten()
                    .flatten()
                    .map(|x| x.abs())
                    .fold(1., f64::max);
                let epsilon = (64. * f64::EPSILON * scale).min(options.tolerance);
                coordinate_scale = coordinate_scale.max(scale);
                let (clip, mut edges) = if let Some(planes) = planes {
                    let Some((clip, edges)) = clip::prepare(
                        body,
                        &rings,
                        (axes, offset),
                        planes,
                        options.tolerance,
                        epsilon,
                    ) else {
                        continue;
                    };
                    (Some(clip), edges)
                } else {
                    (None, Vec::new())
                };
                let walls = visibility::loft_curves(&rings, epsilon, &mut edges);
                lofts.push(ProjectedLoft {
                    source,
                    rings,
                    walls,
                    clip,
                });
                curves.extend(
                    edges
                        .into_iter()
                        .map(|edge| Candidate::new(source, edge, true)),
                );
            }
            LoftSceneMember::Round(body) => {
                if planes.is_some_and(|planes| {
                    body.bounds()
                        .into_iter()
                        .any(|point| !clip::includes(planes, point, options.tolerance))
                }) {
                    return Err(Error::Unresolved);
                }
                coordinate_scale = body
                    .bounds()
                    .into_iter()
                    .flatten()
                    .map(f64::abs)
                    .fold(coordinate_scale, f64::max);
                rounds.push((source, body, false));
                curves.extend(round::candidates(
                    source,
                    body,
                    options.hidden_lines,
                    false,
                )?);
            }
            LoftSceneMember::RetainedRound {
                body,
                edges,
                near_trimmed,
            } => {
                let Some(planes) = planes else {
                    return Err(Error::Invalid);
                };
                let [min, max] = body.bounds();
                if planes.into_iter().enumerate().any(|(axis, window)| {
                    window.is_some_and(|[lo, hi]| {
                        max[axis] <= lo + options.tolerance || min[axis] >= hi - options.tolerance
                    })
                }) {
                    continue;
                }
                coordinate_scale = min
                    .into_iter()
                    .chain(max)
                    .map(f64::abs)
                    .fold(coordinate_scale, f64::max);
                rounds.push((source, body, true));
                if near_trimmed {
                    clipped_rounds.push(source);
                }
                curves.extend(round::candidates(source, body, options.hidden_lines, true)?);
                for &edge in edges {
                    round::add_cut_candidates(source, edge, planes, &mut curves)?;
                }
            }
            LoftSceneMember::Planar(body) => {
                let tags = tangent.next().expect("tangent flags for every planar body");
                for (&[a, b], tangent) in body.edges.iter().zip(tags) {
                    coordinate_scale = a
                        .into_iter()
                        .chain(b)
                        .map(f64::abs)
                        .fold(coordinate_scale, f64::max);
                    curves.push(Candidate::new(
                        source,
                        Curve::line(a, b),
                        options.tangent_edges || !tangent,
                    ));
                }
            }
        }
    }
    let caps: Vec<_> = lofts
        .iter()
        .map(|loft| loft.rings.each_ref().map(|ring| Cap::new(ring)))
        .collect();
    let work = Cell::new(prepared.remaining);
    let charge = |cost| {
        let remaining = work.get().checked_sub(cost).ok_or(Error::Limit)?;
        work.set(remaining);
        Ok(())
    };
    charge(curves.len().saturating_mul(curves.len()))?;
    let wall_count: usize = lofts.iter().map(|loft| loft.walls.len()).sum();
    let ray_cost = 4 * wall_count + planar_faces.iter().map(|p| p.boundary_len()).sum::<usize>();
    let near = cut.map_or(f64::INFINITY, |cut| cut.near);
    let blocked = |source, rim: Option<_>, point| {
        charge(ray_cost + rounds.len())?;
        Ok(planar_faces
            .iter()
            .any(|face| face.covers(point, options.tolerance))
            || caps
                .iter()
                .flatten()
                .any(|cap| cap.blocks(point, near, options.tolerance))
            || lofts
                .iter()
                .flat_map(|loft| &loft.walls)
                .any(|wall| wall.blocks(point, near, options.tolerance))
            || lofts
                .iter()
                .filter_map(|loft| loft.clip.as_ref())
                .any(|clip| clip.blocks(point, options.tolerance))
            || rounds.iter().any(|&(owner, round, retained)| {
                let contour = |rim| {
                    if retained {
                        source == owner && round.covers_axial_contour(rim, near, options.tolerance)
                    } else {
                        round.covers_coincident_rim(rim, options.tolerance)
                    }
                };
                (source != owner || retained)
                    && (rim.is_some_and(contour)
                        || round.blocks_point(
                            point,
                            if retained { near } else { f64::INFINITY },
                            options.tolerance,
                        ))
            }))
    };
    let epsilon = (64. * f64::EPSILON * coordinate_scale).min(options.tolerance);
    let depth_events = |source, curve: Curve, bounds, cuts: &mut Vec<f64>| {
        charge(planar_faces.len() + 4 * (wall_count + rounds.len()))?;
        for face in &planar_faces {
            curve.plane_crossings(face.point, face.normal, cuts);
        }
        for (loft, caps) in lofts
            .iter()
            .zip(&caps)
            .filter(|(loft, _)| loft.source != source)
        {
            for cap in caps {
                cap.depth_events(curve, cuts);
            }
            for wall in &loft.walls {
                wall.depth_events(curve, bounds, epsilon, cuts);
            }
        }
        for &(owner, round, _) in &rounds {
            if owner != source {
                round::depth_events(round, curve, bounds, epsilon, cuts);
            }
        }
        Ok(())
    };
    let edges = visibility::publish(
        curves,
        LoftProjection {
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            stations: [0., 1.],
            depth: None,
            windows: [None; 2],
            tolerance: options.tolerance,
            hidden_lines: options.hidden_lines,
            detail: options.detail,
            edge_limit: options.segment_limit,
        },
        epsilon,
        planes,
        &blocked,
        &depth_events,
    )?;
    let mut visible = vec![false; members.len()];
    for &(source, edge) in &edges {
        visible[source] |= !edge.hidden;
    }
    let mut clipped_members: Vec<_> = lofts
        .iter()
        .filter(|loft| visible[loft.source] && loft.clip.as_ref().is_some_and(|clip| clip.changed))
        .map(|loft| loft.source)
        .chain(clipped_rounds.into_iter().filter(|&source| visible[source]))
        .collect();
    clipped_members.sort_unstable();
    Ok(LoftSceneSection {
        edges,
        clipped_members,
    })
}
