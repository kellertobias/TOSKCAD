//! Canonical involute gear intent. Native receipts still gate geometry Apply.
mod bevel;
mod pair;
mod pair_prepare;
mod prepare;
mod profiles;
mod rack;
mod worm;
use super::{
    BodyOutput, PrimitiveFrame, angle,
    common::*,
    length,
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    sketch::Frame,
};
pub use bevel::*;
pub use pair::*;
pub use pair_prepare::*;
pub use prepare::*;
pub use profiles::*;
pub use rack::*;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
pub use worm::*;

pub const GEAR_TYPE: &str = "model.gear";
pub const GEAR_SCHEMA: u32 = 1;
pub const GEAR_CAPABILITY: &str = "model.gear.1";
pub const INTERNAL_GEAR_CAPABILITY: &str = "model.gear.internal.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearModuleSystem {
    Normal,
    Transverse,
    Axial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearHand {
    None,
    Right,
    Left,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearParameters {
    pub family: GearFamily,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rack: Option<RackDimensions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<BevelDimensions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worm: Option<WormDimensions>,
    pub module_system: GearModuleSystem,
    pub module: Dimension,
    pub teeth: Dimension,
    /// Normal or transverse according to module_system; Worm always uses normal pressure with axial module.
    pub pressure_angle: Dimension,
    pub face_width: Dimension,
    /// Positive magnitude; hand is stored separately. Zero for Spur.
    pub helix_angle: Dimension,
    pub hand: GearHand,
    pub center_gap: Dimension,
    pub bore_diameter: Dimension,
    /// Internal-ring outside diameter. Absent for legacy external recipes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_diameter: Option<Dimension>,
    /// Tangential tooth-thickness reduction per gear at the reference circle.
    pub backlash: Dimension,
    /// Additional tangential thinning, independent of nominal design backlash.
    pub fabrication_allowance: Dimension,
    /// Circular fillet radius divided by the selected system's basic module.
    /// This is direct-designed geometry, not a generated cutter trochoid.
    pub root_fillet_ratio: Dimension,
    pub minimum_wall: Dimension,
    /// Only unshifted full-depth profiles are implemented.
    pub profile_shift: Dimension,
    pub frame: PrimitiveFrame,
}
impl Default for GearParameters {
    fn default() -> Self {
        Self {
            family: GearFamily::Spur,
            rack: None,
            bevel: None,
            worm: None,
            module_system: GearModuleSystem::Normal,
            module: length(2.),
            teeth: Dimension::Literal {
                value: 24.,
                unit: "count".into(),
            },
            pressure_angle: angle(20.),
            face_width: length(10.),
            helix_angle: angle(0.),
            hand: GearHand::None,
            center_gap: length(0.),
            bore_diameter: length(8.),
            outer_diameter: None,
            backlash: length(0.05),
            fabrication_allowance: length(0.),
            root_fillet_ratio: ratio(0.2),
            minimum_wall: length(1.),
            profile_shift: ratio(0.),
            frame: Default::default(),
        }
    }
}
fn ratio(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "ratio".into(),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGear {
    pub family: GearFamily,
    pub teeth: u32,
    pub normal_module_mm: f64,
    pub transverse_module_mm: f64,
    pub normal_pressure_angle_rad: f64,
    pub transverse_pressure_angle_rad: f64,
    pub helix_angle_rad: f64,
    pub pitch_radius_mm: f64,
    pub base_radius_mm: f64,
    pub tip_radius_mm: f64,
    pub root_radius_mm: f64,
    pub root_fillet_radius_mm: f64,
    pub half_base_angle_rad: f64,
    pub nominal_tooth_thickness_mm: f64,
    pub design_tooth_thickness_mm: f64,
    pub physical_tooth_thickness_mm: f64,
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
    pub outer_radius_mm: Option<f64>,
    pub center_gap_mm: f64,
    pub twist_rad_per_mm: f64,
    pub minimum_unshifted_teeth: u32,
    pub frame: Frame,
}
impl GearParameters {
    fn dimensions(&self) -> Vec<(&Dimension, Unit)> {
        let mut dimensions = vec![
            (&self.module, Unit::Millimeter),
            (&self.teeth, Unit::Count),
            (&self.pressure_angle, Unit::Degree),
            (&self.face_width, Unit::Millimeter),
            (&self.helix_angle, Unit::Degree),
            (&self.center_gap, Unit::Millimeter),
            (&self.bore_diameter, Unit::Millimeter),
            (&self.backlash, Unit::Millimeter),
            (&self.fabrication_allowance, Unit::Millimeter),
            (&self.root_fillet_ratio, Unit::Ratio),
            (&self.minimum_wall, Unit::Millimeter),
            (&self.profile_shift, Unit::Ratio),
        ];
        if let Some(outer) = &self.outer_diameter {
            dimensions.push((outer, Unit::Millimeter));
        }
        if let Some(rack) = &self.rack {
            dimensions.push((&rack.backing_thickness, Unit::Millimeter));
        }
        if let Some(bevel) = &self.bevel {
            dimensions.push((&bevel.mate_teeth, Unit::Count));
            dimensions.push((&bevel.shaft_angle, Unit::Degree));
        }
        if let Some(worm) = &self.worm {
            dimensions.push((&worm.mate_teeth, Unit::Count));
            dimensions.push((&worm.diameter_quotient, Unit::Ratio));
        }
        dimensions.extend(self.frame.origin.iter().map(|d| (d, Unit::Millimeter)));
        dimensions
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame.validate()?;
        for (dimension, unit) in self.dimensions() {
            check_dimension(dimension, unit)?;
        }
        if !self.family.supported() {
            return Err(invalid(format!(
                "{} is not implemented",
                self.family.label()
            )));
        }
        if (self.family == GearFamily::Worm) != self.worm.is_some() {
            return Err(invalid(
                "Worm gearing requires declared role/generating dimensions; other families must omit them",
            ));
        }
        if (self.module_system == GearModuleSystem::Axial) != (self.family == GearFamily::Worm) {
            return Err(invalid(
                "Axial module is required exclusively for worm gearing",
            ));
        }
        if self
            .worm
            .as_ref()
            .is_some_and(|w| !w.tooth_system.supported())
        {
            return Err(invalid(
                "Only bounded ZA axial-straight worm gearing is implemented; ZI, ZN and ZK are unavailable",
            ));
        }
        if (self.family == GearFamily::Bevel) != self.bevel.is_some() {
            return Err(invalid(
                "Bevel recipes require declared tooth/cone dimensions; other families must omit them",
            ));
        }
        if self
            .bevel
            .as_ref()
            .is_some_and(|bevel| !bevel.tooth_system.supported())
        {
            return Err(invalid(
                "Only declared spherical-involute straight bevels are available; Gleason/octoid, spiral and zerol are unsupported",
            ));
        }
        if (self.family == GearFamily::Rack) != self.rack.is_some() {
            return Err(invalid(
                "Rack recipes require backing dimensions; rotary gear recipes must omit rack dimensions",
            ));
        }
        if (self.family == GearFamily::InternalRing) != self.outer_diameter.is_some() {
            return Err(invalid(
                "Internal rings require an outside diameter; external gear recipes must omit it",
            ));
        }
        Ok(())
    }
    pub fn resolve(&self, parameters: &EvaluatedParameters) -> Result<ResolvedGear, DocumentError> {
        self.validate()?;
        if self.family == GearFamily::Worm {
            return Err(invalid(
                "Worm gearing has distinct generated geometry; use resolve_geometry",
            ));
        }
        if self.family == GearFamily::Rack {
            return Err(invalid(
                "Rack geometry has no rotary radii; use resolve_geometry",
            ));
        }
        if self.family == GearFamily::Bevel {
            return Err(invalid(
                "Bevel geometry has no cylindrical rotary radii; use resolve_geometry",
            ));
        }
        let module = number(&self.module, Unit::Millimeter, parameters)?;
        let count = number(&self.teeth, Unit::Count, parameters)?;
        let pressure = number(&self.pressure_angle, Unit::Radian, parameters)?;
        let beta = number(&self.helix_angle, Unit::Radian, parameters)?;
        let width = number(&self.face_width, Unit::Millimeter, parameters)?;
        let gap = number(&self.center_gap, Unit::Millimeter, parameters)?;
        let bore = number(&self.bore_diameter, Unit::Millimeter, parameters)?;
        let backlash = number(&self.backlash, Unit::Millimeter, parameters)?;
        let allowance = number(&self.fabrication_allowance, Unit::Millimeter, parameters)?;
        let fillet = number(&self.root_fillet_ratio, Unit::Ratio, parameters)?;
        let wall = number(&self.minimum_wall, Unit::Millimeter, parameters)?;
        let shift = number(&self.profile_shift, Unit::Ratio, parameters)?;
        if module < 0.01
            || !(4. ..=128.).contains(&count)
            || count.fract().abs() > 1e-9
            || !(14.5_f64.to_radians()..=35_f64.to_radians()).contains(&pressure)
            || !(0. ..=45_f64.to_radians()).contains(&beta)
            || width <= 0.
            || gap < 0.
            || bore < 0.
            || backlash < 0.
            || allowance < 0.
            || wall <= 0.
            || !(0.01..=0.2).contains(&fillet)
        {
            return Err(invalid(
                "Gear needs module ≥0.01 mm, 4–128 whole teeth, 14.5–35 degree pressure angle, 0–45 degree helix magnitude, positive width/wall, nonnegative bore/clearance and root-fillet ratio 0.01–0.2",
            ));
        }
        if shift != 0. {
            return Err(invalid(
                "Nonzero gear profile shift is not implemented; select an unshifted profile",
            ));
        }
        if matches!(self.family, GearFamily::Spur | GearFamily::InternalRing) {
            if beta != 0. || self.hand != GearHand::None {
                return Err(invalid(
                    "Spur and internal-ring gears need zero helix angle and hand none",
                ));
            }
        } else if beta <= 0. || self.hand == GearHand::None {
            return Err(invalid(
                "Helical and chevron gears need a positive helix angle and explicit hand",
            ));
        }
        if self.family == GearFamily::DoubleHelical {
            if gap <= 0. || width - gap < 0.02 {
                return Err(invalid(
                    "Gapped double-helical gears need a positive center gap and two tooth bands at least 0.01 mm wide",
                ));
            }
        } else if gap != 0. {
            return Err(invalid(
                "Only gapped double-helical gears may have a center gap; herringbone teeth meet continuously",
            ));
        }
        let (mn, mt, an, at) = match self.module_system {
            GearModuleSystem::Axial => {
                return Err(invalid(
                    "Axial module belongs only to resolved worm geometry",
                ));
            }
            GearModuleSystem::Normal => (
                module,
                module / beta.cos(),
                pressure,
                (pressure.tan() / beta.cos()).atan(),
            ),
            GearModuleSystem::Transverse => (
                module * beta.cos(),
                module,
                (pressure.tan() * beta.cos()).atan(),
                pressure,
            ),
        };
        // Conservative unshifted policy: no virtual-tooth helix credit or
        // generated undercut relief is assumed by this direct-design model.
        let minimum = (2. / an.sin().powi(2)).ceil() as u32;
        if count < f64::from(minimum) {
            return Err(invalid(format!(
                "This unshifted profile needs at least {minimum} teeth under the conservative normal-angle undercut policy; profile-shifted/undercut geometry is unavailable"
            )));
        }
        let teeth = count as u32;
        let internal = self.family == GearFamily::InternalRing;
        if internal
            && (!matches!(teeth, 48 | 60)
                || (pressure.to_degrees() - 20.).abs() > 1e-9
                || bore != 0.)
        {
            return Err(invalid(
                "Internal rings currently support unshifted 20-degree spur profiles with 48 or 60 teeth; bore_diameter must be zero because the toothed opening is derived from module and teeth",
            ));
        }
        let pitch = mt * count / 2.;
        let base = pitch * at.cos();
        let tip = pitch + if internal { -module } else { module };
        let root = pitch
            + if internal {
                1.25 * module
            } else {
                -1.25 * module
            };
        let nominal = PI * mt / 2.;
        let design = nominal - backlash;
        let physical = design - allowance;
        let half_base = physical / (2. * pitch)
            + if internal {
                at - at.tan()
            } else {
                at.tan() - at
            };
        let ta = (tip.powi(2) / base.powi(2) - 1.).sqrt();
        let root_fillet = fillet * module;
        let outer = self
            .outer_diameter
            .as_ref()
            .map(|d| number(d, Unit::Millimeter, parameters).map(|v| v / 2.))
            .transpose()?;
        if internal {
            let outer =
                outer.ok_or_else(|| invalid("Internal rings need an explicit outside diameter"))?;
            if tip <= base
                || outer - root < wall
                || physical <= 0.
                || half_base + ta - ta.atan() <= 1e-5 / tip
                || root - root_fillet <= base
            {
                return Err(invalid(
                    "Internal ring needs involute tip above base circle, positive teeth and sufficient outside rim beyond its roots",
                ));
            }
        } else if root - bore / 2. < wall
            || physical <= 0.
            || half_base - (ta - ta.atan()) <= 1e-5 / tip
        {
            return Err(invalid(
                "Gear bore leaves insufficient root wall, or clearance leaves a pointed/missing tooth",
            ));
        }
        let sign = if self.hand == GearHand::Left { -1. } else { 1. };
        let twist = sign * beta.tan() / pitch;
        if (twist * width).abs() > PI {
            return Err(invalid(
                "Gear face width and helix angle exceed the bounded half-turn sweep",
            ));
        }
        Ok(ResolvedGear {
            family: self.family,
            teeth,
            normal_module_mm: mn,
            transverse_module_mm: mt,
            normal_pressure_angle_rad: an,
            transverse_pressure_angle_rad: at,
            helix_angle_rad: beta,
            pitch_radius_mm: pitch,
            base_radius_mm: base,
            tip_radius_mm: tip,
            root_radius_mm: root,
            root_fillet_radius_mm: root_fillet,
            half_base_angle_rad: half_base,
            nominal_tooth_thickness_mm: nominal,
            design_tooth_thickness_mm: design,
            physical_tooth_thickness_mm: physical,
            face_width_mm: width,
            bore_radius_mm: bore / 2.,
            outer_radius_mm: outer,
            center_gap_mm: gap,
            twist_rad_per_mm: twist,
            minimum_unshifted_teeth: minimum,
            frame: self.frame.resolve(parameters)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearDefinition {
    pub output: BodyOutput,
    pub parameters: GearParameters,
}
impl GearDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.parameters.validate()?;
        if self.output.body.is_nil() {
            return Err(invalid("Gear output identity is nil"));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: GEAR_TYPE.into(),
            schema_version: GEAR_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != GEAR_TYPE || definition.schema_version != GEAR_SCHEMA {
            return Err(invalid("Unsupported gear provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        Ok(parameter_links(
            self.parameters
                .dimensions()
                .into_iter()
                .map(|(d, _)| d)
                .collect(),
        ))
    }
}
pub fn read_gear(document: &Document, feature: EntityId) -> Result<GearDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a gear feature"));
    };
    let source = GearDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Gear source and canonical dependency links disagree",
        ));
    }
    for link in inputs {
        if document
            .get(link.target)
            .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
        {
            return Err(invalid("Gear dependency must be a Parameter"));
        }
    }
    if let Some(part) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &part.data else {
            return Err(invalid("Gear output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Gear output Part does not contain its producer"));
        }
    }
    Ok(source)
}
