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
    model::{self, BevelToothSystem, GearFamily, GearHand, GearModuleSystem, GearParameters},
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
argument_enum!(GearModuleSystemArg, GearModuleSystem, [Normal, Transverse]);

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
    /// Selects the convention for both module and pressure_angle.
    #[serde(default)]
    pub module_system: Option<GearModuleSystemArg>,
    #[serde(default)]
    pub module: Option<Quantity>,
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
    /// Standalone bevel creation requires the intended mate count. Pairs derive
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
    /// Only zero is currently implemented.
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
            if family == GearFamily::Bevel {
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
    if let Some(value) = args.module_system {
        p.module_system = value.into();
    }
    if let Some(value) = args.hand {
        p.hand = value.into();
    }
    for (field, value) in [
        (&mut p.module, &args.module),
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
    let has_bevel_args =
        args.bevel_system.is_some() || args.mate_teeth.is_some() || args.shaft_angle.is_some();
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
    } else if has_bevel_args {
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
    p.resolve_geometry(evaluated)?;
    Ok(p)
}
pub(super) fn gear_profiles(_: &dyn Host, _: &Request, _: NoArgs) -> ToolResult {
    Ok(
        json!({"families":GearFamily::ALL.map(|f| json!({"family":f,"label":f.label(),"supported":f.supported(),"source":f.source()})),
        "sources":[model::GEAR_DIMENSIONS_SOURCE_URL,model::GEAR_PROFILE_SOURCE_URL,model::GEAR_FILLET_SOURCE_URL,model::BEVEL_PROFILE_SOURCE_URL,model::BEVEL_DIMENSIONS_SOURCE_URL],
        "internal_ring":{"teeth":[48,60],"pressure_angle_degrees":20,"pinion_teeth":24,"helix":false,"profile_shift":0,"outer_diameter":"Explicit; rim beyond root must meet minimum_wall","bore_diameter":"Zero; inward toothed opening is derived","root":"Custom circular fillet, not a simulated cutter trochoid"},
        "rack":{"teeth":[4,64],"length":"Derived as teeth*pi*module; whole centered pitch cells","backing_thickness":"Explicit connected base below root line","root":"Custom tangent circular fillet, not a cutter trochoid","pair":"One rack and one matching unshifted non-undercut spur pinion; conservative finite travel keeps full pinion tip-circle projection within rack ends","drive":"Positive pinion CCW advances rack along positive local X"},
        "bevel":{"tooth_system":"spherical_involute_straight","teeth":[24,64],"pressure_angle_degrees":20,"shaft_angle_degrees":90,
        "face_width":"Along pitch cone; at most min(outer cone distance/3,10*module)","datum":"Source frame origin is the pitch apex; paired axes intersect there",
        "blank":"KG standard heel/toe backcones","root":"Custom tangent small circle on the outer reference sphere; not a cutter-generated root",
        "unavailable_systems":["gleason_octoid","spiral","zerol"],"limits":"No hubs, helix, profile shift, load or fit certification; bore and wall checked at toe root"},
        "pair_placement":{"supported":true,"tool":"set_gear_pair","limits":"Matching external family, bounded internal ring/spur pinion, finite straight rack/spur pinion, or bounded intersecting-axis spherical-involute bevels; matching transverse module/pressure angle and width; external helical hands opposite. Internal centers use radius difference and same-direction rotation. Unmodified periodic generator Parts only"},
        "scope":"Bounded rotary involute gears, finite straight racks and compatible parallel/intersecting-axis pairs, with declared custom root blends; no strength, fit or accuracy certification. Unsupported families/combinations fail closed."}),
    )
}
pub(super) fn inspect_gear(host: &dyn Host, _: &Request, args: InspectGearArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let feature = id(&args.gear)?;
    let source = model::read_gear(&document, feature)?;
    let geometry = source.parameters.resolve_geometry(&evaluated)?;
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
    Ok(
        json!({"revision":revision,"gear":feature,"part":source.output.body,"source":source,
        "resolved":{"normal_module_mm":r.normal_module_mm,"transverse_module_mm":r.transverse_module_mm,"teeth":r.teeth,
        "normal_pressure_angle_rad":r.normal_pressure_angle_rad,"transverse_pressure_angle_rad":r.transverse_pressure_angle_rad,
        "pitch_diameter_mm":2.*r.pitch_radius_mm,"base_diameter_mm":2.*r.base_radius_mm,"tip_diameter_mm":2.*r.tip_radius_mm,"root_diameter_mm":2.*r.root_radius_mm,
        "outer_diameter_mm":r.outer_radius_mm.map(|v|2.*v),"rim_thickness_mm":r.outer_radius_mm.map(|v|v-r.root_radius_mm),
        "nominal_tooth_thickness_mm":r.nominal_tooth_thickness_mm,"design_tooth_thickness_mm":r.design_tooth_thickness_mm,"physical_tooth_thickness_mm":r.physical_tooth_thickness_mm,"twist_rad_per_mm":r.twist_rad_per_mm},
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
