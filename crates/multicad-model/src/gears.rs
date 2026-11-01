//! Native involute bodies are derived from the canonical, resolved gear recipe.
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{GearDefinition, GearFamily},
    parameters::EvaluatedParameters,
};
use multicad_kernel::{GearSweep, GeometryPolicy, InvoluteGear, RigidTransform, Solid};

pub(crate) fn build(
    feature: EntityId,
    source: &GearDefinition,
    parameters: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let r = source
        .parameters
        .resolve(parameters)
        .map_err(|e| error(Some(feature), e))?;
    let sweep = match r.family {
        GearFamily::Spur => GearSweep::Spur,
        GearFamily::Helical => GearSweep::Helical,
        GearFamily::Herringbone => GearSweep::Chevron,
        GearFamily::DoubleHelical => GearSweep::GappedChevron {
            gap_mm: r.center_gap_mm,
        },
        _ => return Err(error(Some(feature), "Unsupported native gear family")),
    };
    Solid::involute_gear_named(
        InvoluteGear {
            teeth: r.teeth,
            pitch_radius_mm: r.pitch_radius_mm,
            base_radius_mm: r.base_radius_mm,
            tip_radius_mm: r.tip_radius_mm,
            root_radius_mm: r.root_radius_mm,
            root_fillet_radius_mm: r.root_fillet_radius_mm,
            half_base_angle_rad: r.half_base_angle_rad,
            face_width_mm: r.face_width_mm,
            bore_radius_mm: r.bore_radius_mm,
            twist_rad_per_mm: r.twist_rad_per_mm,
            sweep,
        },
        feature,
        policy,
    )
    .and_then(|solid| {
        solid.transformed(RigidTransform {
            rotation: std::array::from_fn(|i| [r.frame.u[i], r.frame.v[i], r.frame.n[i]]),
            translation_mm: r.frame.origin,
        })
    })
    .map_err(|e| error(Some(feature), e))
}
