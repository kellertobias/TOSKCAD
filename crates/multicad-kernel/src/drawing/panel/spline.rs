//! Exact axial projection of a verified spline sweep.
use super::*;
use multicad_rust_kernel::{display::Tessellation, spline_sweep};

pub(in crate::drawing) fn contains_spline_sweep(solid: &Solid) -> bool {
    solid.spline_sweep_body().is_some()
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(contains_spline_sweep))
}

pub(in crate::drawing) fn project_spline_sweep(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (body, frame) = input.solid.spline_sweep_body()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = frame.normal();
    if dot(axis, u).hypot(dot(axis, v)) > 32. * f64::EPSILON
        || options.detail.is_some()
        || options.section_offset_mm.is_some()
    {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust spline sweep drawing requires an uncropped axial projection",
        )));
    }
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    let (cap, outline, analytic) = body.axial_drawing_rims(frame, dot(axis, toward_viewer) > 0.);
    Some((|| {
        let rims = analytic
            .map(|geometry| {
                Ok(Edge {
                    hidden: false,
                    curve: screen.curve(Some(geometry))?,
                })
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        let splines = outline.iter().filter_map(|edge| match edge {
            spline_sweep::Edge::Cubic(cubic) => Some((edge, cubic)),
            spline_sweep::Edge::Analytic(_) => None,
        });
        let curve_budget = || {
            err(
                GeometryErrorKind::Limit,
                "Spline sweep drawing exceeds its curve budget",
            )
        };
        let mut remaining = options;
        remaining.maximum_curves = (options.maximum_curves as usize)
            .checked_sub(splines.clone().count())
            .filter(|&count| count >= rims.len())
            .ok_or_else(curve_budget)? as u32;
        let project = |point| screen.point(cap.point(point));
        let mut vectors = Vec::new();
        // Both caps have the same section. The rear rims coincide in
        // projection and axial edges project to points, including the bore seam.
        for (edge, cubic) in splines {
            vectors.push(cubic_vector(
                edge,
                cubic,
                &project,
                options,
                &mut remaining.maximum_points,
            )?);
        }
        vectors.extend(curved::publish(rims, remaining, 0)?);
        Ok(vectors)
    })())
}

fn cubic_vector(
    edge: &spline_sweep::Edge,
    cubic: &spline_sweep::Cubic,
    project: &dyn Fn(Point) -> Point,
    options: DrawingProjection,
    remaining_points: &mut u32,
) -> Result<ProjectedVector, GeometryError> {
    let point_budget = || {
        err(
            GeometryErrorKind::Limit,
            "Spline sweep drawing exceeds its point or preview budget",
        )
    };
    if *remaining_points < 2 || cubic.poles().len() > options.maximum_points as usize {
        return Err(point_budget());
    }
    let mut preview_mm: Vec<_> = edge
        .sample(Tessellation {
            deflection: options.chord_tolerance_mm,
            angle: 0.3,
            triangle_limit: *remaining_points as usize - 1,
        })
        .map_err(|_| point_budget())?
        .into_iter()
        .map(project)
        .collect();
    *remaining_points -= preview_mm.len() as u32;
    if cubic.reversed() {
        preview_mm.reverse();
    }
    let knots = cubic.knots();
    let mut multiplicities = vec![2; knots.len()];
    multiplicities[0] = 4;
    multiplicities[knots.len() - 1] = 4;
    Ok(ProjectedVector {
        source_index: 0,
        role: VectorRole::Visible,
        curve: VectorCurve::BSpline {
            degree: 3,
            periodic: false,
            poles: cubic.poles().iter().copied().map(project).collect(),
            weights: vec![1.; cubic.poles().len()],
            knots: knots.to_vec(),
            multiplicities,
            first: knots[0],
            last: knots[knots.len() - 1],
        },
        preview_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_preview_follows_source_knots_and_charges_the_exact_point_count() {
        let cubic = spline_sweep::Cubic::new(
            vec![[3., 1.], [2., 1.], [1., 0.], [0., 0.]],
            vec![-2., 1.],
            true,
        )
        .unwrap();
        let edge = spline_sweep::Edge::Cubic(cubic.clone());
        let project = |[x, y]: Point| [3. + x, -y];
        let options = DrawingProjection {
            chord_tolerance_mm: 1e-5,
            ..Default::default()
        };
        let mut remaining = 1_000;
        let vector = cubic_vector(&edge, &cubic, &project, options, &mut remaining).unwrap();
        assert_eq!(vector.preview_mm[0], [6., -1.]);
        assert_eq!(vector.preview_mm.last(), Some(&[3., 0.]));
        assert_eq!(remaining, 1_000 - vector.preview_mm.len() as u32);
        assert_eq!(
            cubic_vector(&edge, &cubic, &project, options, &mut 2)
                .unwrap_err()
                .kind,
            GeometryErrorKind::Limit
        );
        assert_eq!(
            cubic_vector(
                &edge,
                &cubic,
                &project,
                DrawingProjection {
                    maximum_points: 3,
                    ..options
                },
                &mut 3
            )
            .unwrap_err()
            .kind,
            GeometryErrorKind::Limit
        );
    }
}
