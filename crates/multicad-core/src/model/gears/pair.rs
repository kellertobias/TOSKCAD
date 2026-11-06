//! Pair intent owns dimensional dependencies; assembly joints own occurrences.
use super::*;
use crate::{
    model::{JointAnchor, JointDefinition, JointKind, JointLimits, JointValues, RigidPose},
    references::ReferenceFrame,
};
use std::collections::BTreeSet;
pub const GEAR_PAIR_TYPE: &str = "model.gear_pair";
pub const GEAR_PAIR_SCHEMA: u32 = 1;
pub const GEAR_PAIR_CAPABILITY: &str = "model.gear_pair.1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearPairMember {
    A,
    B,
}
impl GearPairMember {
    pub fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearPairSpace {
    Local,
    Mating,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearPairParameters {
    pub gears: [EntityId; 2],
    pub frame: PrimitiveFrame,
    pub drive_angle: Dimension,
}
impl GearPairParameters {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.gears[0] == self.gears[1] || self.gears.iter().any(|id| id.is_nil()) {
            return Err(invalid("A gear pair needs two distinct gear features"));
        }
        self.frame.validate()?;
        check_dimension(&self.drive_angle, Unit::Degree)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(
            self.frame
                .origin
                .iter()
                .chain([&self.drive_angle])
                .collect(),
        );
        for (index, gear) in self.gears.iter().enumerate() {
            links.push(Link::new(*gear, format!("gear_pair/member/{index}")));
        }
        Ok(links)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearPairOutputs {
    pub assembly: EntityId,
    pub metadata: EntityId,
    pub instances: [EntityId; 2],
    pub joints: [EntityId; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearPairDefinition {
    pub parameters: GearPairParameters,
    pub outputs: GearPairOutputs,
}
impl GearPairDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.parameters.validate()?;
        let ids = [
            self.outputs.assembly,
            self.outputs.metadata,
            self.outputs.instances[0],
            self.outputs.instances[1],
            self.outputs.joints[0],
            self.outputs.joints[1],
        ];
        if ids
            .iter()
            .any(|id| id.is_nil() || self.parameters.gears.contains(id))
            || ids.into_iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(invalid(
                "Gear pair outputs must have distinct, non-nil identities",
            ));
        }
        Ok(Definition {
            type_id: GEAR_PAIR_TYPE.into(),
            schema_version: GEAR_PAIR_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != GEAR_PAIR_TYPE || definition.schema_version != GEAR_PAIR_SCHEMA {
            return Err(invalid("Unsupported gear pair provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.definition()?;
        Ok(source)
    }
}
fn anchor_is(
    anchor: &JointAnchor,
    pair: EntityId,
    member: GearPairMember,
    space: GearPairSpace,
) -> bool {
    matches!(anchor, JointAnchor::GearPairFrame { pair: p, member: m, space: s } if *p == pair && *m == member && *s == space)
}
pub fn read_gear_pair(
    document: &Document,
    pair: EntityId,
) -> Result<GearPairDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(pair).map(|e| &e.data)
    else {
        return Err(invalid("Choose a gear pair definition"));
    };
    let source = GearPairDefinition::from_definition(definition)?;
    let expected = source.parameters.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Gear pair source and canonical dependency links disagree",
        ));
    }
    let mut bodies = vec![];
    for gear in source.parameters.gears {
        let recipe = read_gear(document, gear)?;
        let Some(EntityData::Part { features, .. }) =
            document.get(recipe.output.body).map(|e| &e.data)
        else {
            return Err(invalid("Paired gear Part is missing"));
        };
        if features.last() != Some(&gear)
            || !matches!(
                document.get(gear).map(|e| &e.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        {
            return Err(invalid(
                "Paired Parts must retain their unsuppressed gear generator as the current history head; downstream nonperiodic geometry needs separate verification",
            ));
        }
        bodies.push(recipe.output.body);
    }
    if bodies[0] == bodies[1] {
        return Err(invalid("A pair needs two independent gear Parts"));
    }
    let Some(EntityData::Assembly { instances }) =
        document.get(source.outputs.assembly).map(|e| &e.data)
    else {
        return Err(invalid("Gear pair output Assembly is missing"));
    };
    if instances != &source.outputs.instances {
        return Err(invalid(
            "Gear pair Assembly members disagree with its outputs",
        ));
    }
    let metadata = crate::model::read_assembly(document, source.outputs.metadata)?;
    if metadata.assembly != source.outputs.assembly
        || metadata
            .definition_parts
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            != bodies.iter().copied().collect()
    {
        return Err(invalid(
            "Gear pair definition-only Part metadata disagrees with its members",
        ));
    }
    for member in [GearPairMember::A, GearPairMember::B] {
        let index = member.index();
        let Some(EntityData::Instance {
            component,
            placement_source,
            ..
        }) = document
            .get(source.outputs.instances[index])
            .map(|e| &e.data)
        else {
            return Err(invalid("Gear pair member Instance is missing"));
        };
        if *component != bodies[index] || *placement_source != Some(source.outputs.joints[index]) {
            return Err(invalid(
                "Gear pair member has changed its Part or incoming relationship",
            ));
        }
        let Some(EntityData::Extension { definition, .. }) =
            document.get(source.outputs.joints[index]).map(|e| &e.data)
        else {
            return Err(invalid("Gear pair relationship is missing"));
        };
        // Decode only: read_joint checks this pair in turn, so calling it here
        // would recurse. Its normal document pass validates exact joint links.
        let joint = JointDefinition::from_definition(definition)?;
        if joint.owner != source.outputs.instances[index]
            || joint.component != bodies[index]
            || joint.parameters.kind != JointKind::Fixed
            || joint.parameters.parent.is_some()
            || joint.parameters.values != JointValues::default()
            || joint.parameters.limits != JointLimits::default()
            || !anchor_is(&joint.parameters.moving, pair, member, GearPairSpace::Local)
            || !anchor_is(
                &joint.parameters.target,
                pair,
                member,
                GearPairSpace::Mating,
            )
            || joint.base
                != (crate::model::PlacementBase::Literal {
                    pose: RigidPose::default(),
                })
        {
            return Err(invalid(
                "Gear pair relationships must retain their pair-controlled Fixed anchors and zero motion",
            ));
        }
    }
    Ok(source)
}
#[derive(Clone, Debug)]
pub struct ResolvedGearPair {
    pub gears: [ResolvedGearGeometry; 2],
    pub rack_travel: Option<ResolvedRackTravel>,
    pub bevel: Option<ResolvedBevelPair>,
    pub worm: Option<ResolvedWormPair>,
    pub center_distance_mm: f64,
    pub phases_rad: [f64; 2],
    pub mating: [ReferenceFrame; 2],
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.)
}
pub fn resolve_gear_pair_parameters(
    document: &Document,
    evaluated: &EvaluatedParameters,
    parameters: &GearPairParameters,
) -> Result<ResolvedGearPair, DocumentError> {
    let a = read_gear(document, parameters.gears[0])?;
    let b = read_gear(document, parameters.gears[1])?;
    parameters.resolve_recipes([&a.parameters, &b.parameters], evaluated)
}
impl GearPairParameters {
    /// Shared source-only validation for proposed UI/MCP recipes. A successful
    /// result still requires the complete native candidate gate before Apply.
    pub fn resolve_recipes(
        &self,
        recipes: [&GearParameters; 2],
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedGearPair, DocumentError> {
        self.validate()?;
        super::bevel::validate_bevel_pair_sources(recipes)?;
        super::worm::validate_worm_pair_sources(recipes)?;
        super::coverage::validate_shifted_pair_sources(recipes)?;
        let gears = [
            recipes[0].resolve_geometry(evaluated)?,
            recipes[1].resolve_geometry(evaluated)?,
        ];
        if gears.iter().any(|gear| gear.family() == GearFamily::Worm) {
            return self.resolve_worm_pair(gears, evaluated);
        }
        if gears.iter().any(|gear| gear.family() == GearFamily::Bevel) {
            return self.resolve_bevel_pair(gears, evaluated);
        }
        if gears.iter().any(|gear| gear.family() == GearFamily::Rack) {
            return self.resolve_rack_pair(gears, evaluated);
        }
        let a = gears[0].rotary().expect("rotary pair member");
        let b = gears[1].rotary().expect("rotary pair member");
        let internal = matches!(
            (a.family, b.family),
            (GearFamily::Spur, GearFamily::InternalRing)
                | (GearFamily::InternalRing, GearFamily::Spur)
        );
        if (!internal && a.family != b.family)
            || !close(a.transverse_module_mm, b.transverse_module_mm)
            || !close(
                a.transverse_pressure_angle_rad,
                b.transverse_pressure_angle_rad,
            )
            || !close(a.face_width_mm, b.face_width_mm)
            || !close(a.center_gap_mm, b.center_gap_mm)
            || !close(a.helix_angle_rad, b.helix_angle_rad)
            || (!internal
                && a.family != GearFamily::Spur
                && a.twist_rad_per_mm * b.twist_rad_per_mm >= 0.)
        {
            return Err(invalid(
                "This external parallel-axis pair needs matching family, transverse module/pressure angle, face width and center gap, plus equal helix magnitude and opposite hands",
            ));
        }
        if internal {
            let (pinion, ring) = if a.family == GearFamily::Spur {
                (a, b)
            } else {
                (b, a)
            };
            if pinion.teeth != 24
                || (pinion.transverse_pressure_angle_rad.to_degrees() - 20.).abs() > 1e-9
            {
                return Err(invalid(
                    "Internal pairs currently need a 24-tooth, unshifted 20-degree spur pinion and a 48/60-tooth ring",
                ));
            }
            // KHK internal-gear equations 4.4 and 4.11–4.12, plus its
            // explicit standard 20-degree tooth-difference >9 condition.
            let aa1 = (pinion.base_radius_mm / pinion.tip_radius_mm).acos();
            let aa2 = (ring.base_radius_mm / ring.tip_radius_mm).acos();
            let alpha = ring.transverse_pressure_angle_rad;
            let ratio = f64::from(pinion.teeth) / f64::from(ring.teeth);
            let inv = |angle: f64| angle.tan() - angle;
            let theta1 = ((1. - (aa1.cos() / aa2.cos()).powi(2)) / (1. - ratio.powi(2)))
                .sqrt()
                .asin();
            let theta2 = (((aa2.cos() / aa1.cos()).powi(2) - 1.) / (ratio.recip().powi(2) - 1.))
                .sqrt()
                .asin();
            if !theta1.is_finite()
                || !theta2.is_finite()
                || ring.teeth <= pinion.teeth + 9
                || ratio < 1. - aa2.tan() / alpha.tan()
                || theta1 + inv(aa1) - inv(alpha) < (theta2 + inv(aa2) - inv(alpha)) / ratio
            {
                return Err(invalid(
                    "Internal gear pair violates the bounded KHK involute, trochoid or trimming interference policy",
                ));
            }
        }
        let drive = number(&self.drive_angle, Unit::Radian, evaluated)?;
        if drive.abs() > 2000. * PI {
            return Err(invalid(
                "Gear pair drive angle exceeds the bounded 1000-turn coordinate",
            ));
        }
        let shifted = a.profile_shift_coefficient != 0.
            || b.profile_shift_coefficient != 0.
            || recipes.iter().any(|p| p.shift_mate.is_some());
        if shifted && recipes.iter().any(|p| p.shift_mate.is_none()) {
            return Err(invalid(
                "Shifted pairs require retained matched mate contexts on both members",
            ));
        }
        if internal && shifted {
            return Err(invalid("Shifted internal pairs are unavailable"));
        }
        let center = if shifted {
            shifted_spur_pair_geometry(a, b)?.center_distance_mm
        } else if internal {
            (a.pitch_radius_mm - b.pitch_radius_mm).abs()
        } else {
            a.pitch_radius_mm + b.pitch_radius_mm
        };
        let phases = if internal {
            [
                drive,
                if a.family == GearFamily::InternalRing {
                    PI / f64::from(b.teeth) + drive * f64::from(a.teeth) / f64::from(b.teeth)
                } else {
                    PI * (1. - f64::from(a.teeth) / f64::from(b.teeth)) - PI / f64::from(b.teeth)
                        + drive * f64::from(a.teeth) / f64::from(b.teeth)
                },
            ]
        } else {
            [
                drive,
                PI + PI / f64::from(b.teeth) - drive * f64::from(a.teeth) / f64::from(b.teeth),
            ]
        };
        let base = self.frame.resolve(evaluated)?;
        let mating = std::array::from_fn(|index| {
            let (sin, cos) = phases[index].rem_euclid(2. * PI).sin_cos();
            ReferenceFrame {
                origin_mm: std::array::from_fn(|i| {
                    base.origin[i] + if index == 1 { base.u[i] * center } else { 0. }
                }),
                u: std::array::from_fn(|i| base.u[i] * cos + base.v[i] * sin),
                v: std::array::from_fn(|i| -base.u[i] * sin + base.v[i] * cos),
            }
        });
        Ok(ResolvedGearPair {
            rack_travel: None,
            bevel: None,
            worm: None,
            gears,
            center_distance_mm: center,
            phases_rad: phases,
            mating,
        })
    }
}
pub fn resolve_gear_pair_anchor(
    document: &Document,
    evaluated: &EvaluatedParameters,
    pair: EntityId,
    member: GearPairMember,
    space: GearPairSpace,
) -> Result<ReferenceFrame, DocumentError> {
    let source = read_gear_pair(document, pair)?;
    let resolved = resolve_gear_pair_parameters(document, evaluated, &source.parameters)?;
    let index = member.index();
    Ok(match space {
        GearPairSpace::Mating => resolved.mating[index],
        GearPairSpace::Local => {
            let f = resolved.gears[index].frame();
            ReferenceFrame {
                origin_mm: f.origin,
                u: f.u,
                v: f.v,
            }
        }
    })
}
pub(crate) fn validate_pairs(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        if matches!(&entity.data,EntityData::Extension{definition,..} if definition.type_id == GEAR_PAIR_TYPE)
        {
            read_gear_pair(document, entity.id)?;
        }
    }
    Ok(())
}
