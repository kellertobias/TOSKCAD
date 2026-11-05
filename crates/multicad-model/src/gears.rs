//! Native involute bodies are derived from the canonical, resolved gear recipe.
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{GearDefinition, GearFamily, ResolvedGearGeometry},
    parameters::EvaluatedParameters,
};
use multicad_kernel::{
    GearSweep, GeometryPolicy, InvoluteGear, RigidTransform, Solid, SphericalBevelGear,
    StraightRack,
};

#[derive(Clone, Debug)]
pub(crate) enum NativeSpecification {
    Rotary(InvoluteGear),
    Rack(StraightRack),
    Bevel(SphericalBevelGear),
}

pub(crate) fn specification(
    feature: EntityId,
    geometry: &ResolvedGearGeometry,
) -> Result<NativeSpecification, ModelError> {
    let r = match geometry {
        ResolvedGearGeometry::Rotary(r) => r,
        ResolvedGearGeometry::Rack(r) => {
            return Ok(NativeSpecification::Rack(StraightRack {
                teeth: r.teeth,
                pitch_mm: r.pitch_mm,
                pitch_thickness_mm: r.physical_tooth_thickness_mm,
                pressure_angle_rad: r.pressure_angle_rad,
                addendum_mm: r.addendum_mm,
                dedendum_mm: r.dedendum_mm,
                root_fillet_radius_mm: r.root_fillet_radius_mm,
                backing_thickness_mm: r.backing_thickness_mm,
                face_width_mm: r.face_width_mm,
            }));
        }
        ResolvedGearGeometry::Bevel(r) => {
            return Ok(NativeSpecification::Bevel(SphericalBevelGear {
                teeth: r.teeth,
                pitch_cone_angle_rad: r.pitch_cone_angle_rad,
                base_cone_angle_rad: r.base_cone_angle_rad,
                tip_cone_angle_rad: r.tip_cone_angle_rad,
                root_cone_angle_rad: r.root_cone_angle_rad,
                outer_cone_distance_mm: r.outer_cone_distance_mm,
                face_width_mm: r.face_width_mm,
                bore_radius_mm: r.bore_radius_mm,
                half_base_tooth_angle_rad: r.half_base_angle_rad,
                root_fillet_radius_mm: r.root_fillet_radius_mm,
            }));
        }
    };
    let sweep = match r.family {
        GearFamily::Spur => GearSweep::Spur,
        GearFamily::InternalRing => GearSweep::InternalSpur {
            outer_radius_mm: r
                .outer_radius_mm
                .ok_or_else(|| error(Some(feature), "Internal ring outside diameter is missing"))?,
        },
        GearFamily::Helical => GearSweep::Helical,
        GearFamily::Herringbone => GearSweep::Chevron,
        GearFamily::DoubleHelical => GearSweep::GappedChevron {
            gap_mm: r.center_gap_mm,
        },
        _ => return Err(error(Some(feature), "Unsupported native gear family")),
    };
    Ok(NativeSpecification::Rotary(InvoluteGear {
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
    }))
}
pub(crate) fn build(
    feature: EntityId,
    source: &GearDefinition,
    parameters: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let r = source
        .parameters
        .resolve_geometry(parameters)
        .map_err(|e| error(Some(feature), e))?;
    let solid = match specification(feature, &r)? {
        NativeSpecification::Rotary(spec) => Solid::involute_gear_named(spec, feature, policy),
        NativeSpecification::Rack(spec) => Solid::straight_rack_named(spec, feature, policy),
        NativeSpecification::Bevel(spec) => {
            Solid::spherical_bevel_gear_named(spec, feature, policy)
        }
    };
    let frame = r.frame();
    solid
        .and_then(|solid| {
            solid.transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.n[i]]),
                translation_mm: frame.origin,
            })
        })
        .map_err(|e| error(Some(feature), e))
}
