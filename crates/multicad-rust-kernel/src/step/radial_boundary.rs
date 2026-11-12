//! Re-export the retained finite boundary, without a reconstructed recipe.
use super::*;
use crate::radial_tensor::{Axis, BoundaryInput, Curve, RadialBoundary, Surface};
#[cfg(test)]
mod tests;

pub fn write_radial_boundary(
    boundary: &RadialBoundary,
    tolerance: f64,
) -> Result<String, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let mut file = StepFile::new();
    // Source table indices carry no geometric meaning. Emit their stable
    // first-use order so blind read/re-export does not churn entity numbers.
    let input = canonical(boundary.input());
    let solid = encode(&mut file, &input)?;
    file.finish(vec![solid]);
    verify_candidate(&file.data, boundary, tolerance)?;
    Ok(file.data)
}
fn verify_candidate(
    input: &str,
    expected: &RadialBoundary,
    tolerance: f64,
) -> Result<(), StepError> {
    let restored = crate::step_read::read_radial_boundary(input, tolerance).map_err(|e| {
        if e == crate::step_read::ReadError::Limit {
            StepError::Limit
        } else {
            StepError::Unsupported
        }
    })?;
    // Exact value/ownership equality, including poles, sense and raw axes.
    // Reading can renumber tables by first use, so compare that deterministic
    // representation rather than treating entity order as physical identity.
    if canonical(restored.input()) != canonical(expected.input()) {
        return Err(StepError::Unsupported);
    }
    Ok(())
}
fn canonical(input: &BoundaryInput) -> BoundaryInput {
    let mut out = BoundaryInput {
        vertices: vec![],
        curves: vec![],
        edges: vec![],
        surfaces: vec![],
        faces: vec![],
    };
    let mut vertices = BTreeMap::new();
    let mut curves = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for face in &input.faces {
        let surface = *surfaces.entry(face.surface).or_insert_with(|| {
            let i = out.surfaces.len();
            out.surfaces.push(input.surfaces[face.surface].clone());
            i
        });
        let mut outer = Vec::new();
        for &(edge, direction) in &face.outer {
            let edge = *edges.entry(edge).or_insert_with(|| {
                let source = input.edges[edge];
                let ends = source.vertices.map(|v| {
                    *vertices.entry(v).or_insert_with(|| {
                        let i = out.vertices.len();
                        out.vertices.push(input.vertices[v]);
                        i
                    })
                });
                let curve = *curves.entry(source.curve).or_insert_with(|| {
                    let i = out.curves.len();
                    out.curves.push(input.curves[source.curve].clone());
                    i
                });
                let i = out.edges.len();
                out.edges.push(crate::radial_tensor::Edge {
                    vertices: ends,
                    curve,
                    forward: source.forward,
                });
                i
            });
            outer.push((edge, direction));
        }
        out.faces.push(crate::radial_tensor::Face {
            surface,
            outer,
            forward: face.forward,
        });
    }
    out
}
fn axis(file: &mut StepFile, a: Axis) -> Entity {
    let origin = file.point(a.origin);
    file.axis(origin, a.normal, a.reference)
}
fn knot_data(spans: usize) -> (String, String) {
    let multiplicities = (0..=spans)
        .map(|i| if i == 0 || i == spans { "10" } else { "9" })
        .collect::<Vec<_>>()
        .join(",");
    let knots = (0..=spans)
        .map(|i| format!("{:.16E}", i as f64 / spans as f64))
        .collect::<Vec<_>>()
        .join(",");
    (multiplicities, knots)
}
fn encode(file: &mut StepFile, input: &BoundaryInput) -> Result<Entity, StepError> {
    let vertices = input
        .vertices
        .iter()
        .map(|&p| file.vertex(p))
        .collect::<Vec<_>>();
    let curves=input.curves.iter().map(|c|match c {
        Curve::Circle{axis:a,radius_mm}=>{let a=axis(file,*a);file.entity(format_args!("CIRCLE('',{a},{radius_mm:.16E})"))},
        Curve::Spline{spans,poles}=>{let points=poles.iter().map(|&p|file.point(p)).collect::<Vec<_>>();let(m,k)=knot_data(*spans);file.entity(format_args!("B_SPLINE_CURVE_WITH_KNOTS('',9,{},.UNSPECIFIED.,.F.,.F.,({m}),({k}),.UNSPECIFIED.)",Entities(&points)))},
    }).collect::<Vec<_>>();
    let surfaces=input.surfaces.iter().map(|s|match s {
        Surface::Plane(a)=>{let a=axis(file,*a);file.entity(format_args!("PLANE('',{a})"))},
        Surface::Cone{axis:a,radius_mm,semi_angle_rad}=>{let a=axis(file,*a);if *semi_angle_rad==0. {file.entity(format_args!("CYLINDRICAL_SURFACE('',{a},{radius_mm:.16E})"))}else{file.entity(format_args!("CONICAL_SURFACE('',{a},{radius_mm:.16E},{semi_angle_rad:.16E})"))}},
        Surface::Tensor(t)=>{let points=t.poles().iter().map(|&p|file.point(p)).collect::<Vec<_>>();let rows=points.chunks_exact(t.dimensions()[1]).map(|r|Entities(r).to_string()).collect::<Vec<_>>().join(",");let[nu,nv]=t.spans();let(um,uk)=knot_data(nu);let(vm,vk)=knot_data(nv);file.entity(format_args!("B_SPLINE_SURFACE_WITH_KNOTS('',9,9,({rows}),.UNSPECIFIED.,.F.,.F.,.F.,({um}),({vm}),({uk}),({vk}),.UNSPECIFIED.)"))},
    }).collect::<Vec<_>>();
    let edges = input
        .edges
        .iter()
        .map(|e| {
            let [a, b] = e.vertices.map(|v| vertices[v].vertex);
            let c = curves[e.curve];
            let sense = if e.forward { ".T." } else { ".F." };
            file.entity(format_args!("EDGE_CURVE('',{a},{b},{c},{sense})"))
        })
        .collect::<Vec<_>>();
    let faces = input
        .faces
        .iter()
        .map(|f| {
            let wire = f
                .outer
                .iter()
                .map(|&(e, d)| file.oriented_edge(edges[e], d))
                .collect::<Vec<_>>();
            let bound = file.bound(&wire, true);
            file.face_on(&[bound], surfaces[f.surface], f.forward)
        })
        .collect::<Vec<_>>();
    Ok(file.solid(&faces))
}
