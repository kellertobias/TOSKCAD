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
    model::{self, GearFamily, GearHand, GearModuleSystem, GearParameters},
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
    fn dimension(
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
fn parameters(
    args: &GearArgs,
    previous: Option<&GearParameters>,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<GearParameters, ToolError> {
    let mut p = previous.cloned().unwrap_or_default();
    if let Some(value) = args.family {
        p.family = value.into();
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
    p.resolve(evaluated)?;
    Ok(p)
}
pub(super) fn gear_profiles(_: &dyn Host, _: &Request, _: NoArgs) -> ToolResult {
    Ok(
        json!({"families":GearFamily::ALL.map(|f| json!({"family":f,"label":f.label(),"supported":f.supported(),"source":f.source()})),
        "sources":[model::GEAR_DIMENSIONS_SOURCE_URL,model::GEAR_PROFILE_SOURCE_URL,model::GEAR_FILLET_SOURCE_URL],
        "scope":"Unshifted external gears, custom circular root fillets; no paired placement, strength, fit or accuracy certification. Conservative normal-angle tooth-count policy; unsupported families fail closed."}),
    )
}
pub(super) fn inspect_gear(host: &dyn Host, _: &Request, args: InspectGearArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let feature = id(&args.gear)?;
    let source = model::read_gear(&document, feature)?;
    let r = source.parameters.resolve(&evaluated)?;
    Ok(
        json!({"revision":revision,"gear":feature,"part":source.output.body,"source":source,
        "resolved":{"normal_module_mm":r.normal_module_mm,"transverse_module_mm":r.transverse_module_mm,"teeth":r.teeth,
        "normal_pressure_angle_rad":r.normal_pressure_angle_rad,"transverse_pressure_angle_rad":r.transverse_pressure_angle_rad,
        "pitch_diameter_mm":2.*r.pitch_radius_mm,"base_diameter_mm":2.*r.base_radius_mm,"tip_diameter_mm":2.*r.tip_radius_mm,"root_diameter_mm":2.*r.root_radius_mm,
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
        &args,
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
