//! Axial visibility of the rotated transverse profile of a continuous chevron.
use super::*;
use multicad_rust_kernel::spur_gear::{ToothEdge, ToothRole};
use std::f64::consts::PI;

pub(in crate::drawing) fn project_herringbone(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (body, placement) = input.solid.herringbone_body()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = placement.normal();
    if dot(axis, u).hypot(dot(axis, v)) > 64. * f64::EPSILON
        || options.detail.is_some()
        || options.section_u_mm.is_some()
        || options.section_v_mm.is_some()
    {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust herringbone projection requires an uncropped axial view",
        )));
    }
    let policy = input.solid.geometry_policy();
    let alignment = dot(axis, toward_viewer);
    let offset = dot(
        std::array::from_fn(|i| placement.origin[i] - options.frame.origin[i]),
        toward_viewer,
    );
    let height = body.profile().height_mm();
    let ends = [offset, height.mul_add(alignment, offset)];
    let near = options.section_offset_mm.unwrap_or(f64::INFINITY);
    let far = options
        .section_depth_mm
        .map_or(f64::NEG_INFINITY, |distance| near - distance);
    let top = near.min(ends[0].max(ends[1]));
    let bottom = far.max(ends[0].min(ends[1]));
    if top - bottom <= policy.linear_tolerance_mm {
        return Some(Ok(Vec::new()));
    }
    let [near_z, far_z] =
        [top, bottom].map(|depth| ((depth - offset) / alignment).clamp(0., height));
    let middle = height / 2.;
    let near_phase = body.phase_at(near_z);
    let far_phase = body.phase_at(far_z);
    let seam = (near_z.min(far_z)..near_z.max(far_z)).contains(&middle)
        && near_z != middle
        && far_z != middle;
    let seam_phase = body.phase_at(middle);
    let mut sweep = [near_phase.min(far_phase), near_phase.max(far_phase)];
    if seam {
        sweep = [sweep[0].min(seam_phase), sweep[1].max(seam_phase)];
    }
    let cut = near_z > policy.linear_tolerance_mm && near_z < height - policy.linear_tolerance_mm;
    let cap = DrawingSolid {
        solid: input.solid,
        hatch: input.hatch.filter(|_| cut),
    };
    Some((|| {
        let vectors = section_herringbone(&cap, options, top).expect("herringbone cap")?;
        let mut output = Output { vectors, options };
        if !cut {
            for edge in &mut output.vectors {
                edge.role = VectorRole::Visible;
            }
        }
        output.consume_existing()?;
        let profile = body.profile();
        let teeth = profile.outline().last().expect("gear teeth").tooth + 1;
        let pitch = TAU / f64::from(teeth);
        let tip = profile
            .outline()
            .iter()
            .find(|edge| edge.role == ToothRole::Tip)
            .expect("gear tip")
            .edge;
        let [x, y] = tip.point(0.);
        let half = y.atan2(x).abs();
        let screen = Screen {
            origin: options.frame.origin,
            axes: [u, v],
            toward_viewer,
        };
        output.tip_silhouette(
            &screen,
            body.section(placement, near_z, 1.),
            (x.hypot(y), half),
            (pitch, teeth),
            sweep.map(|phase| phase - near_phase),
        )?;
        let mut visited = [near_phase; 2];
        let mut drawn = vec![near_phase];
        for (depth, phase) in seam
            .then_some((middle, seam_phase))
            .into_iter()
            .chain([(far_z, far_phase)])
        {
            // The two end faces of an uncut chevron have the same outline.
            // A full-tooth rotation also repeats a previously drawn outline.
            if drawn.iter().any(|&previous| {
                let repeated = (phase - previous).rem_euclid(pitch);
                repeated.min(pitch - repeated) <= 64. * f64::EPSILON
            }) {
                visited = [visited[0].min(phase), visited[1].max(phase)];
                continue;
            }
            drawn.push(phase);
            let front_span = if phase < visited[0] {
                Some((false, visited[1] - phase))
            } else if phase > visited[1] {
                Some((true, phase - visited[0]))
            } else {
                None
            };
            visited = [visited[0].min(phase), visited[1].max(phase)];
            let frame = body.section(placement, depth, 1.);
            let project = |[x, y]: Point| {
                screen.point(std::array::from_fn(|i| {
                    frame.u[i].mul_add(x, frame.v[i].mul_add(y, frame.origin_mm[i]))
                }))
            };
            for edge in profile.outline() {
                output.rim_flank(*edge, pitch, front_span, &project)?;
            }
        }
        Ok(output.vectors)
    })())
}

struct Output {
    vectors: Vec<ProjectedVector>,
    options: DrawingProjection,
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust herringbone drawing exceeded its curve or point budget",
    )
}

impl Output {
    fn consume_existing(&mut self) -> Result<(), GeometryError> {
        self.options.maximum_curves = self
            .options
            .maximum_curves
            .checked_sub(self.vectors.len() as u32)
            .ok_or_else(limit)?;
        self.options.maximum_points = self
            .options
            .maximum_points
            .checked_sub(self.vectors.iter().map(|v| v.preview_mm.len() as u32).sum())
            .ok_or_else(limit)?;
        Ok(())
    }

    fn tip_silhouette(
        &mut self,
        screen: &Screen,
        frame: multicad_core::references::ReferenceFrame,
        (radius, half): (f64, f64),
        (pitch, teeth): (f64, u32),
        [lo, hi]: [f64; 2],
    ) -> Result<(), GeometryError> {
        // Fill only the two parts of the nearest cap's inter-tooth gap
        // that are reached by the continuously rotating tip.
        let gap = [half, pitch - half];
        let leading = (half + hi).clamp(gap[0], gap[1]);
        let trailing = (pitch - half + lo).clamp(gap[0], gap[1]);
        let spans = if leading >= trailing {
            [gap, [0.; 2]]
        } else {
            [[gap[0], leading], [trailing, gap[1]]]
        };
        let mut edges = Vec::new();
        for tooth in 0..teeth {
            for [a, b] in spans {
                if b - a <= 64. * f64::EPSILON {
                    continue;
                }
                edges.push(Edge {
                    hidden: false,
                    curve: screen.curve(Some(AnalyticGeometry::CircularArc {
                        frame,
                        radius_mm: radius,
                        start_rad: f64::from(tooth).mul_add(pitch, a),
                        sweep_rad: b - a,
                    }))?,
                });
            }
        }
        let published = curved::publish(edges, self.options, 0)?;
        self.options.maximum_curves -= published.len() as u32;
        self.options.maximum_points -= published
            .iter()
            .map(|v| v.preview_mm.len() as u32)
            .sum::<u32>();
        self.vectors.extend(published);
        Ok(())
    }

    fn rim_flank(
        &mut self,
        tooth: ToothEdge,
        pitch: f64,
        front_span: Option<(bool, f64)>,
        project: &impl Fn(Point) -> Point,
    ) -> Result<(), GeometryError> {
        let right = match tooth.role {
            ToothRole::LeftFillet | ToothRole::LeftRadial | ToothRole::LeftFlank => false,
            ToothRole::RightFillet | ToothRole::RightRadial | ToothRole::RightFlank => true,
            ToothRole::Root | ToothRole::Tip => return Ok(()),
        };
        let visible = front_span
            .filter(|&(positive, _)| positive == right)
            .and_then(|(_, span)| flank_visibility(tooth, pitch, (pitch - span) / 2.));
        if visible.is_none() && !self.options.hidden_lines {
            return Ok(());
        }
        let samples = tooth
            .edge
            .sample(self.options.chord_tolerance_mm / 2., 0.3)
            .map_err(|_| limit())?;
        if let Some([a, b]) = visible {
            if self.options.hidden_lines {
                self.flank_span(tooth, &samples, [0., a], true, project)?;
            }
            self.flank_span(tooth, &samples, [a, b], false, project)?;
            if self.options.hidden_lines {
                self.flank_span(tooth, &samples, [b, 1.], true, project)?;
            }
        } else {
            self.flank_span(tooth, &samples, [0., 1.], true, project)?;
        }
        Ok(())
    }

    fn flank_span(
        &mut self,
        tooth: ToothEdge,
        samples: &[Point],
        [a, b]: [f64; 2],
        hidden: bool,
        project: &impl Fn(Point) -> Point,
    ) -> Result<(), GeometryError> {
        if b - a <= 64. * f64::EPSILON {
            return Ok(());
        }
        let mut last = project(tooth.edge.point(a));
        let count = (samples.len() - 1) as f64;
        let first = (a * count).floor() as usize + 1;
        let end = (b * count).ceil() as usize;
        for next in samples[first..end]
            .iter()
            .copied()
            .chain([tooth.edge.point(b)])
            .map(project)
        {
            if (next[0] - last[0]).hypot(next[1] - last[1]) <= 1e-7 {
                continue;
            }
            self.options.maximum_curves = self
                .options
                .maximum_curves
                .checked_sub(1)
                .ok_or_else(limit)?;
            self.options.maximum_points = self
                .options
                .maximum_points
                .checked_sub(2)
                .ok_or_else(limit)?;
            self.vectors.push(planar::line(
                0,
                if hidden {
                    VectorRole::Hidden
                } else {
                    VectorRole::Visible
                },
                [last, next],
            ));
            last = next;
        }
        Ok(())
    }
}

fn flank_visibility(tooth: ToothEdge, pitch: f64, threshold: f64) -> Option<[f64; 2]> {
    let visible = |fraction| {
        let [x, y] = tooth.edge.point(fraction);
        let angle = y.atan2(x) - f64::from(tooth.tooth) * pitch;
        let width = ((angle + PI).rem_euclid(TAU) - PI).abs();
        width < threshold
    };
    let first = visible(0.);
    let last = visible(1.);
    if first == last {
        return first.then_some([0., 1.]);
    }
    // Each directed flank, including the root fillet, stays in one
    // tooth sector and has monotone polar angle between its endpoints.
    let mut lo = 0_f64;
    let mut hi = 1_f64;
    for _ in 0..48 {
        let mid = lo.midpoint(hi);
        if visible(mid) == first {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let cut = lo.midpoint(hi);
    Some(if first { [0., cut] } else { [cut, 1.] })
}
