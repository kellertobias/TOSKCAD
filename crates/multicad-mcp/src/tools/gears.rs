//! Gear tools retain canonical source and compile a candidate before committing.
use super::{
    NoArgs, Plane, Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    model::{
        self, BevelToothSystem, GearFamily, GearHand, GearModuleSystem, GearParameters,
        GearPitchInput, WormMember, WormToothSystem,
    },
    parameters::{BoundExpression, EvaluatedParameters, Unit},
    sketch::Datum,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

macro_rules! argument_enum {
    ($arg:ident, $core:ident, [$($variant:ident),+]) => {
        #[derive(Clone, Copy, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $arg { $($variant),+ }
        impl From<$arg> for $core {
            fn from(value: $arg) -> Self { match value { $($arg::$variant => Self::$variant),+ } }
        }
    };
}
argument_enum!(
    GearFamilyArg,
    GearFamily,
    [
        Spur,
        Helical,
        Herringbone,
        DoubleHelical,
        InternalRing,
        Rack,
        Bevel,
        Worm
    ]
);
argument_enum!(GearHandArg, GearHand, [None, Right, Left]);
argument_enum!(
    BevelToothSystemArg,
    BevelToothSystem,
    [SphericalInvoluteStraight, GleasonOctoid, Spiral, Zerol]
);
argument_enum!(
    GearModuleSystemArg,
    GearModuleSystem,
    [Normal, Transverse, Axial]
);
argument_enum!(WormMemberArg, WormMember, [Worm, Wheel]);
argument_enum!(
    WormToothSystemArg,
    WormToothSystem,
    [ZaAxialStraight, Zi, Zn, Zk]
);

argument_enum!(GearPitchInputArg, GearPitchInput, [Module, DiametralPitch]);

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GearScalarArg {
    pub unit: String,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub expression: Option<String>,
}
impl GearScalarArg {
    pub(super) fn dimension(
        &self,
        document: &Document,
        evaluated: &EvaluatedParameters,
        expected: Unit,
    ) -> Result<Dimension, ToolError> {
        let unit = Unit::parse(&self.unit)
            .filter(|u| u.dimension() == expected.dimension())
            .ok_or_else(|| {
                ToolError::invalid(format!(
                    "Gear scalar needs units compatible with {}",
                    expected.symbol()
                ))
            })?;
        match (self.value, &self.expression) {
            (Some(value), None) if value.is_finite() => Ok(Dimension::Literal {
                value,
                unit: self.unit.clone(),
            }),
            (None, Some(text)) => {
                let expression = BoundExpression::bind(document, text, unit.symbol(), None)?;
                expression
                    .evaluate(evaluated)?
                    .in_unit(expected)
                    .ok_or_else(|| ToolError::invalid("Gear expression has incompatible units"))?;
                Ok(Dimension::Expression { expression })
            }
            _ => Err(ToolError::invalid(
                "Give exactly one finite gear scalar value or expression",
            )),
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GearArgs {
    pub expected_revision: u64,
    /// Existing feature for an edit. Omitted recipe fields retain saved values.
    #[serde(default)]
    pub gear: Option<String>,
    /// Required for creation; omit for edits.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub recipe: GearRecipeArgs,
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GearRecipeArgs {
    #[serde(default)]
    pub family: Option<GearFamilyArg>,
    /// Normal/transverse conventions apply to rotary gears. Worm Axial module
    /// retains a NORMAL pressure_angle of 20 degrees; axial pressure is derived.
    #[serde(default)]
    pub module_system: Option<GearModuleSystemArg>,
    #[serde(default)]
    pub module: Option<GearScalarArg>,
    /// Selects the one authored module scalar: mm or teeth_per_in.
    #[serde(default)]
    pub pitch_input: Option<GearPitchInputArg>,
    /// Intrinsic external rotary hub, along the member local Z axis.
    #[serde(default)]
    pub hub: Option<GearHubArgs>,
    #[serde(default)]
    pub clear_hub: bool,
    /// Explicit standalone clearance-matched shifted external gear mate context.
    #[serde(default)]
    pub shift_mate: Option<ShiftedGearMateArgs>,
    #[serde(default)]
    pub clear_shift_mate: bool,
    #[serde(default)]
    pub teeth: Option<GearScalarArg>,
    #[serde(default)]
    pub pressure_angle: Option<GearScalarArg>,
    #[serde(default)]
    pub face_width: Option<Quantity>,
    /// Positive angle magnitude; specify hand separately.
    #[serde(default)]
    pub helix_angle: Option<GearScalarArg>,
    #[serde(default)]
    pub hand: Option<GearHandArg>,
    #[serde(default)]
    pub center_gap: Option<Quantity>,
    #[serde(default)]
    pub bore_diameter: Option<Quantity>,
    /// Rack-only connected backing below the root line; omission retains edits.
    #[serde(default)]
    pub backing_thickness: Option<Quantity>,
    /// Bevel tooth system; only spherical_involute_straight is implemented.
    #[serde(default)]
    pub bevel_system: Option<BevelToothSystemArg>,
    /// Explicit standalone worm member; paired members must have opposite roles.
    #[serde(default)]
    pub worm_role: Option<WormMemberArg>,
    /// Only za_axial_straight is available; zi/zn/zk fail closed.
    #[serde(default)]
    pub worm_system: Option<WormToothSystemArg>,
    /// Worm pitch diameter / axial module; only q=10 is supported.
    #[serde(default)]
    pub diameter_quotient: Option<GearScalarArg>,
    /// Standalone bevel or worm creation requires the intended mate count. Pairs derive
    /// this from the opposite member's original typed tooth-count source.
    #[serde(default)]
    pub mate_teeth: Option<GearScalarArg>,
    /// Bevel shaft angle. Standalone creation requires it; pairs share one source.
    #[serde(default)]
    pub shaft_angle: Option<GearScalarArg>,
    /// Explicit outside diameter for an internal ring; omitted edits retain it.
    #[serde(default)]
    pub outer_diameter: Option<Quantity>,
    /// Clear the optional outside diameter when switching to an external family.
    #[serde(default)]
    pub clear_outer_diameter: bool,
    /// Per-gear tangential tooth-thickness reduction at its reference circle.
    #[serde(default)]
    pub backlash: Option<Quantity>,
    /// Additional tangential thinning, independent of design backlash.
    #[serde(default)]
    pub fabrication_allowance: Option<Quantity>,
    #[serde(default)]
    pub root_fillet_ratio: Option<GearScalarArg>,
    #[serde(default)]
    pub minimum_wall: Option<Quantity>,
    /// Nonzero is bounded to sourced external cylindrical gear geometry.
    #[serde(default)]
    pub profile_shift: Option<GearScalarArg>,
    #[serde(default)]
    pub origin: Option<[Quantity; 3]>,
    #[serde(default)]
    pub plane: Option<Plane>,
    #[serde(default)]
    pub axis_u: Option<[f64; 3]>,
    #[serde(default)]
    pub axis_v: Option<[f64; 3]>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GearHubArgs {
    #[serde(default)]
    pub diameter: Option<Quantity>,
    #[serde(default)]
    pub axial_start: Option<Quantity>,
    #[serde(default)]
    pub axial_end: Option<Quantity>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShiftedGearMateArgs {
    #[serde(default)]
    pub teeth: Option<GearScalarArg>,
    #[serde(default)]
    pub profile_shift: Option<GearScalarArg>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectGearArgs {
    pub gear: String,
}
pub(super) fn patched_parameters(
    args: &GearRecipeArgs,
    previous: Option<&GearParameters>,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<GearParameters, ToolError> {
    let mut p = previous.cloned().unwrap_or_default();
    if args.clear_outer_diameter && args.outer_diameter.is_some() {
        return Err(ToolError::invalid(
            "Choose outside diameter replacement or clearing",
        ));
    }
    if args.clear_outer_diameter {
        p.outer_diameter = None;
    }
    if let Some(value) = args.family {
        let family = GearFamily::from(value);
        if family != p.family {
            p.bevel = None;
            if p.family == GearFamily::Worm {
                p.module_system = GearModuleSystem::Normal;
                p.root_fillet_ratio = model::ratio(0.2);
                p.teeth = Dimension::Literal {
                    value: 24.,
                    unit: "count".into(),
                };
            }
            p.worm = None;
            if family == GearFamily::Worm {
                let role = args.worm_role.map(Into::into).unwrap_or(WormMember::Worm);
                p.rack = None;
                p.module_system = GearModuleSystem::Axial;
                p.hand = GearHand::Right;
                p.pitch_input = GearPitchInput::Module;
                p.module = model::length(1.);
                p.teeth = Dimension::Literal {
                    value: if role == WormMember::Worm { 1. } else { 32. },
                    unit: "count".into(),
                };
                p.pressure_angle = model::angle(20.);
                p.face_width = model::length(if role == WormMember::Worm { 24. } else { 1. });
                p.helix_angle = model::angle(0.);
                p.center_gap = model::length(0.);
                p.profile_shift = model::ratio(0.);
                p.root_fillet_ratio = model::ratio(0.);
                p.outer_diameter = None;
                p.bore_diameter = model::length(0.);
                p.backlash = model::length(0.05);
                p.fabrication_allowance = model::length(0.);
                p.worm = Some(model::WormDimensions {
                    role,
                    tooth_system: WormToothSystem::ZaAxialStraight,
                    mate_teeth: Dimension::Literal {
                        value: if role == WormMember::Worm { 32. } else { 1. },
                        unit: "count".into(),
                    },
                    diameter_quotient: model::ratio(10.),
                });
            } else if family == GearFamily::Bevel {
                p.rack = None;
                p.helix_angle = model::angle(0.);
                p.hand = GearHand::None;
                p.center_gap = model::length(0.);
                p.outer_diameter = None;
            } else if family == GearFamily::Rack {
                p.bore_diameter = model::length(0.);
                p.helix_angle = Dimension::Literal {
                    value: 0.,
                    unit: "deg".into(),
                };
                p.hand = GearHand::None;
                p.center_gap = model::length(0.);
                p.outer_diameter = None;
                p.rack = Some(model::RackDimensions {
                    backing_thickness: model::length(4.),
                });
            } else {
                p.rack = None;
            }
        }
        p.family = family;
    }
    if let Some(outer) = &args.outer_diameter {
        p.outer_diameter = Some(outer.dimension(document, evaluated)?.0);
    }
    if let Some(backing) = &args.backing_thickness {
        if p.family != GearFamily::Rack {
            return Err(ToolError::invalid("Backing thickness belongs to a rack"));
        }
        p.rack = Some(model::RackDimensions {
            backing_thickness: backing.dimension(document, evaluated)?.0,
        });
    }
    if let Some(value) = args.pitch_input {
        p.pitch_input = value.into();
    }
    if let Some(value) = &args.module {
        p.module = value.dimension(
            document,
            evaluated,
            match p.pitch_input {
                GearPitchInput::Module => Unit::Millimeter,
                GearPitchInput::DiametralPitch => Unit::TeethPerInch,
            },
        )?;
    }
    if args.clear_hub && args.hub.is_some() {
        return Err(ToolError::invalid(
            "Choose hub replacement/patch or clearing",
        ));
    }
    if args.clear_hub {
        p.hub = None;
    }
    if let Some(hub) = &args.hub {
        let previous = p.hub.as_ref();
        let length = |value: &Option<Quantity>,
                      old: Option<&Dimension>,
                      name: &str|
         -> Result<Dimension, ToolError> {
            match value {
                Some(v) => Ok(v.dimension(document, evaluated)?.0),
                None => old.cloned().ok_or_else(|| {
                    ToolError::invalid(format!("A new intrinsic hub requires {name}"))
                }),
            }
        };
        p.hub = Some(model::GearHub {
            diameter: length(&hub.diameter, previous.map(|h| &h.diameter), "diameter")?,
            axial_start: length(
                &hub.axial_start,
                previous.map(|h| &h.axial_start),
                "axial_start",
            )?,
            axial_end: length(&hub.axial_end, previous.map(|h| &h.axial_end), "axial_end")?,
        });
    }
    if args.clear_shift_mate && args.shift_mate.is_some() {
        return Err(ToolError::invalid(
            "Choose shifted mate context patch or clearing",
        ));
    }
    if args.clear_shift_mate {
        p.shift_mate = None;
    }
    if let Some(mate) = &args.shift_mate {
        let previous = p.shift_mate.as_ref();
        let scalar = |value: &Option<GearScalarArg>,
                      old: Option<&Dimension>,
                      unit: Unit,
                      name: &str|
         -> Result<Dimension, ToolError> {
            match value {
                Some(v) => v.dimension(document, evaluated, unit),
                None => old.cloned().ok_or_else(|| {
                    ToolError::invalid(format!("A new shifted mate context requires {name}"))
                }),
            }
        };
        p.shift_mate = Some(model::ShiftedGearMate {
            teeth: scalar(
                &mate.teeth,
                previous.map(|m| &m.teeth),
                Unit::Count,
                "teeth",
            )?,
            profile_shift: scalar(
                &mate.profile_shift,
                previous.map(|m| &m.profile_shift),
                Unit::Ratio,
                "profile_shift",
            )?,
        });
    }
    if let Some(value) = args.module_system {
        p.module_system = value.into();
    }
    if let Some(value) = args.hand {
        p.hand = value.into();
    }
    for (field, value) in [
        (&mut p.face_width, &args.face_width),
        (&mut p.center_gap, &args.center_gap),
        (&mut p.bore_diameter, &args.bore_diameter),
        (&mut p.backlash, &args.backlash),
        (&mut p.fabrication_allowance, &args.fabrication_allowance),
        (&mut p.minimum_wall, &args.minimum_wall),
    ] {
        if let Some(value) = value {
            *field = value.dimension(document, evaluated)?.0;
        }
    }
    for (field, value, unit) in [
        (&mut p.teeth, &args.teeth, Unit::Count),
        (&mut p.pressure_angle, &args.pressure_angle, Unit::Degree),
        (&mut p.helix_angle, &args.helix_angle, Unit::Degree),
        (
            &mut p.root_fillet_ratio,
            &args.root_fillet_ratio,
            Unit::Ratio,
        ),
        (&mut p.profile_shift, &args.profile_shift, Unit::Ratio),
    ] {
        if let Some(value) = value {
            *field = value.dimension(document, evaluated, unit)?;
        }
    }
    if let Some(origin) = &args.origin {
        p.frame.origin = [
            origin[0].dimension(document, evaluated)?.0,
            origin[1].dimension(document, evaluated)?.0,
            origin[2].dimension(document, evaluated)?.0,
        ];
    }
    if args.plane.is_some() && (args.axis_u.is_some() || args.axis_v.is_some()) {
        return Err(ToolError::invalid(
            "Choose plane or axis_u/axis_v, not both",
        ));
    }
    if let Some(plane) = args.plane {
        let f = Datum::from(plane).frame();
        p.frame.u = f.u;
        p.frame.v = f.v;
    }
    match (args.axis_u, args.axis_v) {
        (Some(u), Some(v)) => {
            p.frame.u = u;
            p.frame.v = v;
        }
        (None, None) => {}
        _ => {
            return Err(ToolError::invalid(
                "An explicit gear basis needs both axis_u and axis_v",
            ));
        }
    }
    let has_bevel_args = args.bevel_system.is_some() || args.shaft_angle.is_some();
    let has_worm_args =
        args.worm_role.is_some() || args.worm_system.is_some() || args.diameter_quotient.is_some();
    if p.family == GearFamily::Worm {
        if has_bevel_args {
            return Err(ToolError::invalid(
                "Bevel system and shaft angle do not belong to a worm recipe",
            ));
        }
        let mut worm = p
            .worm
            .take()
            .ok_or_else(|| ToolError::invalid("Worm dimensions are missing"))?;
        if let Some(role) = args.worm_role {
            worm.role = role.into();
        }
        if let Some(system) = args.worm_system {
            worm.tooth_system = system.into();
        }
        if let Some(value) = &args.mate_teeth {
            worm.mate_teeth = value.dimension(document, evaluated, Unit::Count)?;
        }
        if let Some(value) = &args.diameter_quotient {
            worm.diameter_quotient = value.dimension(document, evaluated, Unit::Ratio)?;
        }
        p.worm = Some(worm);
    } else if has_worm_args {
        return Err(ToolError::invalid(
            "Worm role, system and diameter quotient belong only to a worm recipe",
        ));
    }
    if p.family == GearFamily::Bevel {
        let mut bevel = p.bevel.take().unwrap_or(model::BevelDimensions {
            tooth_system: BevelToothSystem::SphericalInvoluteStraight,
            mate_teeth: p.teeth.clone(),
            shaft_angle: model::angle(90.),
        });
        if let Some(system) = args.bevel_system {
            bevel.tooth_system = system.into();
        }
        if let Some(value) = &args.mate_teeth {
            bevel.mate_teeth = value.dimension(document, evaluated, Unit::Count)?;
        }
        if let Some(value) = &args.shaft_angle {
            bevel.shaft_angle = value.dimension(document, evaluated, Unit::Degree)?;
        }
        p.bevel = Some(bevel);
    } else if has_bevel_args || (args.mate_teeth.is_some() && p.family != GearFamily::Worm) {
        return Err(ToolError::invalid(
            "Bevel system, mate teeth and shaft angle belong to a bevel recipe",
        ));
    }
    Ok(p)
}
pub(super) fn parameters(
    args: &GearRecipeArgs,
    previous: Option<&GearParameters>,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<GearParameters, ToolError> {
    let p = patched_parameters(args, previous, document, evaluated)?;
    if p.family == GearFamily::Bevel
        && previous.is_none_or(|old| old.family != GearFamily::Bevel)
        && (args.mate_teeth.is_none() || args.shaft_angle.is_none())
    {
        return Err(ToolError::invalid(
            "A standalone bevel needs explicit mate_teeth and shaft_angle; set_gear_pair derives paired mate counts",
        ));
    }
    if p.family == GearFamily::Worm
        && previous.is_none_or(|old| old.family != GearFamily::Worm)
        && (args.worm_role.is_none() || args.mate_teeth.is_none())
    {
        return Err(ToolError::invalid(
            "A standalone worm needs explicit worm_role and mate_teeth; set_gear_pair derives paired mate counts",
        ));
    }
    p.resolve_geometry(evaluated)?;
    Ok(p)
}
pub(super) fn gear_profiles(_: &dyn Host, _: &Request, _: NoArgs) -> ToolResult {
    Ok(
        json!({"families":GearFamily::ALL.map(|f| json!({"family":f,"label":f.label(),"supported":f.supported(),"source":f.source()})),
        "sources":[model::GEAR_DIMENSIONS_SOURCE_URL,model::GEAR_PROFILE_SOURCE_URL,model::GEAR_FILLET_SOURCE_URL,model::BEVEL_PROFILE_SOURCE_URL,model::BEVEL_DIMENSIONS_SOURCE_URL],
        "worm":{"source":model::WORM_PROFILE_SOURCE,"tooth_system":"za_axial_straight","module_system":"axial","normal_pressure_angle_degrees":20,"axial_module_mm":[0.5,2.0],"diameter_quotient":10,"worm_starts":1,"hand":"right","wheel_teeth":[32,64],"worm_length":"Independent face_width, 16–32 times module","wheel_face_width":"Independent face_width, 0.5–2 times module","thinning":"Backlash plus fabrication allowance at most 0.1 times module","datum":"Members centered on their own axial Z=0; wheel axis crossed 90 degrees","unsupported_systems":["zi","zn","zk"],"limits":"No load, contact, efficiency or manufacturing certification"},
        "external_profile_coverage":{"pitch_input":["module","diametral_pitch"],"authored_scalar":"module is the ONE active scalar: mm for module, teeth_per_in for diametral_pitch; no numeric-copy conversion","convention":"Independent normal/transverse pitch and pressure convention","hub":"Optional intrinsic diameter and localZ axial_start/axial_end; positive band overlap and minimum bore wall; radius below root","shifted_external":{"source":model::SHIFTED_GEAR_SOURCE,"source_url":model::SHIFTED_GEAR_SOURCE_URL,"undercut_source_url":model::SHIFTED_GEAR_UNDERCUT_SOURCE_URL,"families":["spur","helical","herringbone","double_helical"],"coefficient":[-0.5,1.0],"selected_pressure_degrees":[14.5,35.0],"limits":"Selected-plane source undercut equation applies; positive crest/root, active contact line, ratio and native interference guards remain. Shifted helical pairs require matching selected normal/transverse conventions.","matched_context":"Retained original mate teeth/profile_shift, with KHK paired tip shortening"}},
        "internal_ring":{"teeth":[48,60],"pressure_angle_degrees":20,"pinion_teeth":24,"helix":false,"profile_shift":0,"outer_diameter":"Explicit; rim beyond root must meet minimum_wall","bore_diameter":"Zero; inward toothed opening is derived","root":"Custom circular fillet, not a simulated cutter trochoid"},
        "rack":{"teeth":[4,64],"length":"Derived as teeth*pi*module; whole centered pitch cells","backing_thickness":"Explicit connected base below root line","root":"Custom tangent circular fillet, not a cutter trochoid","pair":"One rack and one matching unshifted non-undercut spur pinion; conservative finite travel keeps full pinion tip-circle projection within rack ends","drive":"Positive pinion CCW advances rack along positive local X"},
        "bevel":{"tooth_system":"spherical_involute_straight","teeth":[24,64],"pressure_angle_degrees":20,"shaft_angle_degrees":90,
        "face_width":"Along pitch cone; at most min(outer cone distance/3,10*module)","datum":"Source frame origin is the pitch apex; paired axes intersect there",
        "blank":"KG standard heel/toe backcones","root":"Custom tangent small circle on the outer reference sphere; not a cutter-generated root",
        "unavailable_systems":["gleason_octoid","spiral","zerol"],"limits":"No hubs, helix, profile shift, load or fit certification; bore and wall checked at toe root"},
        "pair_placement":{"supported":true,"tool":"set_gear_pair","limits":"Matching external family, bounded internal ring/spur pinion, finite straight rack/spur pinion, bounded intersecting-axis spherical-involute bevels, or explicit ZA worm/wheel roles with independent axial widths. Rotary pairs match transverse module/pressure angle and width; external helical hands opposite. Internal centers use radius difference and same-direction rotation. Unmodified periodic generator Parts only"},
        "scope":"Bounded rotary involute gears, finite straight racks, ZA worms/generated wheels and compatible parallel/intersecting/crossed-axis pairs, with declared custom root blends; no strength, fit or accuracy certification. Unsupported families/combinations fail closed."}),
    )
}
pub(super) fn inspect_gear(host: &dyn Host, _: &Request, args: InspectGearArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let feature = id(&args.gear)?;
    let source = model::read_gear(&document, feature)?;
    let geometry = source.parameters.resolve_geometry(&evaluated)?;
    if let Some(r) = geometry.worm() {
        return Ok(
            json!({"revision":revision,"gear":feature,"part":source.output.body,"source":source,
            "resolved":{"role":r.role,"tooth_system":r.tooth_system,"teeth":r.teeth,"starts":r.starts,"wheel_teeth":r.wheel_teeth,
            "axial_module_mm":r.axial_module_mm,"diameter_quotient":r.diameter_quotient,"normal_pressure_angle_rad":r.normal_pressure_angle_rad,
            "axial_pressure_angle_rad":r.axial_pressure_angle_rad,"lead_angle_rad":r.lead_angle_rad,"pitch_mm":r.pitch_mm,"lead_mm":r.lead_mm,
            "pitch_radius_mm":r.pitch_radius_mm,"tip_radius_mm":r.tip_radius_mm,"minimum_root_radius_mm":r.minimum_root_radius_mm,
            "bore_radius_mm":r.bore_radius_mm,"face_width_mm":r.face_width_mm,"nominal_tooth_thickness_mm":r.nominal_tooth_thickness_mm,
            "design_tooth_thickness_mm":r.design_tooth_thickness_mm,"physical_tooth_thickness_mm":r.physical_tooth_thickness_mm,
            "thinning_mm":r.thinning_mm,"hob_pitch_space_thickness_mm":r.hob_pitch_space_thickness_mm},
            "scope":"ZA axial-straight bounded worm or generated wheel; normal20 pressure, axial module; no load/contact/efficiency certification. evaluate_model supplies actual native evidence."}),
        );
    }
    if let model::ResolvedGearGeometry::Rack(r) = &geometry {
        return Ok(
            json!({"revision":revision,"gear":feature,"part":source.output.body,"source":source,
            "resolved":{"teeth":r.teeth,"module_mm":r.module_mm,"pitch_mm":r.pitch_mm,
                "pressure_angle_rad":r.pressure_angle_rad,"length_mm":r.length_mm,
                "face_width_mm":r.face_width_mm,"backing_thickness_mm":r.backing_thickness_mm,
                "addendum_mm":r.addendum_mm,"dedendum_mm":r.dedendum_mm,
                "root_fillet_radius_mm":r.root_fillet_radius_mm,
                "nominal_tooth_thickness_mm":r.nominal_tooth_thickness_mm,
                "design_tooth_thickness_mm":r.design_tooth_thickness_mm,
                "physical_tooth_thickness_mm":r.physical_tooth_thickness_mm},
            "scope":"Whole pitch cells, straight flanks and custom tangent circular root fillets; canonical dimensions, use evaluate_model for native geometry"}),
        );
    }
    if let Some(r) = geometry.bevel() {
        return Ok(
            json!({"revision":revision,"gear":feature,"part":source.output.body,"source":source,
            "resolved":{"tooth_system":"spherical_involute_straight","teeth":r.teeth,"mate_teeth":r.mate_teeth,
            "module_mm":r.module_mm,"pressure_angle_rad":r.pressure_angle_rad,"shaft_angle_rad":r.shaft_angle_rad,
            "pitch_cone_angle_rad":r.pitch_cone_angle_rad,"base_cone_angle_rad":r.base_cone_angle_rad,
            "tip_cone_angle_rad":r.tip_cone_angle_rad,"root_cone_angle_rad":r.root_cone_angle_rad,
            "outer_cone_distance_mm":r.outer_cone_distance_mm,"inner_cone_distance_mm":r.inner_cone_distance_mm,
            "face_width_along_cone_mm":r.face_width_mm,"bore_radius_mm":r.bore_radius_mm,
            "heel_tip_radius_mm":r.heel_tip_radius_mm,"toe_tip_radius_mm":r.toe_tip_radius_mm,
            "heel_root_radius_mm":r.heel_root_radius_mm,"toe_root_radius_mm":r.toe_root_radius_mm,
            "toe_bore_wall_mm":r.toe_root_radius_mm-r.bore_radius_mm,"minimum_wall_mm":r.minimum_wall_mm,
            "nominal_tooth_thickness_mm":r.nominal_tooth_thickness_mm,
            "design_tooth_thickness_mm":r.design_tooth_thickness_mm,"physical_tooth_thickness_mm":r.physical_tooth_thickness_mm,
            "root_fillet_radius_mm":r.root_fillet_radius_mm,"half_base_angle_rad":r.half_base_angle_rad},
            "scope":"Spherical-involute straight flanks, declared backcone blank and custom spherical-circle roots; source dimensions only, use evaluate_model for native geometry"}),
        );
    }
    let r = geometry.rotary().expect("rotary geometry");
    let shifted_source = matches!(
        r.family,
        GearFamily::Spur
            | GearFamily::Helical
            | GearFamily::Herringbone
            | GearFamily::DoubleHelical
    ) && (r.profile_shift_coefficient != 0.
        || source.parameters.shift_mate.is_some()
        || source.parameters.profile_shift.expression().is_some());
    Ok(
        json!({"revision":revision,"gear":feature,"part":source.output.body,"source":source,
        "resolved":{"normal_module_mm":r.normal_module_mm,"transverse_module_mm":r.transverse_module_mm,"teeth":r.teeth,
        "normal_pressure_angle_rad":r.normal_pressure_angle_rad,"transverse_pressure_angle_rad":r.transverse_pressure_angle_rad,
        "pitch_diameter_mm":2.*r.pitch_radius_mm,"base_diameter_mm":2.*r.base_radius_mm,"tip_diameter_mm":2.*r.tip_radius_mm,"root_diameter_mm":2.*r.root_radius_mm,
        "outer_diameter_mm":r.outer_radius_mm.map(|v|2.*v),"rim_thickness_mm":r.outer_radius_mm.map(|v|v-r.root_radius_mm),
        "nominal_tooth_thickness_mm":r.nominal_tooth_thickness_mm,"design_tooth_thickness_mm":r.design_tooth_thickness_mm,"physical_tooth_thickness_mm":r.physical_tooth_thickness_mm,"twist_rad_per_mm":r.twist_rad_per_mm,
        "profile_shift_coefficient":r.profile_shift_coefficient,"tip_shortening_coefficient":r.tip_shortening_coefficient,
        "hub":r.hub.as_ref().map(|h|json!({"diameter_mm":2.*h.radius_mm,"axial_start_mm":h.axial_start_mm,"axial_end_mm":h.axial_end_mm}))},
        "profile_source":shifted_source.then_some(model::SHIFTED_GEAR_SOURCE),"profile_source_url":shifted_source.then_some(model::SHIFTED_GEAR_SOURCE_URL),"profile_undercut_source_url":shifted_source.then_some(model::SHIFTED_GEAR_UNDERCUT_SOURCE_URL),
        "scope":"Canonical source and resolved dimensions; use evaluate_model for native geometry"}),
    )
}
pub(super) fn set_gear(host: &dyn Host, call: &Request, args: GearArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            "Gear request is based on a stale revision",
        )
        .at_revision(revision));
    }
    let edit = args.gear.as_deref().map(id).transpose()?;
    let previous = edit
        .map(|feature| model::read_gear(&document, feature))
        .transpose()?;
    if edit.is_some() && args.name.is_some() {
        return Err(ToolError::invalid(
            "Gear edits retain their name and Part; omit name",
        ));
    }
    let parameters = parameters(
        &args.recipe,
        previous.as_ref().map(|s| &s.parameters),
        &document,
        &evaluated,
    )?;
    let request = if edit.is_none() {
        Some(model::GearRequest::new(
            args.name
                .ok_or_else(|| ToolError::invalid("A new gear needs a name"))?,
            parameters.clone(),
        ))
    } else {
        None
    };
    let expected = args.expected_revision;
    let owned = call.clone();
    let (prepared, feature, body) = with(host, move |target| {
        revision_guard(target, expected)?;
        let prepared = match request {
            Some(request) => {
                model::prepare_gear(target.session(), expected, owned.actor.clone(), &request)?
            }
            None => model::prepare_gear_edit(
                target.session(),
                expected,
                owned.actor.clone(),
                edit.expect("gear edit"),
                &parameters,
            )?,
        };
        let output = prepared.outputs[0].clone();
        Ok::<_, ToolError>((prepared.prepared, output.feature, output.body))
    })??;
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    let committed = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":committed,"gear":feature,"part":body}))
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    #[test]
    fn shared_recipe_keeps_the_existing_flat_wire_and_rejects_unknown_fields() {
        let value = json!({"expected_revision":0,"name":"Gear","module":{"value":2.,"unit":"mm"},"family":"spur"});
        let args: GearArgs = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(args.recipe.family, Some(GearFamilyArg::Spur)));
        assert!(args.recipe.module.is_some());
        let mut bad = value;
        bad["unverified_grade"] = json!(6);
        assert!(serde_json::from_value::<GearArgs>(bad).is_err());
        assert!(
            serde_json::from_value::<GearRecipeArgs>(json!({"gear":"not-a-recipe-field"})).is_err()
        );
        let schema = serde_json::to_value(schemars::schema_for!(GearArgs)).unwrap();
        assert!(schema["properties"]["module"].is_object());
        assert!(schema["properties"]["family"].is_object());
        assert!(schema["properties"].get("recipe").is_none());
    }
}

#[cfg(test)]
mod worm_recipe_tests {
    use super::*;
    fn recipe(value: serde_json::Value) -> GearRecipeArgs {
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn worm_wire_uses_axial_module_normal_pressure_and_preserves_omitted_edits() {
        let session =
            multicad_core::commands::Session::new(Document::new("Worm wire").unwrap()).unwrap();
        let args = recipe(
            json!({"family":"worm","worm_role":"worm","mate_teeth":{"value":40.,"unit":"count"}}),
        );
        let a = parameters(&args, None, session.document(), session.parameters()).unwrap();
        let r = a.resolve_geometry(session.parameters()).unwrap();
        let r = r.worm().unwrap();
        assert_eq!(r.teeth, 1);
        assert_eq!(r.wheel_teeth, 40);
        assert_eq!(a.module_system, GearModuleSystem::Axial);
        assert!((r.normal_pressure_angle_rad.to_degrees() - 20.).abs() < 1e-12);
        assert!((r.axial_pressure_angle_rad - r.normal_pressure_angle_rad).abs() > 1e-4);
        let b = parameters(
            &recipe(json!({"face_width":{"value":20.,"unit":"mm"}})),
            Some(&a),
            session.document(),
            session.parameters(),
        )
        .unwrap();
        assert_eq!(b.worm, a.worm);
        assert_eq!(b.teeth, a.teeth);
        assert_eq!(b.face_width, model::length(20.));
        for bad in [
            json!({"worm_system":"zi"}),
            json!({"hand":"left"}),
            json!({"module_system":"normal"}),
            json!({"diameter_quotient":{"value":11.,"unit":"ratio"}}),
            json!({"teeth":{"value":2.,"unit":"count"}}),
            json!({"outer_diameter":{"value":20.,"unit":"mm"}}),
        ] {
            assert!(
                parameters(
                    &recipe(bad),
                    Some(&a),
                    session.document(),
                    session.parameters()
                )
                .is_err()
            );
        }
        assert!(
            parameters(
                &recipe(json!({"family":"worm","mate_teeth":{"value":40.,"unit":"count"}})),
                None,
                session.document(),
                session.parameters()
            )
            .is_err()
        );
        assert_eq!(session.revision(), 0);
    }
}

#[cfg(test)]
mod coverage_recipe_tests {
    use super::*;
    #[test]
    fn pitch_unit_and_hub_partial_edits_preserve_original_dimensions_and_explicit_clearing() {
        let s =
            multicad_core::commands::Session::new(Document::new("Gear patch contract").unwrap())
                .unwrap();
        let parse = |value| serde_json::from_value::<GearRecipeArgs>(value).unwrap();
        let a=parameters(&parse(json!({"pitch_input":"diametral_pitch","module":{"value":12.7,"unit":"teeth_per_in"},"hub":{"diameter":{"value":20.,"unit":"mm"},"axial_start":{"value":-2.,"unit":"mm"},"axial_end":{"value":12.,"unit":"mm"}}})),None,s.document(),s.parameters()).unwrap();
        let r = a.resolve_geometry(s.parameters()).unwrap();
        assert!((r.rotary().unwrap().normal_module_mm - 2.).abs() < 1e-12);
        let b = parameters(
            &parse(json!({"hub":{"axial_end":{"value":14.,"unit":"mm"}}})),
            Some(&a),
            s.document(),
            s.parameters(),
        )
        .unwrap();
        assert_eq!(a.module, b.module);
        assert_eq!(a.pitch_input, b.pitch_input);
        assert_eq!(
            a.hub.as_ref().unwrap().diameter,
            b.hub.as_ref().unwrap().diameter
        );
        assert_eq!(b.hub.as_ref().unwrap().axial_end, model::length(14.));
        for bad in [
            json!({"pitch_input":"module"}),
            json!({"module":{"value":2.,"unit":"mm"}}),
            json!({"clear_hub":true,"hub":{}}),
            json!({"hub":{"diameter":{"value":100.,"unit":"mm"}}}),
        ] {
            assert!(parameters(&parse(bad), Some(&a), s.document(), s.parameters()).is_err());
        }
        let cleared = parameters(
            &parse(json!({"clear_hub":true})),
            Some(&a),
            s.document(),
            s.parameters(),
        )
        .unwrap();
        assert!(cleared.hub.is_none());
        assert_eq!(cleared.module, a.module);
        let legacy = parameters(&parse(json!({})), None, s.document(), s.parameters()).unwrap();
        assert_eq!(legacy, model::GearParameters::default());
        assert!(
            serde_json::to_value(legacy)
                .unwrap()
                .get("pitch_input")
                .is_none()
        );
    }
    #[test]
    fn dp_wire_keeps_the_authored_source_in_both_helical_pitch_planes() {
        let s = multicad_core::commands::Session::new(Document::new("DP plane contract").unwrap())
            .unwrap();
        let beta = 15_f64.to_radians();
        let normal = 20_f64.to_radians();
        let dimensions = |r: &model::ResolvedGear| {
            [
                r.normal_module_mm,
                r.transverse_module_mm,
                r.pitch_radius_mm,
                r.base_radius_mm,
                r.tip_radius_mm,
                r.root_radius_mm,
            ]
        };
        let mut radii = vec![];
        for (plane, dp, angle) in [
            ("normal", 12.7, 20.),
            (
                "transverse",
                12.7 * beta.cos(),
                (normal.tan() / beta.cos()).atan().to_degrees(),
            ),
        ] {
            let wire = json!({"family":"helical","pitch_input":"diametral_pitch","module_system":plane,"module":{"value":dp,"unit":"teeth_per_in"},"pressure_angle":{"value":angle,"unit":"deg"},"helix_angle":{"value":15.,"unit":"deg"},"hand":"right"});
            let args: GearRecipeArgs = serde_json::from_value(wire.clone()).unwrap();
            let p = parameters(&args, None, s.document(), s.parameters()).unwrap();
            assert_eq!(
                p.module,
                Dimension::Literal {
                    value: dp,
                    unit: "teeth_per_in".into()
                }
            );
            assert_eq!(p.pitch_input, GearPitchInput::DiametralPitch);
            let resolved = p.resolve_geometry(s.parameters()).unwrap();
            let dp_dimensions = dimensions(resolved.rotary().unwrap());
            // A DP input is physically equivalent to an explicit module input
            // in the SAME plane, including that plane's tip/root convention.
            let mut module_wire = wire;
            module_wire["pitch_input"] = json!("module");
            module_wire["module"] = json!({"value":25.4/dp,"unit":"mm"});
            let module_args: GearRecipeArgs = serde_json::from_value(module_wire).unwrap();
            let module = parameters(&module_args, None, s.document(), s.parameters()).unwrap();
            assert_eq!(module.module, model::length(25.4 / dp));
            let module_geometry = module.resolve_geometry(s.parameters()).unwrap();
            for (a, b) in dp_dimensions
                .into_iter()
                .zip(dimensions(module_geometry.rotary().unwrap()))
            {
                assert!((a - b).abs() < 1e-12);
            }
            radii.push(dp_dimensions);
        }
        // Converted planes share reference quantities, while the established
        // addendum/dedendum convention uses each selected module explicitly.
        for (a, b) in radii[0][..4].iter().zip(&radii[1][..4]) {
            assert!((a - b).abs() < 1e-12);
        }
        let selected_difference = 2. / beta.cos() - 2.;
        assert!(radii[1][4] > radii[0][4]);
        assert!(radii[1][5] < radii[0][5]);
        assert!((radii[1][4] - radii[0][4] - selected_difference).abs() < 1e-12);
        assert!((radii[1][5] - radii[0][5] + 1.25 * selected_difference).abs() < 1e-12);
    }
    #[test]
    fn shifted_external_parser_preserves_context_in_both_planes_for_helical_and_chevrons() {
        let s =
            multicad_core::commands::Session::new(Document::new("External shift planes").unwrap())
                .unwrap();
        for family in ["helical", "herringbone", "double_helical"] {
            for plane in ["normal", "transverse"] {
                let wire = json!({"family":family,"pitch_input":"diametral_pitch","module_system":plane,"module":{"value":12.7,"unit":"teeth_per_in"},"pressure_angle":{"value":25.,"unit":"deg"},"helix_angle":{"value":15.,"unit":"deg"},"hand":"right","center_gap":{"value":if family=="double_helical"{2.}else{0.},"unit":"mm"},"profile_shift":{"value":0.2,"unit":"ratio"},"shift_mate":{"teeth":{"value":36.,"unit":"count"},"profile_shift":{"value":0.1,"unit":"ratio"}}});
                let args: GearRecipeArgs = serde_json::from_value(wire).unwrap();
                let p = parameters(&args, None, s.document(), s.parameters()).unwrap();
                let original = p.clone();
                let empty: GearRecipeArgs = serde_json::from_value(json!({})).unwrap();
                let retained = parameters(&empty, Some(&p), s.document(), s.parameters()).unwrap();
                assert_eq!(retained, original);
                assert_eq!(retained.profile_shift, model::ratio(0.2));
                assert!(retained.shift_mate.is_some());
                let r = retained.resolve_geometry(s.parameters()).unwrap();
                assert_eq!(r.rotary().unwrap().profile_shift_coefficient, 0.2);
            }
        }
    }
}
