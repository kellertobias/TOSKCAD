//! Clip exact shell edges and blockers to a section's retained depth.
use super::*;
use multicad_rust_kernel::{
    planar_projection::{
        self, AnalyticFace, DepthRange, FaceTrim, LineCandidate, SpatialRimCandidate,
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
            blockers: Vec::new(),
            output: Vec::new(),
            hatch_budget: section_hatch::Budget::default(),
        };
        let far = options.section_depth_mm.map(|distance| near - distance);
        let depth = DepthRange::new(near, far).map_err(planar::projection_error)?;
        let mut tolerance = GeometryPolicy::default().linear_tolerance_mm;
        for (source, shell, placement, policy) in members {
            let frame = Frame::new(placement, options.frame);
            if !in_window(shell, &frame, options, near, far)? {
                continue;
            }
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
            let cut = frame.intersection(shell, near, scene.options.maximum_curves, policy)?;
            scene.near_cut(&frame, &cut, near, source, policy, solids[source].hatch)?;
            if let Some(far) = far {
                let cut = frame.intersection(shell, far, scene.options.maximum_curves, policy)?;
                scene.far_cut(&frame, &cut, far, source)?;
            }
        }
        scene.finish(tolerance)
    })())
}

struct Retained {
    options: DrawingProjection,
    lines: Vec<LineCandidate>,
    rims: Vec<SpatialRimCandidate>,
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

    fn far_cut(
        &mut self,
        frame: &Frame,
        loops: &[Vec<SectionEdge>],
        depth: f64,
        source: usize,
    ) -> Result<(), GeometryError> {
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
                Curve::Ellipse(ellipse) if ellipse.major == ellipse.minor => {
                    self.rims.push(SpatialRimCandidate {
                        source,
                        ellipse,
                        depth,
                        depth_sine: 0.,
                    })
                }
                Curve::Ellipse(_) => return Err(non_circular()),
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
        if !self.lines.is_empty() || !self.rims.is_empty() {
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
            if major != minor {
                return Err(non_circular());
            }
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

fn non_circular() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust retained shell sections require line or circular cap edges",
    )
}

fn in_window(
    shell: &Shell,
    frame: &Frame,
    options: DrawingProjection,
    near: f64,
    far: Option<f64>,
) -> Result<bool, GeometryError> {
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
        return Ok(false);
    }
    let windows = [options.section_u_mm, options.section_v_mm];
    if windows
        .into_iter()
        .zip(bounds)
        .any(|(window, [lo, hi])| window.is_some_and(|[a, b]| hi <= a || lo >= b))
    {
        return Ok(false);
    }
    if windows
        .into_iter()
        .zip(bounds)
        .any(|(window, [lo, hi])| window.is_some_and(|[a, b]| lo < a || hi > b))
    {
        return Err(err(
            GeometryErrorKind::Unsupported,
            "Rust retained shell sections cannot yet clip a lateral window",
        ));
    }
    Ok(true)
}
