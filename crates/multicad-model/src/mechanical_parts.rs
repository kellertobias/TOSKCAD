//! Native mounting geometry and nut threads from retained canonical parameters.
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{
        MechanicalPartDefinition, ResolvedMechanicalPart, ResolvedScrewDrive, ResolvedScrewHead,
        ThreadMode,
    },
    parameters::EvaluatedParameters,
    references::Origin,
    sketch::Frame,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
};

fn hexagon(across_flats: f64) -> Vec<CurvedEdge2d> {
    let radius = across_flats / 3_f64.sqrt();
    let point = |i: usize| {
        let angle = std::f64::consts::PI / 6. + i as f64 * std::f64::consts::TAU / 6.;
        [radius * angle.cos(), radius * angle.sin()]
    };
    (0..6)
        .map(|i| CurvedEdge2d::Line {
            a: point(i),
            b: point((i + 1) % 6),
        })
        .collect()
}

/// Screw components are built in the underhead coordinate system. Each owns
/// distinct profile and cap roles before native ancestry is combined.
fn screw_prism(
    feature: EntityId,
    component: &str,
    z: f64,
    height: f64,
    outer: Vec<CurvedEdge2d>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let origins = (0..outer.len())
        .map(|i| Origin {
            source: feature,
            role: format!("screw.{component}/edge/{i}"),
        })
        .collect::<Vec<_>>();
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., z],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        outer,
        holes: vec![],
    };
    Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., height],
        feature,
        &origins,
        policy,
    )
    .and_then(|s| {
        s.with_feature_cap_roles(
            feature,
            &format!("screw.{component}/start"),
            &format!("screw.{component}/end"),
        )
    })
    .map_err(|e| error(Some(feature), e))
}
fn screw_boolean(
    feature: EntityId,
    a: &Solid,
    b: &Solid,
    operation: BooleanOperation,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let result = a
        .boolean(b, operation, policy)
        .map_err(|e| error(Some(feature), e))?
        .ok_or_else(|| error(Some(feature), "Machine screw operation produced no solid"))?;
    if result.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
        return Err(error(
            Some(feature),
            "Machine screw must remain one connected solid",
        ));
    }
    Ok(result)
}
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
        ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            shank_radius_mm,
            head,
            drive,
            mut thread,
        } => {
            let frame = thread.frame;
            let shank_length = underhead_length_mm - thread.length_mm;
            if let ResolvedScrewDrive::HexSocket {
                across_flats_mm,
                depth_mm,
            } = &drive
                && (head.height_mm() - depth_mm < policy.minimum_feature_mm
                    || head.minimum_radius_mm() - across_flats_mm / 3_f64.sqrt()
                        < policy.minimum_feature_mm)
            {
                return Err(error(
                    Some(feature),
                    "Hex socket floor or corner wall is below native minimum feature size",
                ));
            }
            let circle = |radius_mm| {
                vec![CurvedEdge2d::Circle {
                    center: [0., 0.],
                    radius_mm,
                }]
            };
            let head_outer = match head {
                ResolvedScrewHead::SocketCap { radius_mm, .. } => circle(radius_mm),
                ResolvedScrewHead::ExternalHex {
                    across_flats_mm, ..
                } => hexagon(across_flats_mm),
            };
            let height = head.height_mm();
            let mut result = screw_prism(feature, "head", -height, height, head_outer, policy)?;
            if let ResolvedScrewDrive::HexSocket {
                across_flats_mm,
                depth_mm,
            } = drive
            {
                let socket = screw_prism(
                    feature,
                    "socket",
                    -height,
                    depth_mm,
                    hexagon(across_flats_mm),
                    policy,
                )?;
                result = screw_boolean(
                    feature,
                    &result,
                    &socket,
                    BooleanOperation::Subtract,
                    policy,
                )?;
            }
            if shank_length > 0. {
                let shank = screw_prism(
                    feature,
                    "shank",
                    0.,
                    shank_length,
                    circle(shank_radius_mm),
                    policy,
                )?;
                result = screw_boolean(feature, &result, &shank, BooleanOperation::Union, policy)?;
            }
            thread.frame = Frame {
                origin: [0., 0., shank_length],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                n: [0., 0., 1.],
            };
            let threaded = crate::threads::build_resolved(
                feature,
                *thread,
                ThreadMode::NewExternal,
                None,
                policy,
            )
            .map_err(|e| error(Some(feature), format!("Screw standalone thread: {e}")))?
            .with_feature_cap_roles(feature, "screw.thread/start", "screw.thread/end")
            .map_err(|e| error(Some(feature), e))?;
            result = screw_boolean(feature, &result, &threaded, BooleanOperation::Union, policy)
                .map_err(|e| error(Some(feature), format!("Screw thread fuse: {e}")))?;
            let topology = result
                .topology_snapshot(feature, 0)
                .map_err(|e| error(Some(feature), e))?;
            if topology.elements.iter().any(|e| {
                e.key.kind == multicad_core::references::TopologyKind::Face && e.origins.is_empty()
            }) {
                return Err(error(
                    Some(feature),
                    "Machine screw has incomplete native face ancestry",
                ));
            }
            result
                .transformed(RigidTransform {
                    rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.n[i]]),
                    translation_mm: frame.origin,
                })
                .map_err(|e| error(Some(feature), format!("Screw world transform: {e}")))
        }
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
