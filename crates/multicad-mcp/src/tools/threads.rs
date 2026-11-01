//! Typed thread tools prepare the canonical API and compile before committing.
use super::{
    NoArgs, Plane, Quantity, Request, ToolResult, commit, compile, current_producer, id,
    revision_guard, snapshot,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    model::{self, ThreadFamily, ThreadHand, ThreadMode, ThreadParameters, ThreadRepresentation},
    parameters::{BoundExpression, EvaluatedParameters, Unit},
    sketch::Datum,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadFamilyArg {
    IsoMetric,
    Unified,
    Whitworth,
    Bspp,
    Bspt,
    Npt,
    Trapezoidal,
    Acme,
    Buttress,
    Round,
    Custom,
}
impl From<ThreadFamilyArg> for ThreadFamily {
    fn from(f: ThreadFamilyArg) -> Self {
        match f {
            ThreadFamilyArg::IsoMetric => Self::IsoMetric,
            ThreadFamilyArg::Unified => Self::Unified,
            ThreadFamilyArg::Whitworth => Self::Whitworth,
            ThreadFamilyArg::Bspp => Self::Bspp,
            ThreadFamilyArg::Bspt => Self::Bspt,
            ThreadFamilyArg::Npt => Self::Npt,
            ThreadFamilyArg::Trapezoidal => Self::Trapezoidal,
            ThreadFamilyArg::Acme => Self::Acme,
            ThreadFamilyArg::Buttress => Self::Buttress,
            ThreadFamilyArg::Round => Self::Round,
            ThreadFamilyArg::Custom => Self::Custom,
        }
    }
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadModeArg {
    NewExternal,
    CutExternal,
    CutInternal,
}
impl From<ThreadModeArg> for ThreadMode {
    fn from(m: ThreadModeArg) -> Self {
        match m {
            ThreadModeArg::NewExternal => Self::NewExternal,
            ThreadModeArg::CutExternal => Self::CutExternal,
            ThreadModeArg::CutInternal => Self::CutInternal,
        }
    }
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadHandArg {
    Right,
    Left,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadRepresentationArg {
    Modeled,
    CosmeticEnvelope,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadScalarArg {
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub expression: Option<String>,
    /// deg/rad for a flank angle; ratio/% for radius taper. Give exactly one value or expression.
    pub unit: String,
}
impl ThreadScalarArg {
    fn dimension(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        expected: Unit,
    ) -> Result<Dimension, ToolError> {
        let unit = Unit::parse(&self.unit)
            .filter(|unit| unit.dimension() == expected.dimension())
            .ok_or_else(|| {
                ToolError::invalid(format!(
                    "Thread scalar needs units compatible with {}",
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
                    .evaluate(parameters)?
                    .in_unit(expected)
                    .ok_or_else(|| {
                        ToolError::invalid("Thread scalar expression has incompatible units")
                    })?;
                Ok(Dimension::Expression { expression })
            }
            _ => Err(ToolError::invalid(
                "Give exactly one finite thread scalar value or expression",
            )),
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadProfileArgs {
    pub depth: Quantity,
    pub crest_width: Quantity,
    pub lower_flank: ThreadScalarArg,
    pub upper_flank: ThreadScalarArg,
    pub radial_taper: ThreadScalarArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadArgs {
    pub expected_revision: u64,
    /// Existing thread feature ID. Omitted fields retain its canonical values and target predecessor.
    #[serde(default)]
    pub thread: Option<String>,
    /// Required for creation. Rename an existing feature through the general metadata API.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mode: Option<ThreadModeArg>,
    /// Required by new receiving cuts; omitted for a new shaft. Cannot retarget an edit.
    #[serde(default)]
    pub target_part: Option<String>,
    #[serde(flatten)]
    pub recipe: ThreadRecipeArgs,
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadRecipeArgs {
    #[serde(default)]
    pub family: Option<ThreadFamilyArg>,
    /// Full explicit replacement of a Custom profile, including its angle/taper units.
    #[serde(default)]
    pub custom_profile: Option<ThreadProfileArgs>,
    #[serde(default)]
    pub major_diameter: Option<Quantity>,
    #[serde(default)]
    pub pitch: Option<Quantity>,
    #[serde(default)]
    pub length: Option<Quantity>,
    #[serde(default)]
    pub hand: Option<ThreadHandArg>,
    #[serde(default)]
    pub starts: Option<u32>,
    #[serde(default)]
    pub radial_allowance: Option<Quantity>,
    #[serde(default)]
    pub axial_allowance: Option<Quantity>,
    #[serde(default)]
    pub minimum_wall: Option<Quantity>,
    #[serde(default)]
    pub representation: Option<ThreadRepresentationArg>,
    #[serde(default)]
    pub origin: Option<[Quantity; 3]>,
    /// Principal plane normal defines the thread axis. Do not combine with axis_u/axis_v.
    #[serde(default)]
    pub plane: Option<Plane>,
    #[serde(default)]
    pub axis_u: Option<[f64; 3]>,
    #[serde(default)]
    pub axis_v: Option<[f64; 3]>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectThreadArgs {
    pub thread: String,
}

pub(super) fn parameters(
    args: &ThreadRecipeArgs,
    previous: Option<&ThreadParameters>,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<ThreadParameters, ToolError> {
    let mut p = previous.cloned().unwrap_or_default();
    if let Some(family) = args.family {
        p.family = family.into();
    }
    p.custom = if p.family == ThreadFamily::Custom {
        match &args.custom_profile {
            Some(profile) => Some(model::CustomThreadProfile {
                depth: profile.depth.dimension(document, evaluated)?.0,
                crest_width: profile.crest_width.dimension(document, evaluated)?.0,
                lower_flank: profile
                    .lower_flank
                    .dimension(document, evaluated, Unit::Degree)?,
                upper_flank: profile
                    .upper_flank
                    .dimension(document, evaluated, Unit::Degree)?,
                radial_taper: profile
                    .radial_taper
                    .dimension(document, evaluated, Unit::Ratio)?,
            }),
            None => Some(p.custom.ok_or_else(|| {
                ToolError::invalid("Custom thread creation needs an explicit custom_profile")
            })?),
        }
    } else {
        if args.custom_profile.is_some() {
            return Err(ToolError::invalid(
                "custom_profile is valid only for family custom",
            ));
        }
        None
    };
    for (field, value) in [
        (&mut p.major_diameter, &args.major_diameter),
        (&mut p.pitch, &args.pitch),
        (&mut p.length, &args.length),
        (&mut p.radial_allowance, &args.radial_allowance),
        (&mut p.axial_allowance, &args.axial_allowance),
        (&mut p.minimum_wall, &args.minimum_wall),
    ] {
        if let Some(value) = value {
            *field = value.dimension(document, evaluated)?.0;
        }
    }
    if let Some(hand) = args.hand {
        p.hand = match hand {
            ThreadHandArg::Right => ThreadHand::Right,
            ThreadHandArg::Left => ThreadHand::Left,
        };
    }
    if let Some(starts) = args.starts {
        p.starts = starts;
    }
    if let Some(representation) = args.representation {
        p.representation = match representation {
            ThreadRepresentationArg::Modeled => ThreadRepresentation::Modeled,
            ThreadRepresentationArg::CosmeticEnvelope => ThreadRepresentation::CosmeticEnvelope,
        };
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
            "Choose plane or an explicit axis_u/axis_v basis, not both",
        ));
    }
    if let Some(plane) = args.plane {
        let frame = Datum::from(plane).frame();
        p.frame.u = frame.u;
        p.frame.v = frame.v;
    }
    match (args.axis_u, args.axis_v) {
        (Some(u), Some(v)) => {
            p.frame.u = u;
            p.frame.v = v;
        }
        (None, None) => {}
        _ => {
            return Err(ToolError::invalid(
                "An explicit thread basis needs both axis_u and axis_v",
            ));
        }
    }
    p.validate()?;
    Ok(p)
}
pub(super) fn thread_profiles(_: &dyn Host, _: &Request, _: NoArgs) -> ToolResult {
    Ok(
        json!({"families":ThreadFamily::ALL.into_iter().map(|family|json!({"family":family,"label":family.label(),"supported":family.supported(),"source":family.source()})).collect::<Vec<_>>(),"metric_source":model::METRIC_THREAD_SOURCE_URL,"metric_presets":model::metric_thread_presets().into_iter().map(|p|json!({"designation":p.designation,"diameter_mm":p.diameter_mm,"pitch_mm":p.pitch_mm})).collect::<Vec<_>>(),"fit":"Nominal basic dimensions only; no standardized tolerance class","runout":"flat_cut_ends","manufacturing":"Cosmetic envelopes are refused for STEP, mesh and print output; select modeled"}),
    )
}
pub(super) fn inspect_thread(host: &dyn Host, _: &Request, args: InspectThreadArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let feature = id(&args.thread)?;
    let source = model::read_thread(&document, feature)?;
    let resolved = source.parameters.resolve(&evaluated, source.mode)?;
    Ok(
        json!({"revision":revision,"thread":feature,"part":source.output.body,"source":source,"resolved":{"nominal_major_diameter_mm":2.*resolved.nominal_major_radius_mm,"nominal_pitch_diameter_mm":resolved.nominal_pitch_diameter_mm,"major_radius_mm":resolved.major_radius_mm,"pitch_mm":resolved.pitch_mm,"lead_mm":resolved.lead_mm,"starts":resolved.starts,"hand":resolved.hand,"representation":resolved.representation},"scope":"Canonical source and resolved dimensions; use evaluate_model for current native geometry"}),
    )
}
pub(super) fn set_thread(host: &dyn Host, call: &Request, args: ThreadArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            format!(
                "Request is based on revision {}, but the document is at {revision}",
                args.expected_revision
            ),
        )
        .at_revision(revision));
    }
    let edit = args.thread.as_deref().map(id).transpose()?;
    let previous = edit
        .map(|feature| model::read_thread(&document, feature))
        .transpose()?;
    let parameters = parameters(
        &args.recipe,
        previous.as_ref().map(|source| &source.parameters),
        &document,
        &evaluated,
    )?;
    let mode = args.mode.map(ThreadMode::from).unwrap_or_else(|| {
        previous
            .as_ref()
            .map_or(ThreadMode::NewExternal, |source| source.mode)
    });
    if let Some(source) = &previous
        && (args.name.is_some() || mode != source.mode || args.target_part.is_some())
    {
        return Err(ToolError::invalid(
            "A thread edit retains its name, mode, Part and original predecessor; omit name and target_part, and omit mode or keep its saved value",
        ));
    }
    let request = if edit.is_none() {
        let name = args
            .name
            .ok_or_else(|| ToolError::invalid("A new thread needs a name"))?;
        Some(if mode == ThreadMode::NewExternal {
            if args.target_part.is_some() {
                return Err(ToolError::invalid(
                    "A new shaft must not specify target_part",
                ));
            }
            model::ThreadRequest::new(name, parameters.clone())
        } else {
            let part = id(args
                .target_part
                .as_deref()
                .ok_or_else(|| ToolError::invalid("A receiving thread needs target_part"))?)?;
            model::ThreadRequest::receiving(
                name,
                mode,
                model::BodyInput {
                    body: part,
                    producer: current_producer(&document, part)?,
                },
                parameters.clone(),
            )?
        })
    } else {
        None
    };
    let expected = args.expected_revision;
    let owned = call.clone();
    let (prepared, feature, body) = with(host, move |target| {
        revision_guard(target, expected)?;
        let session = target.session();
        let prepared = match &request {
            Some(request) => {
                model::prepare_thread(session, expected, owned.actor.clone(), request)?
            }
            None => model::prepare_thread_edit(
                session,
                expected,
                owned.actor.clone(),
                edit.expect("thread edit"),
                &parameters,
            )?,
        };
        let output = prepared.outputs[0].clone();
        Ok::<_, ToolError>((prepared.prepared, output.feature, output.body))
    })??;
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    let committed = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":committed,"thread":feature,"part":body}))
}

#[cfg(test)]
mod recipe_wire_tests {
    use super::*;
    #[test]
    fn shared_recipe_preserves_the_flat_thread_contract_and_rejects_controls_inside_recipes() {
        let args:ThreadArgs=serde_json::from_value(json!({"expected_revision":0,"name":"Thread","mode":"new_external","major_diameter":{"value":6.,"unit":"mm"},"pitch":{"value":1.,"unit":"mm"},"length":{"value":5.2,"unit":"mm"}})).unwrap();
        assert!(args.recipe.major_diameter.is_some());
        assert!(matches!(args.mode, Some(ThreadModeArg::NewExternal)));
        assert!(
            serde_json::from_value::<ThreadRecipeArgs>(json!({"mode":"cut_internal"})).is_err()
        );
        assert!(
            serde_json::from_value::<ThreadArgs>(json!({"expected_revision":0,"bogus":1})).is_err()
        );
        let schema = serde_json::to_value(schemars::schema_for!(ThreadArgs)).unwrap();
        assert!(schema["properties"]["major_diameter"].is_object());
        assert!(schema["properties"].get("recipe").is_none());
    }
}
