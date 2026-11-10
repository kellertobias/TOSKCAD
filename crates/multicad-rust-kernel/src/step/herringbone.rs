//! One closed chevron with shared seam edges and bounded tensor surfaces.
use super::{
    helical::{DEGREE, Spline, product},
    *,
};
use crate::{
    normal_sweep::Edge as PlanarEdge,
    spur_gear::{Edge, Error, herringbone::Herringbone},
    step_read::MAX_STEP_ENTITIES,
};

const MAX_ERROR_MM: f64 = 1e-7;

/// Whether this single authored chevron fits the placement and spline budget.
pub fn supports_herringbone(
    gear: &Herringbone,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> bool {
    Placement::new(origin, u, v).is_ok() && prepare(gear, origin, tolerance_mm).is_ok()
}

pub fn write_herringbone(
    gear: &Herringbone,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_herringbone(gear, origin, u, v, tolerance_mm))
}

#[derive(Clone)]
pub(crate) struct Chart {
    pub(crate) degree: usize,
    pub(crate) interior_mult: usize,
    pub(crate) knots: Vec<f64>,
    pub(crate) poles: Vec<[f64; 2]>,
    pub(crate) weights: Option<Vec<f64>>,
}
impl Chart {
    fn from_spin(spline: Spline) -> Self {
        Self {
            degree: DEGREE,
            interior_mult: DEGREE,
            knots: spline.knots,
            poles: spline.poles,
            weights: None,
        }
    }

    fn transverse(edge: Edge, ends: [[f64; 2]; 2], tolerance: f64) -> Result<Self, StepError> {
        let mut chart = match edge {
            Edge::Planar(PlanarEdge::Line { .. }) => Self {
                degree: 1,
                interior_mult: 1,
                knots: vec![0., 1.],
                poles: ends.to_vec(),
                weights: None,
            },
            Edge::Planar(edge @ PlanarEdge::Arc { center, .. }) => {
                let sweep = edge.signed_sweep_rad().ok_or(StepError::InvalidGeometry)?;
                let spans = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as u32;
                let weight = (sweep / (2. * f64::from(spans))).cos();
                let mut poles = vec![ends[0]];
                let mut weights = vec![1.];
                let knots = (0..=spans)
                    .map(|i| f64::from(i) / f64::from(spans))
                    .collect::<Vec<_>>();
                for pair in knots.windows(2) {
                    let mid = edge.point(pair[0].midpoint(pair[1]));
                    poles.push(std::array::from_fn(|i| {
                        center[i] + (mid[i] - center[i]) / weight
                    }));
                    poles.push(edge.point(pair[1]));
                    weights.extend([weight, 1.]);
                }
                Self {
                    degree: 2,
                    interior_mult: 2,
                    knots,
                    poles,
                    weights: Some(weights),
                }
            }
            Edge::Involute(flank) => {
                let cubic = flank
                    .cubic_spline(tolerance / 16.)
                    .map_err(|cause| match cause {
                        Error::Limit => StepError::Limit,
                        _ => StepError::InvalidGeometry,
                    })?;
                Self {
                    degree: 3,
                    interior_mult: 2,
                    knots: cubic.knots,
                    poles: cubic.poles_mm,
                    weights: None,
                }
            }
            Edge::Planar(PlanarEdge::Circle { .. }) => return Err(StepError::Unsupported),
        };
        let last = chart.poles.len() - 1;
        for (index, expected) in [0, last].into_iter().zip(ends) {
            let [x, y] = std::array::from_fn(|i| chart.poles[index][i] - expected[i]);
            if x.hypot(y) > tolerance / 16. {
                return Err(StepError::Limit);
            }
            chart.poles[index] = expected;
        }
        Ok(chart)
    }

    fn reversed(self) -> Self {
        let end = self.knots[self.knots.len() - 1];
        Self {
            knots: self.knots.into_iter().rev().map(|k| end - k).collect(),
            poles: self.poles.into_iter().rev().collect(),
            weights: self
                .weights
                .map(|weights| weights.into_iter().rev().collect()),
            ..self
        }
    }

    fn knot_values(&self) -> String {
        self.knots
            .iter()
            .map(|knot| format!("{knot:.16E}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn multiplicities(&self) -> String {
        let mut values = vec![self.interior_mult; self.knots.len()];
        values[0] = self.degree + 1;
        values[self.knots.len() - 1] = self.degree + 1;
        values
            .into_iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn edge(
        &self,
        file: &mut StepFile,
        [a, b]: [Vertex; 2],
        points: impl Iterator<Item = Point>,
    ) -> Entity {
        let poles = points
            .enumerate()
            .map(|(i, point)| {
                if i == 0 {
                    a.cartesian
                } else if i + 1 == self.poles.len() {
                    b.cartesian
                } else {
                    file.point(point)
                }
            })
            .collect::<Vec<_>>();
        let mult = self.multiplicities();
        let knots = self.knot_values();
        let curve = if let Some(weights) = &self.weights {
            let weights = weights
                .iter()
                .map(|value| format!("{value:.16E}"))
                .collect::<Vec<_>>()
                .join(",");
            file.entity(format_args!(
                "(BOUNDED_CURVE() B_SPLINE_CURVE({},{},.UNSPECIFIED.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS(({mult}),({knots}),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE(({weights})) REPRESENTATION_ITEM(''))",
                self.degree, Entities(&poles),
            ))
        } else {
            file.entity(format_args!(
                "B_SPLINE_CURVE_WITH_KNOTS('',{},{},.UNSPECIFIED.,.F.,.F.,({mult}),({knots}),.UNSPECIFIED.)",
                self.degree, Entities(&poles),
            ))
        };
        file.entity(format_args!(
            "EDGE_CURVE('',{},{},{curve},.T.)",
            a.vertex, b.vertex
        ))
    }
}

fn axial_rotation(angle: f64, height: f64, error: f64) -> Result<(Chart, Vec<f64>), StepError> {
    let magnitude = angle.abs();
    // Degree-five two-end Hermite matches value and two derivatives.
    // The remainder is bounded by (h/2)^6 / 6! for a unit complex rotation.
    if (magnitude / 2.).powi(6) / 720. <= error {
        let control_half = |p: [f64; 2], step: f64| {
            let tangent = [-p[1], p[0]];
            let point = |linear: f64, quadratic: f64| {
                std::array::from_fn(|i| {
                    p[i] + linear * step * tangent[i] - quadratic * step.powi(2) * p[i]
                })
            };
            [p, point(0.2, 0.), point(0.4, 0.05)]
        };
        let (sin, cos) = angle.sin_cos();
        let [a, b, c] = control_half([1., 0.], angle);
        let [f, e, d] = control_half([cos, sin], -angle);
        let poles = vec![a, b, c, d, e, f];
        // B and B' are convex combinations of their Bernstein controls.
        // Pairwise strictly signed cross products prove monotone angle.
        let monotone = poles.iter().all(|[x, y]| {
            poles.windows(2).all(|pair| {
                let cross = x * (pair[1][1] - pair[0][1]) - y * (pair[1][0] - pair[0][0]);
                angle.signum() * cross > 64. * f64::EPSILON
            })
        });
        if monotone {
            return Ok((
                Chart {
                    degree: 5,
                    interior_mult: 5,
                    knots: vec![0., magnitude],
                    poles,
                    weights: None,
                },
                (0..=5).map(|i| height * f64::from(i) / 5.).collect(),
            ));
        }
    }
    let spin = Spline::new(0., magnitude, [1.; 2], angle.signum(), error)?;
    if !spin.advances_angle(angle.signum()) {
        return Err(StepError::Limit);
    }
    let z = spin.linear_poles(height);
    Ok((Chart::from_spin(spin), z))
}

pub(crate) struct Charts {
    pub(crate) transverse: Vec<Chart>,
    pub(crate) spins: [Chart; 2],
    pub(crate) heights: [Vec<f64>; 2],
    pub(crate) phases: [[f64; 2]; 3],
    pub(crate) levels: [f64; 3],
}

pub(crate) fn prepare(
    gear: &Herringbone,
    origin: Point,
    tolerance: f64,
) -> Result<Charts, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance.min(MAX_ERROR_MM);
    let profile = gear.profile();
    let height = profile.height_mm();
    let middle = height * 0.5;
    let [lo, hi] = gear.bounds();
    let extent = |i: usize| lo[i].abs().max(hi[i].abs());
    let radius = extent(0).hypot(extent(1));
    let scale = origin.into_iter().map(f64::abs).fold(0., f64::max) + radius + height + 1.;
    if 128. * f64::EPSILON * scale > tolerance / 4. {
        return Err(StepError::Limit);
    }

    // A common complex rotation polynomial keeps every section in a band a
    // single scaled rotation of one transverse section. The two bands share
    // the same interpolant in reverse and therefore meet at the same apex.
    let angle = gear.phase_at(middle);
    let (spin, z) = axial_rotation(angle, middle, tolerance / (16. * radius))?;
    let upper_z = z.iter().rev().map(|z| height - z).collect::<Vec<_>>();
    let spins = [spin.clone(), spin.reversed()];
    let heights = [z, upper_z];
    let phases = [[1., 0.], spins[1].poles[0], [1., 0.]];
    let levels = [0., middle, height];
    let outline = profile.outline();
    let count = outline.len();
    let transverse = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(a, b)| Chart::transverse(a.edge, [a.edge.point(0.), b.edge.point(0.)], tolerance))
        .collect::<Result<Vec<_>, _>>()?;
    let poles: usize = transverse.iter().map(|chart| chart.poles.len()).sum();
    let overhead = count
        .checked_mul(2 * spins[0].poles.len() + 64)
        .ok_or(StepError::Limit)?;
    if poles
        .checked_mul(2 * spins[0].poles.len() + 3)
        .and_then(|cost| cost.checked_add(overhead + 128))
        .is_none_or(|cost| cost > MAX_STEP_ENTITIES)
    {
        return Err(StepError::Limit);
    }

    Ok(Charts {
        transverse,
        spins,
        heights,
        phases,
        levels,
    })
}

pub(super) fn encode(
    file: &mut StepFile,
    gear: &Herringbone,
    placement: Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    let Charts {
        transverse,
        spins,
        heights,
        phases,
        levels,
    } = prepare(gear, placement.origin, tolerance)?;
    let profile = gear.profile();
    let count = transverse.len();
    let height = levels[2];
    let stations = [0, 1, 2].map(|level| {
        transverse
            .iter()
            .map(|chart| {
                file.vertex(
                    placement.point_f64(product(chart.poles[0], phases[level]), levels[level]),
                )
            })
            .collect::<Vec<_>>()
    });
    let rims = [0, 1, 2].map(|level| {
        transverse
            .iter()
            .enumerate()
            .map(|(index, chart)| {
                let next = (index + 1) % count;
                let [a, b] = [stations[level][index], stations[level][next]];
                if chart.degree == 1 {
                    return line_edge(file, a, b);
                }
                let points = chart.poles.iter().map(|&point| {
                    placement.point_f64(product(point, phases[level]), levels[level])
                });
                Ok(chart.edge(file, [a, b], points))
            })
            .collect::<Result<Vec<_>, _>>()
    });
    let [start, apex, end] = rims;
    let rims = [start?, apex?, end?];

    let mut faces = Vec::with_capacity(2 * count + 3);
    for (band, axial) in spins.iter().enumerate() {
        let heights = &heights[band];
        let rails = transverse
            .iter()
            .enumerate()
            .map(|(index, chart)| {
                let points = axial
                    .poles
                    .iter()
                    .zip(heights)
                    .map(|(&spin, &z)| placement.point_f64(product(chart.poles[0], spin), z));
                axial.edge(
                    file,
                    [stations[band][index], stations[band + 1][index]],
                    points,
                )
            })
            .collect::<Vec<_>>();
        for (index, profile) in transverse.iter().enumerate() {
            let next = (index + 1) % count;
            let rows = profile
                .poles
                .iter()
                .map(|&point| {
                    let row = axial
                        .poles
                        .iter()
                        .zip(heights)
                        .map(|(&spin, &z)| file.point(placement.point_f64(product(point, spin), z)))
                        .collect::<Vec<_>>();
                    Entities(&row).to_string()
                })
                .collect::<Vec<_>>()
                .join(",");
            let [um, vm] = [profile.multiplicities(), axial.multiplicities()];
            let [uk, vk] = [profile.knot_values(), axial.knot_values()];
            let surface = if let Some(weights) = &profile.weights {
                let weights = weights
                    .iter()
                    .map(|value| {
                        let row = vec![format!("{value:.16E}"); axial.poles.len()].join(",");
                        format!("({row})")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                file.entity(format_args!(
                    "(BOUNDED_SURFACE() B_SPLINE_SURFACE({},{},({rows}),.UNSPECIFIED.,.F.,.F.,.F.) B_SPLINE_SURFACE_WITH_KNOTS(({um}),({vm}),({uk}),({vk}),.UNSPECIFIED.) GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE(({weights})) REPRESENTATION_ITEM('') SURFACE())",
                    profile.degree, axial.degree,
                ))
            } else {
                file.entity(format_args!(
                    "B_SPLINE_SURFACE_WITH_KNOTS('',{},{},({rows}),.UNSPECIFIED.,.F.,.F.,.F.,({um}),({vm}),({uk}),({vk}),.UNSPECIFIED.)",
                    profile.degree, axial.degree,
                ))
            };
            let wire = [
                (rims[band][index], true),
                (rails[next], true),
                (rims[band + 1][index], false),
                (rails[index], false),
            ]
            .map(|(edge, sense)| file.oriented_edge(edge, sense));
            let bound = file.bound(&wire, true);
            faces.push(file.face_on(&[bound], surface, true));
        }
    }

    let centers = [0., height].map(|z| placement.point_f64([0.; 2], z));
    let bore = profile
        .bore()
        .and_then(PlanarEdge::radius_mm)
        .map(|radius| CylindricalTrim::new(file, centers, placement.normal, placement.u, radius))
        .transpose()?;
    for (end, ring) in [(&rims[0], false), (&rims[2], true)]
        .into_iter()
        .enumerate()
    {
        let (ring, top) = ring;
        let mut edges = ring
            .iter()
            .map(|&edge| file.oriented_edge(edge, top))
            .collect::<Vec<_>>();
        if !top {
            edges.reverse();
        }
        let outer = file.bound(&edges, true);
        let mut loops = vec![outer];
        if let Some(bore) = &bore {
            loops.push(bore.cap_bound(file, top, false));
        }
        let point = file.point(centers[end]);
        let normal = placement.normal.map(|n| if top { n } else { -n });
        faces.push(file.face(&loops, point, normal, placement.u));
    }
    if let Some(bore) = bore {
        faces.push(bore.wall(file, false));
    }
    let solid = file.solid(&faces);
    if file.next + 128 > MAX_STEP_ENTITIES
        || file.data.len() + 4096 > crate::step_read::MAX_PLANAR_STEP_BYTES
    {
        return Err(StepError::Limit);
    }
    Ok(solid)
}

#[cfg(test)]
mod tests;
