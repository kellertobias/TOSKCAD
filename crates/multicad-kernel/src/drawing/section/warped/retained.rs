//! Clip exact shell edges and blockers to a section's retained depth.
use super::*;
use multicad_rust_kernel::{
    planar_projection::{
        self, AnalyticFace, DepthRange, FaceTrim, LineCandidate, SpatialEllipseCandidate,
        SpatialRimCandidate,
    },
    section_hatch,
    trim_curve::{Ellipse as SpatialEllipse, EllipseArc},
};

pub(in crate::drawing::section) fn project(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
    near: f64,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let members = projection_members(solids)
        .into_iter()
        .map(|(source, solid)| {
            let (shell, placement) = solid.drawing_warped_shell()?;
            Some((source, shell, placement, solid.geometry_policy()))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((|| {
        let mut scene = Retained {
            options,
            lines: Vec::new(),
            rims: Vec::new(),
            ellipses: Vec::new(),
            blockers: Vec::new(),
            output: Vec::new(),
            hatch_budget: section_hatch::Budget::default(),
        };
        let far = options.section_depth_mm.map(|distance| near - distance);
        let windows = [options.section_u_mm, options.section_v_mm];
        let depth = DepthRange::new(near, far)
            .and_then(|depth| depth.with_lateral(windows))
            .map_err(planar::projection_error)?;
        let mut tolerance = GeometryPolicy::default().linear_tolerance_mm;
        for (source, shell, placement, policy) in members {
            let frame = Frame::new(placement, options.frame);
            let Some(sides) = window_sides(shell, &frame, windows, near, far) else {
                continue;
            };
            tolerance = tolerance.max(policy.linear_tolerance_mm);
            let elevation = curved::warped::elevation(
                shell,
                placement,
                source,
                policy.linear_tolerance_mm,
                options,
            )?;
            for line in elevation.lines {
                scene
                    .lines
                    .extend(depth.clip_line(line).map_err(planar::projection_error)?);
            }
            for rim in elevation.rims {
                scene
                    .rims
                    .extend(depth.clip_rim(rim).map_err(planar::projection_error)?);
            }
            scene.blockers.extend(
                elevation
                    .blockers
                    .into_iter()
                    .filter_map(|face| face.retained(depth)),
            );
            let cut = frame.clip(
                frame.intersection(shell, near, scene.options.maximum_curves, policy)?,
                options,
                policy,
            )?;
            scene.near_cut(&frame, &cut, near, source, policy, solids[source].hatch)?;
            if let Some(far) = far {
                let cut = frame.clip(
                    frame.intersection(shell, far, scene.options.maximum_curves, policy)?,
                    options,
                    policy,
                )?;
                scene.far_cut(&frame, &cut, far, source);
            }
            for (axis, coordinates) in sides.into_iter().enumerate() {
                for coordinate in coordinates.into_iter().flatten() {
                    let cut = frame.section_at(
                        shell,
                        axis,
                        coordinate,
                        options.maximum_curves,
                        policy,
                    )?;
                    scene.side_cut(&frame, cut, axis, coordinate, depth, source)?;
                }
            }
        }
        scene.finish(tolerance)
    })())
}

struct Retained {
    options: DrawingProjection,
    lines: Vec<LineCandidate>,
    rims: Vec<SpatialRimCandidate>,
    ellipses: Vec<SpatialEllipseCandidate>,
    blockers: Vec<AnalyticFace>,
    output: Vec<ProjectedVector>,
    hatch_budget: section_hatch::Budget,
}

impl Retained {
    fn near_cut(
        &mut self,
        frame: &Frame,
        loops: &[Vec<SectionEdge>],
        depth: f64,
        source: usize,
        policy: GeometryPolicy,
        hatch: Option<DrawingHatch>,
    ) -> Result<(), GeometryError> {
        if loops.is_empty() {
            return Ok(());
        }
        let rings = loops
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|edge| section_trim(frame, *edge, depth))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let face = AnalyticFace::plane(
            [0., 0., depth],
            [0., 0., 1.],
            rings,
            policy.linear_tolerance_mm,
        )
        .map_err(planar::projection_error)?;
        self.blockers.extend(face);
        let vectors = frame.vectors(loops, self.options, source)?;
        self.extend(vectors)?;
        if let Some(hatch) = hatch.filter(|h| h.pattern != HatchPattern::None) {
            let boundary = frame.hatch(
                loops,
                self.options
                    .chord_tolerance_mm
                    .min(policy.minimum_feature_mm / 4.),
            )?;
            let mut curves = self.options.maximum_curves as usize;
            let mut points = self.options.maximum_points as usize;
            let hatch = publish_hatch(
                source,
                hatch,
                &boundary,
                policy.linear_tolerance_mm,
                &mut curves,
                &mut points,
                &mut self.hatch_budget,
            )?;
            self.extend(hatch)?;
        }
        Ok(())
    }

    fn far_cut(&mut self, frame: &Frame, loops: &[Vec<SectionEdge>], depth: f64, source: usize) {
        for curve in loops
            .iter()
            .flatten()
            .flat_map(|&edge| frame.curves(edge).into_iter().flatten())
        {
            match curve {
                Curve::Line(points) => self.lines.push(LineCandidate {
                    source,
                    points: points.map(|[x, y]| [x, y, depth]),
                }),
                Curve::Ellipse(ellipse) => self.ellipses.push(SpatialEllipseCandidate {
                    source,
                    ellipse,
                    depth,
                    depth_cosine: 0.,
                    depth_sine: 0.,
                }),
            }
        }
    }

    fn side_cut(
        &mut self,
        frame: &Frame,
        loops: Vec<Vec<SectionEdge>>,
        axis: usize,
        coordinate: f64,
        depth: DepthRange,
        source: usize,
    ) -> Result<(), GeometryError> {
        let other = 1 - axis;
        let across = frame.directions.map(|direction| direction[other]);
        let depth_axis = frame.directions.map(|direction| direction[2]);
        let screen_point = |p| {
            let [x, y] = frame.point(p);
            let mut point = [x, y, frame.origin[2] + dot(depth_axis, p)];
            point[axis] = coordinate;
            point
        };
        // The cut face is edge-on. Its near and far connectors are already
        // part of those caps, and connectors to another side project to points.
        for edge in loops.into_iter().flatten() {
            match edge.curve {
                Trim::Line => self.lines.extend(
                    depth
                        .clip_line(LineCandidate {
                            source,
                            points: edge.ends_mm.map(screen_point),
                        })
                        .map_err(planar::projection_error)?,
                ),
                Trim::Ellipse(arc) => {
                    let ellipse = arc.ellipse();
                    let [major, minor] = ellipse.semiaxes_mm();
                    let [u, v] = [
                        ellipse.major_axis().map(|component| component * major),
                        ellipse.minor_axis().map(|component| component * minor),
                    ];
                    let [a, b] = [dot(across, u), dot(across, v)];
                    let phase = b.atan2(a);
                    let (sin, cos) = phase.sin_cos();
                    let [du, dv] = [dot(depth_axis, u), dot(depth_axis, v)];
                    // Align the projected coordinate with cosine. The depth of
                    // an oblique ellipse retains both terms after this rotation.
                    let depth_cosine = du.mul_add(cos, dv * sin);
                    let depth_sine = (-du).mul_add(sin, dv * cos);
                    let [x, y, z] = screen_point(ellipse.center_mm());
                    let sweep = arc.sweep_rad();
                    let lo = (arc.start_rad() + sweep.min(0.) - phase).rem_euclid(TAU);
                    let hi = lo + sweep.abs();
                    for trim in [[lo, hi.min(TAU)], [0., hi - TAU]] {
                        if trim[1] > trim[0] {
                            self.ellipses.extend(
                                depth
                                    .clip_ellipse(SpatialEllipseCandidate {
                                        source,
                                        ellipse: Ellipse {
                                            center: [x, y],
                                            u: std::array::from_fn(|i| f64::from(i == other)),
                                            v: std::array::from_fn(|i| f64::from(i == axis)),
                                            major: a.hypot(b),
                                            minor: 0.,
                                            trim,
                                        },
                                        depth: z,
                                        depth_cosine,
                                        depth_sine,
                                    })
                                    .map_err(planar::projection_error)?,
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn extend(&mut self, vectors: Vec<ProjectedVector>) -> Result<(), GeometryError> {
        let points = vectors
            .iter()
            .map(|vector| vector.preview_mm.len() as u32)
            .sum();
        self.options.maximum_points = self
            .options
            .maximum_points
            .checked_sub(points)
            .ok_or_else(limit)?;
        self.options.maximum_curves = self
            .options
            .maximum_curves
            .checked_sub(vectors.len() as u32)
            .ok_or_else(limit)?;
        self.output.extend(vectors);
        Ok(())
    }

    fn finish(mut self, tolerance: f64) -> Result<Vec<ProjectedVector>, GeometryError> {
        if !self.lines.is_empty() || !self.rims.is_empty() || !self.ellipses.is_empty() {
            let segment_limit = (self.options.maximum_curves as usize)
                .min(self.options.maximum_points as usize / 2);
            if segment_limit == 0 {
                return Err(limit());
            }
            let result = planar_projection::project_scene_with_analytic_faces(
                &[],
                &[],
                planar_projection::SceneExtras {
                    lines: &self.lines,
                    spatial_rims: &self.rims,
                    spatial_ellipses: &self.ellipses,
                    ..Default::default()
                },
                &self.blockers,
                planar_projection::Options {
                    tolerance,
                    hidden_lines: self.options.hidden_lines,
                    tangent_edges: self.options.tangent_edges,
                    detail: None,
                    segment_limit,
                },
            )
            .map_err(planar::projection_error)?;
            self.output
                .extend(curved::publish_scene(result, self.options, &[], &[])?);
        }
        self.output.sort_by_key(|vector| vector.source_index);
        Ok(self.output)
    }
}

fn section_trim(frame: &Frame, edge: SectionEdge, depth: f64) -> Result<FaceTrim, GeometryError> {
    match edge.curve {
        Trim::Line => Ok(FaceTrim::Line(edge.ends_mm.map(|point| {
            let [x, y] = frame.point(point);
            [x, y, depth]
        }))),
        Trim::Ellipse(arc) => {
            let ellipse = arc.ellipse();
            let [major, minor] = ellipse.semiaxes_mm();
            let [x, y] = frame.point(ellipse.center_mm());
            let [u, v] =
                [ellipse.major_axis(), ellipse.minor_axis()].map(|axis| frame.vector(axis));
            let sign = (u[0] * v[1] - u[1] * v[0]).signum();
            let ellipse = SpatialEllipse::new(
                [x, y, depth],
                [0., 0., sign],
                [u[0], u[1], 0.],
                [major, minor],
            )
            .map_err(|_| invalid_cap())?;
            let arc = EllipseArc::new(ellipse, arc.start_rad(), arc.sweep_rad())
                .map_err(|_| invalid_cap())?;
            Ok(FaceTrim::Arc(arc))
        }
    }
}

fn invalid_cap() -> GeometryError {
    err(
        GeometryErrorKind::InvalidResult,
        "Invalid retained section cap",
    )
}

fn window_sides(
    shell: &Shell,
    frame: &Frame,
    windows: [Option<[f64; 2]>; 2],
    near: f64,
    far: Option<f64>,
) -> Option<[[Option<f64>; 2]; 2]> {
    let [min, max] = shell.bounds_mm();
    let bounds: [[f64; 2]; 3] = std::array::from_fn(|axis| {
        [false, true].map(|upper| {
            let point = std::array::from_fn(|i| {
                if upper == (frame.directions[i][axis] >= 0.) {
                    max[i]
                } else {
                    min[i]
                }
            });
            frame.origin[axis] + dot(point, frame.directions.map(|direction| direction[axis]))
        })
    });
    if near <= bounds[2][0] || far.is_some_and(|far| far >= bounds[2][1]) {
        return None;
    }
    if windows
        .into_iter()
        .zip(bounds)
        .any(|(window, [lo, hi])| window.is_some_and(|[a, b]| hi <= a || lo >= b))
    {
        return None;
    }
    Some(std::array::from_fn(|axis| {
        windows[axis].map_or([None; 2], |[a, b]| {
            let [lo, hi] = bounds[axis];
            [(lo < a).then_some(a), (hi > b).then_some(b)]
        })
    }))
}
