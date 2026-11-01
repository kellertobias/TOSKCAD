//! Native mounting geometry and nut threads from retained canonical parameters.
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{MechanicalPartDefinition, ResolvedMechanicalPart, ThreadMode},
    parameters::EvaluatedParameters,
    references::Origin,
    sketch::Frame,
};
use multicad_kernel::{CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid};
fn extrude(
    feature: EntityId,
    frame: Frame,
    height: f64,
    outer: Vec<CurvedEdge2d>,
    holes: Vec<Vec<CurvedEdge2d>>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let mut origins = (0..outer.len())
        .map(|i| Origin {
            source: feature,
            role: format!("hardware.outer/{i}"),
        })
        .collect::<Vec<_>>();
    for (loop_index, edges) in holes.iter().enumerate() {
        origins.extend((0..edges.len()).map(|i| Origin {
            source: feature,
            role: format!("hardware.bore/{loop_index}/{i}"),
        }));
    }
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: frame.origin,
            u: frame.u,
            v: frame.v,
        },
        outer,
        holes,
    };
    Solid::extrude_curved_profile_vector_named(
        &profile,
        frame.n.map(|n| n * height),
        feature,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(feature), e))
}
fn ring(
    feature: EntityId,
    frame: Frame,
    bore: f64,
    outside: f64,
    height: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let circle = |radius_mm| CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm,
    };
    extrude(
        feature,
        frame,
        height,
        vec![circle(outside)],
        vec![vec![circle(bore)]],
        policy,
    )
}
pub(crate) fn build(
    feature: EntityId,
    source: &MechanicalPartDefinition,
    evaluated: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let resolved = source
        .parameters
        .resolve(evaluated)
        .map_err(|e| error(Some(feature), e))?;
    match resolved {
        ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            frame,
        } => ring(
            feature,
            frame,
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            policy,
        ),
        ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            frame,
        } => ring(
            feature,
            frame,
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            policy,
        ),
        ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } => {
            let radius = across_flats_mm / 3_f64.sqrt();
            let point = |i: usize| {
                let angle = std::f64::consts::PI / 6. + i as f64 * std::f64::consts::TAU / 6.;
                [radius * angle.cos(), radius * angle.sin()]
            };
            let outer = (0..6)
                .map(|i| CurvedEdge2d::Line {
                    a: point(i),
                    b: point((i + 1) % 6),
                })
                .collect();
            let blank = extrude(
                feature,
                thread.frame,
                thread.length_mm,
                outer,
                vec![],
                policy,
            )?;
            crate::threads::build_resolved(
                feature,
                *thread,
                ThreadMode::CutInternal,
                Some(&blank),
                policy,
            )
        }
    }
}
