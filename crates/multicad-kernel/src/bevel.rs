//! Spherical-involute straight bevel geometry with actual conical back caps.
//! Tooth-system and mating conventions belong to the canonical model.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyKey, TopologyKind},
};
use std::{collections::BTreeMap, f64::consts::FRAC_PI_2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphericalBevelGear {
    pub teeth: u32,
    pub pitch_cone_angle_rad: f64,
    pub base_cone_angle_rad: f64,
    pub tip_cone_angle_rad: f64,
    pub root_cone_angle_rad: f64,
    pub outer_cone_distance_mm: f64,
    /// Distance along the pitch generator, not an axial extrusion height.
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
    pub half_base_tooth_angle_rad: f64,
    /// Small-circle radius on the outer reference sphere; explicitly custom.
    pub root_fillet_radius_mm: f64,
}
fn failure(kind: GeometryErrorKind, message: impl ToString) -> GeometryError {
    GeometryError {
        operation: "spherical bevel gear",
        kind,
        message: message.to_string(),
    }
}
impl SphericalBevelGear {
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::BevelData, GeometryError> {
        policy.validate()?;
        for value in [
            self.outer_cone_distance_mm,
            self.face_width_mm,
            self.root_fillet_radius_mm,
        ] {
            policy.length(value)?;
        }
        let angles = [
            self.pitch_cone_angle_rad,
            self.base_cone_angle_rad,
            self.tip_cone_angle_rad,
            self.root_cone_angle_rad,
            self.half_base_tooth_angle_rad,
        ];
        if !(4..=64).contains(&self.teeth)
            || angles.iter().any(|a| !a.is_finite())
            || self.root_cone_angle_rad <= 0.
            || self.base_cone_angle_rad <= 0.
            || self.base_cone_angle_rad >= self.pitch_cone_angle_rad
            || self.root_cone_angle_rad >= self.pitch_cone_angle_rad
            || self.pitch_cone_angle_rad >= self.tip_cone_angle_rad
            || self.tip_cone_angle_rad >= FRAC_PI_2
            || self.face_width_mm >= self.outer_cone_distance_mm
            || self.root_fillet_radius_mm >= self.outer_cone_distance_mm
            || self.half_base_tooth_angle_rad <= 0.
            || self.half_base_tooth_angle_rad >= FRAC_PI_2
            || !self.bore_radius_mm.is_finite()
            || self.bore_radius_mm < 0.
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Bevel needs 4–64 teeth, acute ordered root/base/pitch/tip cones, positive inner cone distance, finite custom root and a resolvable tooth",
            ));
        }
        let root_toe = (self.outer_cone_distance_mm - self.face_width_mm)
            * self.root_cone_angle_rad.sin()
            / (self.root_cone_angle_rad - self.pitch_cone_angle_rad).cos();
        if root_toe - self.bore_radius_mm < policy.minimum_feature_mm {
            return Err(failure(
                GeometryErrorKind::Input,
                "Bevel bore consumes the minimum toe root wall",
            ));
        }
        let radius = self.outer_cone_distance_mm * self.tip_cone_angle_rad.sin()
            / (self.tip_cone_angle_rad - self.pitch_cone_angle_rad).cos();
        policy.point([
            radius,
            radius,
            self.outer_cone_distance_mm / self.pitch_cone_angle_rad.cos(),
        ])?;
        Ok(ffi::BevelData {
            teeth: self.teeth,
            pitch: self.pitch_cone_angle_rad,
            base: self.base_cone_angle_rad,
            tip: self.tip_cone_angle_rad,
            root: self.root_cone_angle_rad,
            distance: self.outer_cone_distance_mm,
            width: self.face_width_mm,
            bore: self.bore_radius_mm,
            half_base: self.half_base_tooth_angle_rad,
            fillet: self.root_fillet_radius_mm,
            tolerance: policy.linear_tolerance_mm,
            minimum: policy.minimum_feature_mm,
        })
    }
}
impl Solid {
    pub fn spherical_bevel_gear_named(
        spec: SphericalBevelGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Bevel feature identity must not be nil",
            ));
        }
        let data = spec.checked(policy)?;
        let native =
            ffi::spherical_bevel_gear(&data).map_err(|e| failure(GeometryErrorKind::Native, e))?;
        let mut solid = Self::adopt(native, policy, "spherical bevel gear")?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Bevel must retain one connected solid",
            ));
        }
        solid.name_bevel(feature, spec.teeth)?;
        Ok(solid)
    }
    fn name_bevel(&mut self, feature: EntityId, teeth: u32) -> Result<(), GeometryError> {
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(feature, 0)?
            .elements
            .into_iter()
            .map(|e| (e.key, e))
            .collect();
        let mut caps = [false; 2];
        for seed in ffi::seeds(self.occt()?).map_err(|e| failure(GeometryErrorKind::Native, e))? {
            let role = match seed.role {
                0 | 1 if seed.source_index < 2 * teeth => {
                    caps[seed.role as usize] = true;
                    format!(
                        "bevel/{}/sector/{}/{}",
                        if seed.role == 0 { "heel" } else { "toe" },
                        seed.source_index / 2,
                        if seed.source_index % 2 == 0 {
                            "outer"
                        } else {
                            "core"
                        }
                    )
                }
                2 => "bevel/bore".into(),
                6 if seed.source_index / 8 < teeth => format!(
                    "bevel/tooth/{}/{}",
                    seed.source_index / 8,
                    [
                        "fillet.left",
                        "meridian.left",
                        "spherical_involute.left",
                        "tip",
                        "spherical_involute.right",
                        "meridian.right",
                        "fillet.right",
                        "root"
                    ][seed.source_index as usize % 8]
                ),
                _ => {
                    return Err(failure(
                        GeometryErrorKind::InvalidResult,
                        "Unknown spherical bevel face source",
                    ));
                }
            };
            let element = elements
                .get_mut(&TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: seed.face,
                })
                .ok_or_else(|| {
                    failure(
                        GeometryErrorKind::InvalidResult,
                        "Bevel source names an absent face",
                    )
                })?;
            element.origins.insert(Origin {
                source: feature,
                role,
            });
        }
        if !caps.into_iter().all(|v| v)
            || elements
                .values()
                .any(|e| e.key.kind == TopologyKind::Face && e.origins.is_empty())
        {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Bevel needs both conical caps and complete native face ancestry",
            ));
        }
        let nodes =
            ffi::topology(self.occt()?).map_err(|e| failure(GeometryErrorKind::Native, e))?;
        self.finish_provenance(elements, &nodes)
    }
}
