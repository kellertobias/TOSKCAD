//! Shared orthographic visibility for planar bodies and bilinear lofts.
use super::*;
use crate::planar_projection::{Body, Options, ProjectionError, prepare_loft_faces};
use std::cell::Cell;
use visibility::{Cap, Wall};

pub enum LoftSceneMember<'a> {
    Loft {
        body: &'a NonplanarLoft,
        axes: [Point; 3],
        offset: Point,
    },
    Planar(&'a Body),
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
    /// Indices of loft members with retained geometry changed by a cut.
    pub clipped_lofts: Vec<usize>,
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
            LoftSceneMember::Loft { .. } => None,
        })
        .collect();
    let prepared = prepare_loft_faces(&planar, &options, limit).map_err(|cause| match cause {
        ProjectionError::Invalid => Error::Invalid,
        ProjectionError::Limit => Error::Limit,
    })?;
    let planar_faces = prepared.faces;
    let mut lofts = Vec::new();
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
                curves.extend(edges.into_iter().map(|edge| (source, edge, true)));
            }
            LoftSceneMember::Planar(body) => {
                let tags = tangent.next().expect("tangent flags for every planar body");
                for (&[a, b], tangent) in body.edges.iter().zip(tags) {
                    coordinate_scale = a
                        .into_iter()
                        .chain(b)
                        .map(f64::abs)
                        .fold(coordinate_scale, f64::max);
                    curves.push((source, Curve::line(a, b), options.tangent_edges || !tangent));
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
    let blocked = |point| {
        charge(ray_cost)?;
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
                .any(|clip| clip.blocks(point, options.tolerance)))
    };
    let epsilon = (64. * f64::EPSILON * coordinate_scale).min(options.tolerance);
    let depth_events = |source, curve: Curve, bounds, cuts: &mut Vec<f64>| {
        charge(planar_faces.len() + 4 * wall_count)?;
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
    let clipped_lofts = lofts
        .iter()
        .filter(|loft| visible[loft.source] && loft.clip.as_ref().is_some_and(|clip| clip.changed))
        .map(|loft| loft.source)
        .collect();
    Ok(LoftSceneSection {
        edges,
        clipped_lofts,
    })
}
