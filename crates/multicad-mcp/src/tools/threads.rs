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
    model::{
        self, ThreadFamily, ThreadFit, ThreadHand, ThreadMode, ThreadParameters,
        ThreadRepresentation,
    },
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
    Unc,
    Unf,
    Bsw,
    Bsf,
    /// Legacy ambiguous family; unavailable. Choose unc or unf explicitly.
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
            ThreadFamilyArg::Unc => Self::Unc,
            ThreadFamilyArg::Unf => Self::Unf,
            ThreadFamilyArg::Bsw => Self::Bsw,
            ThreadFamilyArg::Bsf => Self::Bsf,
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
pub enum ThreadFitArg {
    #[serde(rename = "nominal_basic")]
    NominalBasic,
    #[serde(rename = "unified_2a")]
    Unified2A,
    #[serde(rename = "unified_2b")]
    Unified2B,
    #[serde(rename = "whitworth_medium_external")]
    WhitworthMediumExternal,
    #[serde(rename = "whitworth_medium_internal")]
    WhitworthMediumInternal,
    #[serde(rename = "bspp_class_b_external")]
    BsppClassBExternal,
    #[serde(rename = "bspp_internal")]
    BsppInternal,
    #[serde(rename = "bspt_nominal")]
    BsptNominal,
    #[serde(rename = "npt_historical_midpoint")]
    NptHistoricalMidpoint,
}
impl From<ThreadFitArg> for ThreadFit {
    fn from(f: ThreadFitArg) -> Self {
        match f {
            ThreadFitArg::NominalBasic => Self::NominalBasic,
            ThreadFitArg::Unified2A => Self::Unified2A,
            ThreadFitArg::Unified2B => Self::Unified2B,
            ThreadFitArg::WhitworthMediumExternal => Self::WhitworthMediumExternal,
            ThreadFitArg::WhitworthMediumInternal => Self::WhitworthMediumInternal,
            ThreadFitArg::BsppClassBExternal => Self::BsppClassBExternal,
            ThreadFitArg::BsppInternal => Self::BsppInternal,
            ThreadFitArg::BsptNominal => Self::BsptNominal,
            ThreadFitArg::NptHistoricalMidpoint => Self::NptHistoricalMidpoint,
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
    /// Sourced families require their explicit external/internal class. Omitted edits retain the saved class.
    #[serde(default)]
    pub fit: Option<ThreadFitArg>,
    /// Explicit ISO 7-1:1994 nominal gauge R/Rc intent. Omitted edits retain it.
    #[serde(default)]
    pub pipe: Option<PipeThreadArgs>,
    /// Historical plain-face NPT with explicit midpoint truncation construction.
    #[serde(default)]
    pub npt: Option<NptThreadArgs>,
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

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PipeFormArg {
    R,
    Rc,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
pub enum PipeSizeArg {
    #[serde(rename = "1/8")]
    OneEighth,
    #[serde(rename = "1/4")]
    OneQuarter,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PipeGaugeArg {
    Iso7Nominal1994,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PipeThreadArgs {
    pub form: PipeFormArg,
    pub size: PipeSizeArg,
    pub gauge: PipeGaugeArg,
}
impl From<PipeThreadArgs> for model::PipeThreadIntent {
    fn from(p: PipeThreadArgs) -> Self {
        Self {
            form: match p.form {
                PipeFormArg::R => model::PipeThreadForm::R,
                PipeFormArg::Rc => model::PipeThreadForm::Rc,
            },
            size: match p.size {
                PipeSizeArg::OneEighth => model::PipeThreadSize::OneEighth,
                PipeSizeArg::OneQuarter => model::PipeThreadSize::OneQuarter,
            },
            gauge: model::PipeGaugeConvention::Iso7Nominal1994,
        }
    }
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NptFormArg {
    External,
    Internal,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NptGaugeArg {
    Nbs1959PlainFace,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NptTruncationArg {
    MidpointConeParallel,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NptThreadArgs {
    pub form: NptFormArg,
    pub size: PipeSizeArg,
    pub gauge: NptGaugeArg,
    pub truncation: NptTruncationArg,
}
impl From<NptThreadArgs> for model::NptThreadIntent {
    fn from(p: NptThreadArgs) -> Self {
        Self {
            form: match p.form {
                NptFormArg::External => model::NptThreadForm::External,
                NptFormArg::Internal => model::NptThreadForm::Internal,
            },
            size: match p.size {
                PipeSizeArg::OneEighth => model::NptThreadSize::OneEighth,
                PipeSizeArg::OneQuarter => model::NptThreadSize::OneQuarter,
            },
            gauge: model::NptGaugeConvention::Nbs1959PlainFace,
            truncation: model::NptTruncationTarget::MidpointConeParallel,
        }
    }
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
    if let Some(fit) = args.fit {
        p.fit = fit.into();
    }
    if let Some(npt) = args.npt {
        p.npt = Some(npt.into());
    } else if p.family != ThreadFamily::Npt {
        p.npt = None;
    }
    if p.family != ThreadFamily::Npt && args.npt.is_some() {
        return Err(ToolError::invalid(
            "npt intent is valid only for family npt",
        ));
    }
    if let Some(pipe) = args.pipe {
        p.pipe = Some(pipe.into());
    } else if p.family != ThreadFamily::Bspt {
        p.pipe = None;
    }
    if p.family != ThreadFamily::Bspt && args.pipe.is_some() {
        return Err(ToolError::invalid(
            "pipe intent is valid only for family bspt",
        ));
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
    let families:Vec<_> = ThreadFamily::ALL.into_iter().map(|family| {
        let fit_classes:Vec<_> = if family.supported() && family.is_unified() {
            vec![ThreadFit::Unified2A, ThreadFit::Unified2B]
        } else if matches!(family,ThreadFamily::Bsw|ThreadFamily::Bsf) {
            vec![ThreadFit::WhitworthMediumExternal,ThreadFit::WhitworthMediumInternal]
        } else if family==ThreadFamily::Bspp {
            vec![ThreadFit::BsppClassBExternal,ThreadFit::BsppInternal]
        } else if family==ThreadFamily::Bspt {
            vec![ThreadFit::BsptNominal]
        } else if family==ThreadFamily::Npt {
            vec![ThreadFit::NptHistoricalMidpoint]
        } else if family.supported() {
            vec![ThreadFit::NominalBasic]
        } else {vec![]};
        json!({"family":family,"label":family.label(),"supported":family.supported(),"source":family.source(),"fit_classes":fit_classes})
    }).collect();
    let unified_presets:Vec<_> = model::unified_thread_presets().into_iter().map(|p|json!({
        "family":p.family,"designation":p.designation,"diameter_mm":p.diameter_mm,"pitch_mm":p.pitch_mm,
        "nominal_diameter_inch":p.nominal_diameter_inch,"threads_per_inch":p.threads_per_inch,
        "external":{"class":ThreadFit::Unified2A,"limits":p.external,"allowance_mm":p.external_allowance_mm,"minor_reference_mm":p.external_minor_reference_mm},
        "internal":{"class":ThreadFit::Unified2B,"limits":p.internal},
        "engagement_limit_mm":1.5*p.diameter_mm,
    })).collect();
    let rounded_presets:Vec<_> = model::whitworth_thread_presets().into_iter().map(|p|json!({
        "family":p.family,"designation":p.designation,"diameter_mm":p.diameter_mm,"pitch_mm":p.pitch_mm,
        "nominal_diameter_inch":p.nominal_diameter_inch,"threads_per_inch":p.threads_per_inch,
        "external":{"class":if p.family==ThreadFamily::Bspp {ThreadFit::BsppClassBExternal}else{ThreadFit::WhitworthMediumExternal},"limits":p.external},
        "internal":{"class":if p.family==ThreadFamily::Bspp {ThreadFit::BsppInternal}else{ThreadFit::WhitworthMediumInternal},"limits":p.internal},
        "engagement_limit_mm":if p.family==ThreadFamily::Bspp {None}else{Some(15.*p.pitch_mm)},
        "source":p.source,"source_url":p.source_url,"profile_source_url":p.profile_source_url,
    })).collect();
    Ok(json!({"families":families,
        "metric_source":model::METRIC_THREAD_SOURCE_URL,
        "metric_presets":model::metric_thread_presets().into_iter().map(|p|json!({"designation":p.designation,"diameter_mm":p.diameter_mm,"pitch_mm":p.pitch_mm})).collect::<Vec<_>>(),
        "unified_source":model::UNIFIED_THREAD_SOURCE,"unified_source_url":model::UNIFIED_THREAD_SOURCE_URL,"unified_profile_source_url":model::UNIFIED_PROFILE_SOURCE_URL,
        "unified_presets":unified_presets,
        "rounded_presets":rounded_presets,
        "bspt_presets":model::pipe_thread_presets(),
        "npt_presets":model::npt_thread_presets(),
        "npt_support":{"forms":["external","internal"],"hands":["right"],"starts":[1],"gauge":"nbs1959_plain_face","truncation":"midpoint_cone_parallel","source":model::NPT_THREAD_SOURCE,"source_url":model::NPT_THREAD_SOURCE_URL,"included_angle_deg":60,"diameter_taper":"1:16","external_length":"L2; crests clipped to pipe stock OD; no L2..L4 vanish","internal_length":"at least L1 plus three pitches plus positive gauge travel","gauge_tolerance_turns":1,"radial_allowance":"0 to pitch / 32","axial_allowance_mm":[0],"nptf":"unavailable"},
        "bspt_support":{"forms":["r","rc"],"hands":["right"],"gauge":"iso7_nominal1994","source":model::BSPT_THREAD_SOURCE,"source_url":model::BSPT_THREAD_SOURCE_URL,"diameter_taper":"1:16","radial_taper":{"r":0.03125,"rc":-0.03125},"reference":{"r":"small end","rc":"fitting face; gauge half a pitch inward"},"starts":[1],"axial_allowance_mm":[0],"rp":"unavailable","sealing":"Appropriate jointing medium required; no pressure, physical fit or gage certification"},
        "rounded_support":{"starts":[1],"included_angle_deg":55,"crest_and_root":"tangent_circular_arcs","axial_allowance_mm":[0],"coating":"uncoated","bspp_sealing":"Parallel fastening thread; sealing is outside the thread. G labels are nominal pipe sizes, not physical inch diameters.","claim":"BSW/BSF: bounded BS 84:2007 Medium rows. BSPP: bounded Elesa+Ganter 2018 table and NBK JIS B 0202:1999 basic form; no pressure-seal, full ISO compliance or physical-fit certification."},
        "unified_support":{"starts":[1],"form":"un_flat_root","coating":"uncoated","max_length_nominal_diameters":1.5,"limits":"Null range endpoints are unspecified; an external minor reference is not an acceptance limit","claim":"Historical sourced dimensions; no current ASME, calibrated-gage or physical-fit certification"},
        "fit":"ISO metric and Custom use nominal basic dimensions; UNC/UNF require 2A/2B; BSW/BSF require Medium external/internal; BSPP requires Class B external or the sourced internal class",
        "runout":"flat_cut_ends","manufacturing":"Cosmetic envelopes are refused for STEP, mesh and print output; select modeled"}))
}
pub(super) fn inspect_thread(host: &dyn Host, _: &Request, args: InspectThreadArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let feature = id(&args.thread)?;
    let source = model::read_thread(&document, feature)?;
    let resolved = source.parameters.resolve(&evaluated, source.mode)?;
    Ok(
        json!({"revision":revision,"thread":feature,"part":source.output.body,"source":source,"resolved":{"nominal_major_diameter_mm":2.*resolved.nominal_major_radius_mm,"nominal_pitch_diameter_mm":resolved.nominal_pitch_diameter_mm,"actual_pitch_diameter_mm":resolved.actual_pitch_diameter_mm,"fit":resolved.fit,"pipe":resolved.pipe,"npt":resolved.npt,"major_radius_mm":resolved.major_radius_mm,"pitch_mm":resolved.pitch_mm,"lead_mm":resolved.lead_mm,"starts":resolved.starts,"hand":resolved.hand,"representation":resolved.representation,"profile":{"depth_mm":resolved.profile.depth_mm,"crest_width_mm":resolved.profile.crest_width_mm,"rounding_radius_mm":resolved.profile.rounding_radius_mm,"radial_taper":resolved.profile.radial_taper,"lower_flank_deg":resolved.profile.lower_flank_deg,"upper_flank_deg":resolved.profile.upper_flank_deg}},"scope":"Canonical source and resolved construction dimensions; nominal pitch diameter is basic, actual_pitch_diameter_mm and fit describe sourced fitted construction, not a measured or certified manufactured thread; use evaluate_model for current native geometry"}),
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
        assert!(schema["properties"]["fit"].is_object());
        for (family, fit, expected_family, expected_fit) in [
            ("unc", "unified_2a", ThreadFamily::Unc, ThreadFit::Unified2A),
            ("unf", "unified_2b", ThreadFamily::Unf, ThreadFit::Unified2B),
        ] {
            let recipe: ThreadRecipeArgs =
                serde_json::from_value(json!({"family":family,"fit":fit})).unwrap();
            assert_eq!(ThreadFamily::from(recipe.family.unwrap()), expected_family);
            assert_eq!(ThreadFit::from(recipe.fit.unwrap()), expected_fit);
        }
        assert!(serde_json::from_value::<ThreadRecipeArgs>(json!({"fit":"2A"})).is_err());
    }
}
