//! Full periodic torus encoded as one outward rectangular surface chart.
use super::*;
use crate::torus::Torus;
use std::f64::consts::TAU;

pub(super) fn encode(
    file: &mut StepFile,
    torus: Torus,
    placement: &Placement,
) -> Result<Entity, StepError> {
    let major = torus.major_radius_mm();
    let minor = torus.minor_radius_mm();
    let outer = major + minor;
    if placement
        .origin
        .iter()
        .any(|&x| x.abs() + outer > 1e12 || x + minor == x)
    {
        return Err(StepError::InvalidPlacement);
    }
    let center = file.point(placement.origin);
    let frame = file.axis(center, placement.normal, placement.u);
    let support = file.entity(format_args!(
        "TOROIDAL_SURFACE('',{frame},{major:.16E},{minor:.16E})"
    ));
    let vertex = file.vertex(placement.point_f64([outer, 0.], 0.)).vertex;
    let profile_center = file.point(placement.point_f64([major, 0.], 0.));
    let meridian_frame = file.axis(profile_center, placement.v.map(|x| -x), placement.u);
    let context = file.entity(format_args!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(2)PARAMETRIC_REPRESENTATION_CONTEXT()REPRESENTATION_CONTEXT('2D SPACE',''))"
    ));
    let outer = seam(file, support, context, vertex, frame, outer, 0);
    let meridian = seam(file, support, context, vertex, meridian_frame, minor, 1);
    let edges = [
        file.oriented_edge(outer, true),
        file.oriented_edge(meridian, true),
        file.oriented_edge(outer, false),
        file.oriented_edge(meridian, false),
    ];
    let bound = file.bound(&edges, false);
    let face = file.face_on(&[bound], support, true);
    Ok(file.solid(&[face]))
}

fn seam(
    file: &mut StepFile,
    support: Entity,
    context: Entity,
    vertex: Entity,
    frame: Entity,
    radius: f64,
    moving: usize,
) -> Entity {
    let circle = file.entity(format_args!("CIRCLE('',{frame},{radius:.16E})"));
    let mut direction = [0, 0];
    direction[moving] = 1;
    let [du, dv] = direction;
    let dir = file.entity(format_args!("DIRECTION('',({du}.,{dv}.))"));
    let vector = file.entity(format_args!("VECTOR('',{dir},1.)"));
    let [low, high] = [0., TAU].map(|position| {
        let mut coordinates = [0., 0.];
        coordinates[1 - moving] = position;
        let [u, v] = coordinates;
        let point = file.entity(format_args!("CARTESIAN_POINT('',({u:.16E},{v:.16E}))"));
        let line = file.entity(format_args!("LINE('',{point},{vector})"));
        let curve = file.entity(format_args!(
            "DEFINITIONAL_REPRESENTATION('',({line}),{context})"
        ));
        file.entity(format_args!("PCURVE('',{support},{curve})"))
    });
    let seam = file.entity(format_args!(
        "SEAM_CURVE('',{circle},({low},{high}),.PCURVE_S1.)"
    ));
    file.entity(format_args!("EDGE_CURVE('',{vertex},{vertex},{seam},.T.)"))
}
