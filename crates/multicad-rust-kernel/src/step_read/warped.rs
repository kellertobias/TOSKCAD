use super::*;
use crate::{
    planar_boolean::{self, Union},
    trim_curve::{Ellipse, EllipseArc},
    warped_union::{self, Band},
};
use std::f64::consts::FRAC_PI_2;

mod bosses;

#[derive(Debug)]
pub struct ImportedWarped {
    pub shell: warped_union::Shell,
    pub origin_mm: Point,
    pub u: Point,
    pub v: Point,
}

/// Decode one solid in the bounded quarter-round analytic STEP profile.
pub fn read_warped_solid(input: &str, tolerance_mm: f64) -> Result<ImportedWarped> {
    compound::read_members_by(input, tolerance_mm, true, true, |reader, id, _| {
        read_member(reader, id).map(|(_, imported)| imported)
    })?
    .pop()
    .ok_or(ReadError::Invalid)
}

#[derive(Clone, Copy)]
enum TrimCurve {
    Line,
    Arc(EllipseArc),
}

#[derive(Clone, Copy)]
enum Surface {
    Plane {
        origin: Point,
        normal: Point,
    },
    Cylinder {
        origin: Point,
        axis: Point,
        radius: f64,
        outward: bool,
    },
}

#[cfg_attr(test, derive(Clone))]
struct FaceLoop {
    surface: Surface,
    loop_: Vec<(u32, bool)>,
    holes: Vec<Vec<(u32, bool)>>,
}

#[cfg_attr(test, derive(Clone))]
struct Boundary {
    faces: Vec<FaceLoop>,
    curves: BTreeMap<u32, ([u32; 2], TrimCurve)>,
}

impl Boundary {
    fn parse(reader: &mut Shell<'_, '_>, solid: u32) -> Result<Self> {
        let face_ids = reader.member_faces(solid)?;
        let mut faces = Vec::with_capacity(face_ids.len());
        let mut curves = BTreeMap::new();
        let mut uses = BTreeMap::<u32, [usize; 2]>::new();
        for id in face_ids {
            let [Value::String(_), bounds, surface, forward] = reader.args(id, "ADVANCED_FACE")?
            else {
                return Err(ReadError::Invalid);
            };
            let surface = surface_geometry(reader, reference(surface)?, sense(forward)?)?;
            let bounds = refs(bounds)?;
            let implicit_outer = bounds.len() == 1;
            let mut outer = None;
            let mut holes = Vec::new();
            let mut remaining = MAX_VERTICES;
            for bound in bounds {
                let (exterior, wire, sense_) = reader.bound(bound, implicit_outer)?;
                let loop_ =
                    Self::parse_loop(reader, wire, sense_, remaining, &mut curves, &mut uses)?;
                remaining -= loop_.len();
                if exterior {
                    if outer.replace(loop_).is_some() {
                        return Err(ReadError::Invalid);
                    }
                } else {
                    holes.push(loop_);
                }
            }
            let loop_ = outer.ok_or(ReadError::Unsupported)?;
            faces.push(FaceLoop {
                surface,
                loop_,
                holes,
            });
        }
        if uses.values().any(|&usage| usage != [1, 1]) {
            return Err(ReadError::Invalid);
        }
        Ok(Self { faces, curves })
    }

    fn parse_loop(
        reader: &mut Shell<'_, '_>,
        wire: u32,
        sense_: bool,
        remaining: usize,
        curves: &mut BTreeMap<u32, ([u32; 2], TrimCurve)>,
        uses: &mut BTreeMap<u32, [usize; 2]>,
    ) -> Result<Vec<(u32, bool)>> {
        let [Value::String(_), oriented] = reader.args(wire, "EDGE_LOOP")? else {
            return Err(ReadError::Invalid);
        };
        let mut oriented = refs(oriented)?;
        if oriented.len() > remaining {
            return Err(ReadError::Limit);
        }
        if !sense_ {
            oriented.reverse();
        }
        let mut loop_ = Vec::with_capacity(oriented.len());
        let mut vertices = Vec::with_capacity(oriented.len());
        for use_ in oriented {
            let [
                Value::String(_),
                Value::Derived,
                Value::Derived,
                edge,
                direction,
            ] = reader.args(use_, "ORIENTED_EDGE")?
            else {
                return Err(ReadError::Invalid);
            };
            let edge = reference(edge)?;
            let forward = sense(direction)? == sense_;
            let (ends, _) = match curves.entry(edge) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(trim_curve(reader, edge)?)
                }
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            };
            let ends = if forward { *ends } else { [ends[1], ends[0]] };
            vertices.push(ends);
            loop_.push((edge, forward));
            uses.entry(edge).or_default()[usize::from(forward)] += 1;
        }
        if vertices.len() < 3
            || vertices
                .iter()
                .zip(vertices.iter().cycle().skip(1))
                .any(|(a, b)| a[1] != b[0])
        {
            return Err(ReadError::Invalid);
        }
        Ok(loop_)
    }

    fn readback(&self, reader: &Shell<'_, '_>, volume_mm3: f64) -> Result<Readback> {
        let anchor = *reader.points.values().next().ok_or(ReadError::Invalid)?;
        let mut bounds_mm = [anchor; 2];
        for &point in reader.points.values() {
            include_point(&mut bounds_mm, point);
        }
        let mut edges_mm = Vec::new();
        let mut arcs = Vec::new();
        let mut ellipses = Vec::new();
        for &(vertices, curve) in self.curves.values() {
            match curve {
                TrimCurve::Line => edges_mm.push(vertices.map(|id| reader.points[&id])),
                TrimCurve::Arc(arc) => {
                    let [lower, upper] = &mut bounds_mm;
                    for (i, (min, max)) in lower.iter_mut().zip(upper).enumerate() {
                        let axis = std::array::from_fn(|j| f64::from(j == i));
                        let [lo, hi] = arc.range_along(axis);
                        *min = min.min(lo);
                        *max = max.max(hi);
                    }
                    let ellipse = arc.ellipse();
                    let [major, minor] = ellipse.semiaxes_mm();
                    if major == minor {
                        let center_mm = ellipse.center_mm();
                        let [radial_start, radial_end] = arc
                            .endpoints()
                            .map(|p| sub(p, center_mm).map(|x| x / major));
                        arcs.push(ArcEdge {
                            center_mm,
                            normal: ellipse.normal().map(|v| v * arc.sweep_rad().signum()),
                            radius_mm: major,
                            radial_start,
                            radial_end,
                            sweep_rad: arc.sweep_rad().abs(),
                        });
                    } else {
                        ellipses.push(arc);
                    }
                }
            }
        }
        Ok(Readback {
            bounds_mm,
            volume_mm3,
            face_count: self.faces.len(),
            edges_mm,
            circles: Vec::new(),
            semicircles: Vec::new(),
            arcs,
            ellipses,
            solid_count: 1,
        })
    }
}

fn surface_geometry(reader: &Shell<'_, '_>, id: u32, forward: bool) -> Result<Surface> {
    let entity = reader.simple(id)?;
    match (entity.name, entity.args.as_slice()) {
        ("PLANE", [Value::String(_), place]) => {
            let plane = reader.placement(reference(place)?)?;
            Ok(Surface::Plane {
                origin: plane.origin,
                normal: plane.normal.map(|v| if forward { v } else { -v }),
            })
        }
        ("CYLINDRICAL_SURFACE", [Value::String(_), place, Value::Number(radius)]) => {
            let plane = reader.placement(reference(place)?)?;
            if *radius <= reader.tolerance || *radius > 1e8 {
                return Err(ReadError::Invalid);
            }
            Ok(Surface::Cylinder {
                origin: plane.origin,
                axis: plane.normal,
                radius: *radius,
                outward: forward,
            })
        }
        _ => Err(ReadError::Unsupported),
    }
}

fn trim_curve(reader: &mut Shell<'_, '_>, id: u32) -> Result<([u32; 2], TrimCurve)> {
    let [Value::String(_), start, end, geometry, forward] = reader.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    let vertices = [reference(start)?, reference(end)?];
    let forward = sense(forward)?;
    let geometry = reference(geometry)?;
    let entity = reader.simple(geometry)?;
    let (placement, axes) = match (entity.name, entity.args.as_slice()) {
        ("LINE", _) => {
            reader.line_curve(geometry, vertices, forward)?;
            return Ok((vertices, TrimCurve::Line));
        }
        ("CIRCLE", [Value::String(_), place, Value::Number(radius)]) => {
            (reference(place)?, [*radius; 2])
        }
        (
            "ELLIPSE",
            [
                Value::String(_),
                place,
                Value::Number(major),
                Value::Number(minor),
            ],
        ) => (reference(place)?, [*major, *minor]),
        _ => return Err(ReadError::Unsupported),
    };
    let Plane {
        origin, normal, u, ..
    } = reader.placement(placement)?;
    let ellipse = Ellipse::new(origin, normal, u, axes).map_err(|_| ReadError::Invalid)?;
    let ends = [reader.vertex(vertices[0])?, reader.vertex(vertices[1])?];
    let arc = EllipseArc::from_endpoints(ellipse, ends, !forward, reader.tolerance)
        .map_err(|_| ReadError::Invalid)?;
    if arc.sweep_rad().abs() > FRAC_PI_2 + 1e-8 {
        return Err(ReadError::Unsupported);
    }
    Ok((vertices, TrimCurve::Arc(arc)))
}

#[derive(Clone, Copy)]
struct Chart {
    origin: Point,
    u: Point,
    v: Point,
    up: Point,
    band: Band,
}

impl Chart {
    fn local(self, point: Point) -> Point {
        self.vector(sub(point, self.origin))
    }

    fn vector(self, vector: Point) -> Point {
        [
            dot(vector, self.u),
            dot(vector, self.v),
            dot(vector, self.up),
        ]
    }

    fn height(self, z: f64, tolerance: f64) -> f64 {
        let Band {
            center_z_mm: c,
            radius_mm: r,
        } = self.band;
        let dz = z - c;
        let scale = (64. * f64::EPSILON * r.max(z.abs()).max(c.abs())).min(tolerance);
        if dz < -r - scale {
            dz + r
        } else if dz <= -r + scale {
            0.
        } else if dz < -scale {
            ((r + dz) * (r - dz)).sqrt()
        } else if dz <= scale {
            r
        } else {
            r + dz
        }
    }
}

pub(super) fn read_member(
    reader: &mut Shell<'_, '_>,
    solid: u32,
) -> Result<(Readback, ImportedWarped)> {
    let boundary = Boundary::parse(reader, solid)?;
    let imported = if boundary.faces.iter().any(|face| !face.holes.is_empty()) {
        bosses::reconstruct(reader, &boundary)?
    } else {
        resolve_chart(reader, &boundary, false)?.0
    };
    let readback = boundary.readback(reader, imported.shell.volume_mm3())?;
    Ok((readback, imported))
}

fn resolve_chart(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
    allow_flat: bool,
) -> Result<(ImportedWarped, Chart)> {
    let ellipse = boundary.curves.values().find_map(|(_, curve)| match curve {
        TrimCurve::Arc(arc) if arc.ellipse().semiaxes_mm()[0] > arc.ellipse().semiaxes_mm()[1] => {
            Some(arc.ellipse())
        }
        _ => None,
    });
    let origin = *reader.points.values().next().ok_or(ReadError::Invalid)?;
    let Some(ellipse) = ellipse else {
        if !allow_flat {
            return Err(ReadError::Unsupported);
        }
        let lower = reader
            .points
            .values()
            .map(|point| point[2] - origin[2])
            .fold(f64::INFINITY, f64::min);
        let chart = Chart {
            origin,
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            up: [0., 0., 1.],
            band: Band {
                center_z_mm: lower - 1.,
                radius_mm: 1.,
            },
        };
        return reconstruct(reader, boundary, chart).map(|shell| {
            (
                ImportedWarped {
                    shell,
                    origin_mm: origin,
                    u: chart.u,
                    v: chart.v,
                },
                chart,
            )
        });
    };
    let cylinder_axis = boundary
        .faces
        .iter()
        .find_map(|face| match face.surface {
            Surface::Cylinder { axis, .. } => Some(axis),
            Surface::Plane { .. } => None,
        })
        .ok_or(ReadError::Unsupported)?;
    let [major, minor] = ellipse.semiaxes_mm();
    for (direction, radius_mm) in [(ellipse.major_axis(), major), (ellipse.minor_axis(), minor)] {
        if dot(direction, cylinder_axis).abs() > 1e-10 {
            continue;
        }
        for sign in [-1., 1.] {
            let up = direction.map(|x| x * sign);
            let v = unit(cross(up, cylinder_axis))?;
            let u = cross(v, up);
            let band = Band {
                center_z_mm: dot(sub(ellipse.center_mm(), origin), up),
                radius_mm,
            };
            let chart = Chart {
                origin,
                u,
                v,
                up,
                band,
            };
            match reconstruct(reader, boundary, chart) {
                Ok(shell) => {
                    return Ok((
                        ImportedWarped {
                            shell,
                            origin_mm: origin,
                            u,
                            v,
                        },
                        chart,
                    ));
                }
                Err(ReadError::Limit) => return Err(ReadError::Limit),
                Err(_) => {}
            }
        }
    }
    Err(ReadError::Unsupported)
}

fn reconstruct(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
    chart: Chart,
) -> Result<warped_union::Shell> {
    let tol = reader.tolerance;
    let chart_points: BTreeMap<_, _> = reader
        .points
        .iter()
        .map(|(&id, &point)| {
            let [x, y, z] = chart.local(point);
            (id, [x, y, chart.height(z, tol)])
        })
        .collect();
    let rings: Vec<_> = boundary
        .faces
        .iter()
        .map(|face| {
            face.loop_
                .iter()
                .map(|&(edge, forward)| {
                    let index = boundary.curves[&edge].0[usize::from(!forward)];
                    chart_points[&index]
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let union = Union::verify_boundary(&rings, tol).map_err(|err| match err {
        planar_boolean::Error::Limit => ReadError::Limit,
        _ => ReadError::Invalid,
    })?;
    // The chart verifier already budgets the same per-face metric work.
    let shell = warped_union::remap(&union, chart.band, tol).map_err(|_| ReadError::Invalid)?;
    verify_geometry(reader, boundary, chart, &shell)?;
    Ok(shell)
}

fn verify_geometry(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
    chart: Chart,
    shell: &warped_union::Shell,
) -> Result<()> {
    let tol = reader.tolerance;
    // This comparison verifies each directed edge in EACH incident support
    // and also rejects split, duplicated, or disconnected geometric topology.
    if shell.edges().len() != boundary.curves.len() || shell.vertices().len() != reader.points.len()
    {
        return Err(ReadError::Invalid);
    }
    for (decoded, expected) in boundary.faces.iter().zip(shell.faces()) {
        if !support_matches(decoded.surface, expected, chart, tol) {
            return Err(ReadError::Invalid);
        }
        for (&(edge, forward), usage) in decoded.loop_.iter().zip(&expected.boundary) {
            let (vertices, decoded_curve) = boundary.curves[&edge];
            let model = shell.edges()[usage.edge];
            let start = vertices[usize::from(!forward)];
            let reconstructed_start = shell.vertices()[model.vertices[usize::from(!usage.forward)]];
            if length(sub(chart.local(reader.points[&start]), reconstructed_start)) > tol {
                return Err(ReadError::Invalid);
            }
            let expected_curve = model.curve;
            if !curve_matches(
                decoded_curve,
                forward,
                expected_curve,
                usage.forward,
                chart,
                tol,
            ) {
                return Err(ReadError::Invalid);
            }
        }
    }
    Ok(())
}

fn support_matches(
    decoded: Surface,
    expected: &warped_union::Face,
    chart: Chart,
    tolerance: f64,
) -> bool {
    match (decoded, expected.support) {
        (Surface::Plane { origin, normal }, warped_union::Support::Plane { normal: model }) => {
            length(sub(chart.vector(normal), model)) <= 1e-10
                && dot(sub(expected.point_mm, chart.local(origin)), model).abs() <= tolerance
        }
        (
            Surface::Cylinder {
                origin,
                axis,
                radius,
                outward,
            },
            warped_union::Support::Cylinder {
                axis_origin_mm,
                axis_direction,
                radius_mm,
                outward: model_outward,
            },
        ) => {
            length(cross(chart.vector(axis), axis_direction)) <= 1e-10
                && length(cross(
                    sub(chart.local(origin), axis_origin_mm),
                    axis_direction,
                )) <= tolerance
                && (radius - radius_mm).abs() <= tolerance
                && outward == model_outward
        }
        _ => false,
    }
}

fn curve_matches(
    decoded: TrimCurve,
    forward: bool,
    expected: warped_union::Curve,
    expected_forward: bool,
    chart: Chart,
    tolerance: f64,
) -> bool {
    match (decoded, expected) {
        (TrimCurve::Line, warped_union::Curve::Line) => true,
        (TrimCurve::Arc(input), warped_union::Curve::Ellipse(output)) => {
            let a = input.ellipse();
            let b = output.ellipse();
            let axes = a.semiaxes_mm();
            let equal_axes = axes
                .into_iter()
                .zip(b.semiaxes_mm())
                .all(|(a, b)| (a - b).abs() <= tolerance);
            if !equal_axes
                || length(sub(chart.local(a.center_mm()), b.center_mm())) > tolerance
                || length(cross(chart.vector(a.normal()), b.normal())) > 1e-10
                || ((axes[0] - axes[1]).abs() > tolerance
                    && length(cross(chart.vector(a.major_axis()), b.major_axis())) > 1e-10)
            {
                return false;
            }
            // A common ellipse has exactly two trims with the same endpoints.
            // Compare direction and sweep, including the opposite traversal.
            let signed_sweep =
                |arc: EllipseArc, forwards: bool| arc.sweep_rad() * if forwards { 1. } else { -1. };
            let input_sweep =
                signed_sweep(input, forward) * dot(chart.vector(a.normal()), b.normal()).signum();
            let output_sweep = signed_sweep(output, expected_forward);
            (input_sweep - output_sweep).abs() * axes[0] <= tolerance
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
