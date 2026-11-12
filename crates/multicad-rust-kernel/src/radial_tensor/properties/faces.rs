//! Retained face ownership and oriented edge parameterizations.
use super::*;
use measure::Density;
use source::{Circle, Source, Tensor};
pub(super) struct FaceSources {
    pub index: usize,
    pub density: Density,
    pub sources: Vec<Source>,
    pub plane: Option<Axis>,
    pub length: I,
    pub spline_length: I,
    pub circular_length: I,
    pub edge_count: usize,
}
pub(super) fn build(body: &RadialBoundary, work: &mut Work<'_>) -> Result<Vec<FaceSources>, Error> {
    with_limit(body, work, 32768)
}
pub(super) fn with_limit(
    body: &RadialBoundary,
    work: &mut Work<'_>,
    source_limit: usize,
) -> Result<Vec<FaceSources>, Error> {
    let axial = &body.ordered.caps.axial;
    let input = body.input();
    let chart = &axial.chart;
    let mut faces = Vec::with_capacity(input.faces.len());
    // The certified 3n tensor/6n cone/10n cap-edge structure permits
    // at most 128 edge occurrences. Each retained degree-nine source has
    // bounded poles, so this count cannot overflow even a 32-bit usize.
    // Every spline edge is one of the certified tensor iso-edges. Thus
    // calculation sources <= MAX_POLES/81 +2*MAX_POLES/45 +64 circles,
    // below 32768 even at the full public manufacturing pole capacity.
    let mut source_count = 0usize;
    for tensor in &axial.tensors {
        let mut sources = Vec::new();
        source_count += tensor.spans[0] * tensor.spans[1];
        if source_count > source_limit {
            return Err(Error::Limit);
        }
        let height = 9 * tensor.spans[1] + 1;
        for u in 0..tensor.spans[0] {
            for v in 0..tensor.spans[1] {
                work.charge(100)?;
                let p = std::array::from_fn(|i| {
                    tensor.controls[(9 * u + i / 10) * height + 9 * v + i % 10]
                });
                let geometry = crate::tapered_thread::calculation_cell(p);
                work.charge(8192)?;
                let normal =
                    std::sync::Arc::new(crate::tapered_thread::normal_polynomial(&geometry));
                sources.push(Source::Tensor(Tensor { geometry, normal }));
            }
        }
        faces.push(FaceSources {
            index: tensor.face,
            density: Density::Tensor,
            sources,
            plane: None,
            length: I::point(0.),
            spline_length: I::point(0.),
            circular_length: I::point(0.),
            edge_count: 0,
        });
    }
    for (index, face) in input.faces.iter().enumerate() {
        work.step()?;
        let (density, plane, edges) = match &input.surfaces[face.surface] {
            Surface::Tensor(_) => continue,
            Surface::Plane(axis) => {
                let end = usize::from(index == axial.caps[1]);
                let edges = body.ordered.caps.rims[end]
                    .iter()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                (
                    Density::Cap(chart.project(axis.origin)[2]),
                    Some(*axis),
                    edges,
                )
            }
            Surface::Cone { .. } => (Density::Cone, None, face.outer.clone()),
        };
        let mut sources = Vec::new();
        let mut spline_length = I::point(0.);
        let mut circular_length = I::point(0.);
        for &(edge, direction) in &edges {
            let e = input.edges[edge];
            match &input.curves[e.curve] {
                Curve::Spline { spans, poles } => {
                    source_count += *spans;
                    if source_count > source_limit {
                        return Err(Error::Limit);
                    }
                    let mut points = Vec::with_capacity(poles.len());
                    for chunk in poles.chunks(128) {
                        work.charge(chunk.len())?;
                        points.extend(chunk.iter().map(|&p| chart.project(p)));
                    }
                    if e.forward != direction {
                        points.reverse();
                    }
                    for span in 0..*spans {
                        let p = &points[9 * span..=9 * span + 9];
                        for pair in p.windows(2) {
                            let d = std::array::from_fn(|i| pair[1][i] - pair[0][i]);
                            spline_length = spline_length + dot(d, d).sqrt();
                        }
                        sources.push(Source::Spline(p.to_vec()));
                    }
                }
                Curve::Circle { axis, radius_mm } => {
                    source_count += 1;
                    if source_count > source_limit {
                        return Err(Error::Limit);
                    }
                    work.charge(64)?;
                    let basis = checked_axis(*axis)?;
                    let [a, b] = if direction {
                        e.vertices
                    } else {
                        [e.vertices[1], e.vertices[0]]
                    };
                    let local = |p: Point| {
                        let d = std::array::from_fn(|i| I::point(p[i]) - I::point(axis.origin[i]));
                        [dot(d, basis[0]), dot(d, basis[1]), I::point(0.)]
                    };
                    let [a, b] = [local(input.vertices[a]), local(input.vertices[b])];
                    let sweep = if e.forward == direction {
                        caps::phase(a, b)
                    } else {
                        caps::phase(b, a).map(|v| -v)
                    }
                    .ok_or(Error::Unresolved)?;
                    let start = argument([a[0], a[1]])?;
                    let radius = I::point(*radius_mm);
                    circular_length = circular_length + radius * I::point(sweep.magnitude());
                    sources.push(Source::Circle(Circle {
                        center: chart.project(axis.origin),
                        u: chart.axes.map(|row| dot(row, basis[0])),
                        v: chart.axes.map(|row| dot(row, basis[1])),
                        radius,
                        start,
                        sweep,
                        range: [0., 1.],
                    }));
                }
            }
        }
        faces.push(FaceSources {
            index,
            density,
            sources,
            plane,
            length: spline_length + circular_length,
            spline_length,
            circular_length,
            edge_count: edges.len(),
        });
    }
    faces.sort_by_key(|f| f.index);
    Ok(faces)
}
fn argument(p: [I; 2]) -> Result<I, Error> {
    let h = p.map(|v| I::point(v.midpoint()));
    let dot = h[0] * p[0] + h[1] * p[1];
    dot.positive()?;
    let delta = ((h[0] * p[1] - h[1] * p[0]) / dot).atan();
    // As in the interval atan/sin/cos primitives, include libm's directed
    // preparation reserve; the relative interval proves the branch choice.
    let a = h[1].lo.atan2(h[0].lo);
    (I::new(
        (a - 8. * f64::EPSILON).next_down(),
        (a + 8. * f64::EPSILON).next_up(),
    ) + delta)
        .checked()
}
