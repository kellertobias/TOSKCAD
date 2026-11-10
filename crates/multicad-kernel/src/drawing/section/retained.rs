//! Adapt exact extruded material bands to retained section visibility.
use super::*;
use multicad_rust_kernel::{
    orthogonal::Box3,
    retained_section::{self, AxialProfile, Body, Options, Profile},
    stadium::Stadium,
};

use std::cell::Cell;

type Point = [f64; 3];
const AXIS_TOLERANCE: f64 = 32. * f64::EPSILON;
const OCCLUSION_BUDGET: usize = 2_000_000;

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn unsupported() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust retained sections require aligned rectilinear stock and compatible extruded curved profiles",
    )
}

pub(super) fn project(
    solids: &[DrawingSolid<'_>],
    mut options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let tolerance = solids
        .iter()
        .map(|input| input.solid.geometry_policy().linear_tolerance_mm)
        .fold(GeometryPolicy::default().linear_tolerance_mm, f64::max);
    let scene = adapt_scene(solids, options, depth, tolerance)?;
    let projection = Options {
        depth_mm: depth,
        far_mm: options.section_depth_mm.map(|range| depth - range),
        windows_mm: [options.section_u_mm, options.section_v_mm],
        tolerance_mm: tolerance,
        segment_limit: (options.maximum_curves as usize).min(options.maximum_points as usize / 2),
    };
    let visible = if scene.along_v.is_empty() {
        retained_section::project(&scene.along_u, projection)
    } else {
        retained_section::project_crossed(&scene.along_u, &scene.along_v, projection)
    }
    .map_err(planar::projection_error)?;
    options.maximum_curves -= visible.len() as u32;
    options.maximum_points -= 2 * visible.len() as u32;
    options.section_only = true;
    options.section_depth_mm = None;
    let mut output: Vec<_> = visible
        .into_iter()
        .map(|segment| planar::line(segment.source, VectorRole::Visible, segment.ends))
        .collect();
    output.extend(machined::project_members(
        scene.cut_members,
        options,
        depth,
    )?);
    Ok(output)
}

struct Scene<'a> {
    along_u: Vec<Body>,
    along_v: Vec<Body>,
    cut_members: Vec<(usize, DrawingSolid<'a>)>,
}

fn adapt_scene<'a>(
    solids: &[DrawingSolid<'a>],
    options: DrawingProjection,
    depth: f64,
    tolerance: f64,
) -> Result<Scene<'a>, GeometryError> {
    let visible_region = Region {
        windows: [
            options.section_u_mm,
            options.section_v_mm,
            Some([
                options
                    .section_depth_mm
                    .map_or(f64::NEG_INFINITY, |range| depth - range),
                depth,
            ]),
        ],
        tolerance,
    };
    let members = projection_members(solids);
    let blockers = opaque_cells(&members, options.frame, visible_region)?;
    let remaining = Cell::new(OCCLUSION_BUDGET);
    let mut along_u = Vec::new();
    let mut along_v = Vec::new();
    let blockers_v: Vec<_> = blockers
        .iter()
        .map(|&(owner, cell)| (owner, transpose_box(cell)))
        .collect();
    let mut cut_members = Vec::new();
    let world = Alignment::new(PlaneFrame::default(), options.frame, false);
    for (member, &(source, solid)) in members.iter().enumerate() {
        let bounds = solid.bounds()?;
        let projected = world.bounds(Box3 {
            min: bounds.min,
            max: bounds.max,
        });
        if !visible_region.intersects(projected) {
            continue;
        }
        let scope = Scope {
            region: visible_region,
            blockers: &blockers,
            member,
            near: depth,
            remaining: &remaining,
        };
        match adapt(solid, source, options.frame, false, scope) {
            Ok(body) => along_u.push(body),
            Err(error) if error.kind == GeometryErrorKind::Unsupported => {
                let [u, v, z] = visible_region.windows;
                let scope = Scope {
                    region: Region {
                        windows: [v, u, z],
                        tolerance,
                    },
                    blockers: &blockers_v,
                    member,
                    near: depth,
                    remaining: &remaining,
                };
                along_v.push(adapt(solid, source, options.frame, true, scope)?);
            }
            Err(error) => return Err(error),
        }
        if projected.min[2] < depth - tolerance && projected.max[2] > depth + tolerance {
            cut_members.push((
                source,
                DrawingSolid {
                    solid,
                    hatch: solids[source].hatch,
                },
            ));
        }
    }
    Ok(Scene {
        along_u,
        along_v,
        cut_members,
    })
}

#[derive(Clone, Copy)]
struct Region {
    windows: [Option<[f64; 2]>; 3],
    tolerance: f64,
}

impl Region {
    fn intersects(self, bounds: Box3) -> bool {
        self.windows.iter().enumerate().all(|(i, window)| {
            window.is_none_or(|[lo, hi]| {
                bounds.max[i] > lo + self.tolerance && bounds.min[i] < hi - self.tolerance
            })
        })
    }

    fn clip(self, mut bounds: Box3) -> Box3 {
        for (i, window) in self.windows.into_iter().enumerate() {
            if let Some([lo, hi]) = window {
                bounds.min[i] = bounds.min[i].max(lo);
                bounds.max[i] = bounds.max[i].min(hi);
            }
        }
        bounds
    }
}

struct Scope<'a> {
    region: Region,
    blockers: &'a [(usize, Box3)],
    member: usize,
    near: f64,
    remaining: &'a Cell<usize>,
}

impl Scope<'_> {
    fn completely_hidden(&self, bounds: Box3) -> Result<bool, GeometryError> {
        let tolerance = self.region.tolerance;
        let bounds = self.region.clip(bounds);
        // The front section must still see the exact cut geometry if the
        // opening reaches it. Only wholly rear features may be suppressed.
        if bounds.max[2] >= self.near - tolerance {
            return Ok(false);
        }
        for (owner, blocker) in self.blockers {
            let remaining = self
                .remaining
                .get()
                .checked_sub(1)
                .ok_or_else(section_limit)?;
            self.remaining.set(remaining);
            if *owner != self.member
                && blocker.max[2] > bounds.max[2] + tolerance
                && (0..2)
                    .all(|i| blocker.min[i] <= bounds.min[i] && blocker.max[i] >= bounds.max[i])
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn section_limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust retained section exceeded its occlusion budget",
    )
}

fn transpose_box(Box3 { min, max }: Box3) -> Box3 {
    let transpose = |[u, v, depth]: Point| [v, u, depth];
    Box3 {
        min: transpose(min),
        max: transpose(max),
    }
}

fn opaque_cells(
    members: &[(usize, &Solid)],
    view: PlaneFrame,
    region: Region,
) -> Result<Vec<(usize, Box3)>, GeometryError> {
    let mut blockers = Vec::new();
    for (member, (_, solid)) in members.iter().enumerate() {
        let (cells, frame) = if let Some((stock, frame)) = solid.orthogonal_body() {
            if stock.has_curved_cuts() {
                continue;
            }
            (
                std::borrow::Cow::Borrowed(stock.drawing_stock().cells),
                frame,
            )
        } else if let Some((cells, frame)) = solid.drawing_rectilinear_cells()? {
            (std::borrow::Cow::Owned(cells), frame)
        } else {
            continue;
        };
        let alignment = Alignment::new(frame, view, false);
        if alignment.rectilinear() {
            if cells.len() > 100_000usize.saturating_sub(blockers.len()) {
                return Err(section_limit());
            }
            blockers.extend(
                cells
                    .iter()
                    .map(|&cell| alignment.bounds(cell))
                    .filter(|&cell| region.intersects(cell))
                    .map(|cell| (member, region.clip(cell))),
            );
        }
    }
    Ok(blockers)
}

#[derive(Clone, Copy)]
struct Alignment {
    origin: Point,
    /// Each column is a local axis in drawing (u, v, depth) coordinates.
    axes: [Point; 3],
}

impl Alignment {
    fn new(local: PlaneFrame, view: PlaneFrame, transposed: bool) -> Self {
        let mut screen = planar::view_axes(view);
        if transposed {
            screen.swap(0, 1);
        }
        let into_view = |vector| screen.map(|axis| dot(axis, vector));
        Self {
            origin: into_view(subtract(local.origin, view.origin)),
            axes: planar::view_axes(local).map(into_view),
        }
    }

    fn point(self, point: Point) -> Point {
        std::array::from_fn(|i| {
            point
                .into_iter()
                .zip(self.axes)
                .fold(self.origin[i], |sum, (p, axis)| p.mul_add(axis[i], sum))
        })
    }

    fn bounds(self, bounds: Box3) -> Box3 {
        let [min, max] = [false, true].map(|upper| {
            std::array::from_fn(|i| {
                self.axes
                    .iter()
                    .enumerate()
                    .fold(self.origin[i], |sum, (j, axis)| {
                        let coordinate = if upper == (axis[i] >= 0.) {
                            bounds.max[j]
                        } else {
                            bounds.min[j]
                        };
                        coordinate.mul_add(axis[i], sum)
                    })
            })
        });
        Box3 { min, max }
    }

    fn rectilinear(self) -> bool {
        self.axes
            .iter()
            .all(|axis| axis.iter().filter(|v| v.abs() > AXIS_TOLERANCE).count() == 1)
    }

    fn along_u(self, local_axis: usize) -> bool {
        self.axes[local_axis][1].hypot(self.axes[local_axis][2]) <= AXIS_TOLERANCE
    }

    fn axial_span(self, axis: usize, values: [f64; 2]) -> [f64; 2] {
        let [a, b] = values.map(|x| self.axes[axis][0].mul_add(x, self.origin[0]));
        [a.min(b), a.max(b)]
    }

    fn profile(self, point: Point) -> [f64; 2] {
        let [_, v, depth] = self.point(point);
        [v, depth]
    }

    fn stadium(self, centers: [Point; 2], radius: f64) -> Result<Profile, GeometryError> {
        Stadium::new(centers.map(|point| self.profile(point)), radius)
            .map(Profile::Stadium)
            .map_err(|_| unsupported())
    }
}

fn adapt(
    solid: &Solid,
    source: usize,
    view: PlaneFrame,
    transposed: bool,
    scope: Scope<'_>,
) -> Result<Body, GeometryError> {
    let mut result = Body {
        source,
        stock: Vec::new(),
        cuts: Vec::new(),
        positive: Vec::new(),
    };
    if let Some((stock, frame)) = solid.orthogonal_body() {
        let alignment = Alignment::new(frame, view, transposed);
        if !alignment.rectilinear() {
            return Err(unsupported());
        }
        let stock = stock.drawing_stock();
        result
            .stock
            .extend(stock.cells.iter().map(|&cell| alignment.bounds(cell)));
        for pocket in stock.pockets {
            let face = &stock.faces[pocket.face];
            let Some(span) = cavity(
                alignment,
                &scope,
                face.normal,
                [pocket.center_mm; 2],
                pocket.radius_mm,
                pocket.depth_mm,
            )?
            else {
                continue;
            };
            result.cuts.push(AxialProfile {
                span,
                profile: Profile::Circle {
                    center: alignment.profile(pocket.center_mm),
                    radius: pocket.radius_mm,
                },
            });
        }
        for slot in stock.slots {
            let face = &stock.faces[slot.face];
            let Some(span) = cavity(
                alignment,
                &scope,
                face.normal,
                slot.centers_mm,
                slot.radius_mm,
                slot.depth_mm,
            )?
            else {
                continue;
            };
            result.cuts.push(AxialProfile {
                span,
                profile: alignment.stadium(slot.centers_mm, slot.radius_mm)?,
            });
        }
        return Ok(result);
    }
    if let Some(placed) = solid
        .polygon_prism()
        .filter(|placed| !placed.pockets.is_empty())
    {
        let alignment = Alignment::new(placed.frame, view, transposed);
        if !alignment.rectilinear() {
            return Err(unsupported());
        }
        let cells =
            multicad_rust_kernel::orthogonal::prism_cells(&placed.prism, placed.reverse_sweep)
                .map_err(|_| unsupported())?
                .ok_or_else(unsupported)?;
        result
            .stock
            .extend(cells.into_iter().map(|cell| alignment.bounds(cell)));
        let sign = if placed.reverse_sweep { -1. } else { 1. };
        for pocket in placed.pockets.iter() {
            let [x, y] = pocket.center_mm;
            let center = [
                x,
                y,
                sign * pocket.mouth_z_mm(placed.prism.height().as_f64()),
            ];
            let normal = [0., 0., if pocket.end { sign } else { -sign }];
            if let Some(span) = cavity(
                alignment,
                &scope,
                normal,
                [center; 2],
                pocket.radius_mm,
                pocket.depth_mm,
            )? {
                result.cuts.push(AxialProfile {
                    span,
                    profile: Profile::Circle {
                        center: alignment.profile(center),
                        radius: pocket.radius_mm,
                    },
                });
            }
        }
        return Ok(result);
    }
    if let Some((cells, frame)) = solid.drawing_rectilinear_cells()? {
        let alignment = Alignment::new(frame, view, transposed);
        if !alignment.rectilinear() {
            return Err(unsupported());
        }
        result
            .stock
            .extend(cells.into_iter().map(|cell| alignment.bounds(cell)));
        return Ok(result);
    }
    if let Some((body, frame)) = solid.rounded_slot_body() {
        let alignment = Alignment::new(frame, view, transposed);
        if !alignment.along_u(2) {
            return Err(unsupported());
        }
        let (stadium, axial) = body.section_profile();
        result.positive.push(AxialProfile {
            span: alignment.axial_span(2, axial),
            profile: alignment.stadium(
                stadium.centers().map(|[u, v]| [u, v, 0.]),
                stadium.radius_mm(),
            )?,
        });
        return Ok(result);
    }
    if let Some((frame, radius, height, bore)) = curved::circular_dimensions(solid) {
        let alignment = Alignment::new(frame, view, transposed);
        if !alignment.along_u(2) {
            return Err(unsupported());
        }
        let span = alignment.axial_span(2, [0., height]);
        let center = alignment.profile([0.; 3]);
        result.positive.push(AxialProfile {
            span,
            profile: Profile::Circle { center, radius },
        });
        if let Some(radius) = bore {
            result.cuts.push(AxialProfile {
                span,
                profile: Profile::Circle { center, radius },
            });
        }
        return Ok(result);
    }
    Err(unsupported())
}

fn cavity(
    alignment: Alignment,
    scope: &Scope<'_>,
    normal: Point,
    centers: [Point; 2],
    radius: f64,
    depth: f64,
) -> Result<Option<[f64; 2]>, GeometryError> {
    let axis = normal
        .iter()
        .position(|v| v.abs() == 1.)
        .ok_or_else(unsupported)?;
    let opening = centers[0][axis];
    let bottom = opening - normal[axis] * depth;
    let bounds = Box3 {
        min: std::array::from_fn(|i| {
            if i == axis {
                opening.min(bottom)
            } else {
                centers[0][i].min(centers[1][i]) - radius
            }
        }),
        max: std::array::from_fn(|i| {
            if i == axis {
                opening.max(bottom)
            } else {
                centers[0][i].max(centers[1][i]) + radius
            }
        }),
    };
    let bounds = alignment.bounds(bounds);
    if !scope.region.intersects(bounds) {
        return Ok(None);
    }
    if !alignment.along_u(axis) {
        if scope.completely_hidden(bounds)? {
            return Ok(None);
        }
        return Err(unsupported());
    }
    Ok(Some(alignment.axial_span(axis, [opening, bottom])))
}
