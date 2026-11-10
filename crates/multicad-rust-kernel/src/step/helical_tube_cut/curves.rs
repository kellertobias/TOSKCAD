use super::*;
use std::f64::consts::FRAC_PI_2;

pub(super) enum Master {
    Curve,
    Pcurve,
    Seam,
}

pub(super) struct UvContext(Entity);

impl UvContext {
    pub(super) fn new(file: &mut StepFile) -> Self {
        Self(file.entity(format_args!(
            "(GEOMETRIC_REPRESENTATION_CONTEXT(2)PARAMETRIC_REPRESENTATION_CONTEXT()REPRESENTATION_CONTEXT('2D SPACE',''))"
        )))
    }

    pub(super) fn line(
        &self,
        file: &mut StepFile,
        surface: Entity,
        domain: [f64; 2],
        ends: [[f64; 2]; 2],
    ) -> Entity {
        self.spline(
            file,
            surface,
            &UvSpline {
                degree: 1,
                knots: domain.into(),
                multiplicities: vec![2; 2],
                poles_uv: ends.into(),
            },
        )
    }

    pub(super) fn spline(&self, file: &mut StepFile, surface: Entity, spline: &UvSpline) -> Entity {
        let poles = spline
            .poles_uv
            .iter()
            .map(|[u, v]| file.entity(format_args!("CARTESIAN_POINT('',({u:.16E},{v:.16E}))")))
            .collect::<Vec<_>>();
        let curve = spline_record(
            file,
            spline.degree,
            &poles,
            &spline.knots,
            &spline.multiplicities,
        );
        let context = self.0;
        let definition = file.entity(format_args!(
            "DEFINITIONAL_REPRESENTATION('',({curve}),{context})"
        ));
        file.entity(format_args!("PCURVE('',{surface},{definition})"))
    }
}

pub(super) fn edge(
    file: &mut StepFile,
    [from, to]: [Vertex; 2],
    curve: Entity,
    pcurves: &[Entity],
    master: Master,
) -> Entity {
    let [kind, selector] = match master {
        Master::Curve => ["SURFACE_CURVE", "CURVE_3D"],
        Master::Pcurve => ["SURFACE_CURVE", "PCURVE_S1"],
        Master::Seam => ["SEAM_CURVE", "PCURVE_S1"],
    };
    let curve = file.entity(format_args!(
        "{kind}('',{curve},{},.{selector}.)",
        Entities(pcurves),
    ));
    let (a, b) = (from.vertex, to.vertex);
    file.entity(format_args!("EDGE_CURVE('',{a},{b},{curve},.T.)"))
}

pub(super) fn circle(file: &mut StepFile, axis: Entity, radius: f64) -> Entity {
    file.entity(format_args!("CIRCLE('',{axis},{radius:.16E})"))
}

pub(super) struct StockChart<'a> {
    pub(super) uv: &'a UvContext,
    pub(super) cylinder: Entity,
    pub(super) placement: &'a Placement,
    pub(super) radius: f64,
}

impl StockChart<'_> {
    pub(super) fn arc(
        &self,
        file: &mut StepFile,
        vertices: [Vertex; 2],
        center: Entity,
        [a, b]: [f64; 2],
        z: f64,
    ) -> Entity {
        let (sin, cos) = a.sin_cos();
        let axis = file.axis(
            center,
            self.placement.normal,
            self.placement.vector([cos, sin, 0.]),
        );
        let curve = circle(file, axis, self.radius);
        let pcurve = self
            .uv
            .line(file, self.cylinder, [0., b - a], [[a, z], [b, z]]);
        edge(file, vertices, curve, &[pcurve], Master::Curve)
    }
}

pub(super) fn floor_cylinder_uv(
    cut: Cut,
    lift: f64,
    tolerance: f64,
) -> Result<UvSpline, StepError> {
    let [a, b] = cut.edge_domain(BoundaryEdge::FloorCylinder);
    let spec = cut.spec();
    let rise = spec.pitch_mm / TAU;
    let height_factor = spec.stock_radius_mm.powi(2) / rise;
    let sin = helical::Spline::new(a, b, [1.; 2], 1., tolerance / (32. * height_factor))?;
    let count = sin.knots.len();
    let parameters = std::iter::once(a).chain(sin.knots.windows(2).flat_map(|span| {
        (1..=helical::DEGREE).map(move |i| {
            if i == helical::DEGREE {
                span[1]
            } else {
                span[0] + (span[1] - span[0]) * i as f64 / helical::DEGREE as f64
            }
        })
    }));
    let poles_uv = sin
        .poles
        .into_iter()
        .zip(parameters)
        .map(|([_, y], u)| {
            [
                spec.start_angle_rad + lift + u,
                spec.start_height_mm - height_factor * y,
            ]
        })
        .collect();
    Ok(UvSpline {
        degree: helical::DEGREE,
        knots: sin.knots.into_iter().map(|u| u - FRAC_PI_2).collect(),
        multiplicities: (0..count)
            .map(|i| helical::DEGREE + usize::from(i == 0 || i + 1 == count))
            .collect(),
        poles_uv,
    })
}

pub(super) fn spline(
    file: &mut StepFile,
    spline: &Spline,
    [from, to]: [Vertex; 2],
    placement: &Placement,
    endpoint_tolerance: f64,
) -> Result<Entity, StepError> {
    let world = |[x, y, z]: Point| placement.point_f64([x, y], z);
    for (index, vertex) in [(0, from), (spline.poles_mm.len() - 1, to)] {
        let expected = world(spline.poles_mm[index]);
        let [x, y, z] = std::array::from_fn(|i| expected[i] - vertex.point[i]);
        if x.hypot(y).hypot(z) > endpoint_tolerance {
            return Err(StepError::InvalidGeometry);
        }
    }
    let poles = spline
        .poles_mm
        .iter()
        .enumerate()
        .map(|(i, &point)| {
            if i == 0 {
                from.cartesian
            } else if i + 1 == spline.poles_mm.len() {
                to.cartesian
            } else {
                file.point(world(point))
            }
        })
        .collect::<Vec<_>>();
    Ok(spline_record(
        file,
        spline.degree,
        &poles,
        &spline.knots,
        &spline.multiplicities,
    ))
}

fn values<T: std::fmt::Display>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn knots(values: &[f64]) -> String {
    values
        .iter()
        .map(|value| format!("{value:.16E}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn spline_record(
    file: &mut StepFile,
    degree: usize,
    poles: &[Entity],
    knot_values: &[f64],
    multiplicities: &[usize],
) -> Entity {
    let mults = values(multiplicities);
    let knots = knots(knot_values);
    file.entity(format_args!(
        "B_SPLINE_CURVE_WITH_KNOTS('',{degree},{},.UNSPECIFIED.,.F.,.F.,({mults}),({knots}),.UNSPECIFIED.)",
        Entities(poles)
    ))
}

pub(super) fn surface(
    file: &mut StepFile,
    tensor: &Surface,
    world: impl Fn(Point) -> Point,
) -> Entity {
    let [profile, sweep] = &tensor.axes;
    let row_len = (sweep.knots.len() - 1) * sweep.degree + 1;
    let net = tensor
        .poles_mm
        .chunks_exact(row_len)
        .map(|row| {
            let points = row
                .iter()
                .map(|&point| file.point(world(point)))
                .collect::<Vec<_>>();
            Entities(&points).to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    let [degree_u, degree_v] = [profile.degree, sweep.degree];
    let [mult_u, mult_v] = [profile, sweep].map(|axis| values(&axis.multiplicities));
    let [knots_u, knots_v] = [profile, sweep].map(|axis| knots(&axis.knots));
    file.entity(format_args!(
        "B_SPLINE_SURFACE_WITH_KNOTS('',{degree_u},{degree_v},({net}),.UNSPECIFIED.,.F.,.F.,.F.,({mult_u}),({mult_v}),({knots_u}),({knots_v}),.UNSPECIFIED.)"
    ))
}
