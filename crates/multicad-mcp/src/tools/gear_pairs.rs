use super::{
    Plane, Quantity, Request, ToolResult, commit, compile,
    gears::{GearRecipeArgs, GearScalarArg, parameters as gear_parameters},
    id, revision_guard, snapshot,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    model::{self, GearHand, GearPairParameters, GearParameters},
    parameters::{EvaluatedParameters, Unit},
    sketch::Datum,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GearPairArgs {
    pub expected_revision: u64,
    /// Existing pair definition. Omitted fields retain their saved values.
    #[serde(default)]
    pub pair: Option<String>,
    /// Required for creation. Edits retain the pair and member names.
    #[serde(default)]
    pub name: Option<String>,
    /// First member recipe. Frames here describe the local Part definition.
    #[serde(default)]
    pub first: Option<GearRecipeArgs>,
    /// Creation inherits the first member's settings with 30 teeth and opposite hand, then applies these overrides. Edits preserve omitted member fields independently.
    #[serde(default)]
    pub second: Option<GearRecipeArgs>,
    #[serde(default)]
    pub drive_angle: Option<GearScalarArg>,
    /// Pair layout origin, in the reusable assembly's coordinates. Initial outer placement is identity; use normal component placement tools for independent occurrences.
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
pub struct InspectGearPairArgs {
    pub pair: String,
}
fn placement(
    args: &GearPairArgs,
    mut p: GearPairParameters,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<GearPairParameters, ToolError> {
    if let Some(value) = &args.drive_angle {
        p.drive_angle = value.dimension(document, evaluated, Unit::Degree)?;
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
            "Choose a pair plane or an explicit axis_u/axis_v basis, not both",
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
                "An explicit pair basis needs both axis_u and axis_v",
            ));
        }
    }
    p.validate()?;
    Ok(p)
}
pub(super) fn inspect_gear_pair(
    host: &dyn Host,
    _: &Request,
    args: InspectGearPairArgs,
) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let pair = id(&args.pair)?;
    let source = model::read_gear_pair(&document, pair)?;
    let r = model::resolve_gear_pair_parameters(&document, &evaluated, &source.parameters)?;
    let members = source
        .parameters
        .gears
        .iter()
        .map(|feature| {
            model::read_gear(&document, *feature)
                .map(|s| json!({"gear":feature,"part":s.output.body,"source":s}))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"revision":revision,"pair":pair,"source":source,"members":members,
        "resolved":{"center_distance_mm":r.center_distance_mm,"phases_rad":r.phases_rad,"mating_frames":r.mating},
        "scope":"Canonical source and resolved placement only. evaluate_model reports actual native pair checks and physical leaves."}),
    )
}
pub(super) fn set_gear_pair(host: &dyn Host, call: &Request, args: GearPairArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            "Gear pair request is based on a stale revision",
        )
        .at_revision(revision));
    }
    let edit = args.pair.as_deref().map(id).transpose()?;
    let previous = edit
        .map(|pair| model::read_gear_pair(&document, pair))
        .transpose()?;
    if edit.is_some() && args.name.is_some() {
        return Err(ToolError::invalid(
            "Pair edits retain names and member/output identities; omit name",
        ));
    }
    let old_gears = if let Some(source) = &previous {
        Some([
            model::read_gear(&document, source.parameters.gears[0])?,
            model::read_gear(&document, source.parameters.gears[1])?,
        ])
    } else {
        None
    };
    let empty = GearRecipeArgs::default();
    let default_first = GearParameters {
        teeth: Dimension::Literal {
            value: 20.,
            unit: "count".into(),
        },
        ..Default::default()
    };
    let first = gear_parameters(
        args.first.as_ref().unwrap_or(&empty),
        Some(
            old_gears
                .as_ref()
                .map_or(&default_first, |g| &g[0].parameters),
        ),
        &document,
        &evaluated,
    )?;
    let default_second = GearParameters {
        teeth: Dimension::Literal {
            value: 30.,
            unit: "count".into(),
        },
        hand: match first.hand {
            GearHand::None => GearHand::None,
            GearHand::Left => GearHand::Right,
            GearHand::Right => GearHand::Left,
        },
        ..first.clone()
    };
    let second = gear_parameters(
        args.second.as_ref().unwrap_or(&empty),
        Some(
            old_gears
                .as_ref()
                .map_or(&default_second, |g| &g[1].parameters),
        ),
        &document,
        &evaluated,
    )?;
    let request = if edit.is_none() {
        let name = args
            .name
            .clone()
            .ok_or_else(|| ToolError::invalid("A new gear pair needs a name"))?;
        let mut request = model::GearPairRequest::new(
            &name,
            [
                model::GearRequest::new(format!("{name} A"), first.clone()),
                model::GearRequest::new(format!("{name} B"), second.clone()),
            ],
        );
        request.parameters = placement(&args, request.parameters, &document, &evaluated)?;
        Some(request)
    } else {
        None
    };
    let parameters = if let Some(source) = &previous {
        Some(placement(
            &args,
            source.parameters.clone(),
            &document,
            &evaluated,
        )?)
    } else {
        None
    };
    let update_geometry = args.first.is_some() || args.second.is_some();
    let expected = args.expected_revision;
    let owned = call.clone();
    let (prepared, mut result) = with(host, move |target| {
        revision_guard(target, expected)?;
        let prepared = match request {
            Some(request) => {
                model::prepare_gear_pair(target.session(), expected, owned.actor.clone(), &request)?
            }
            None => {
                let pair = edit.expect("pair edit");
                let parameters = parameters.as_ref().expect("pair parameters");
                if update_geometry {
                    model::prepare_gear_pair_geometry_edit(
                        target.session(),
                        expected,
                        owned.actor.clone(),
                        pair,
                        parameters,
                        &[first, second],
                    )?
                } else {
                    model::prepare_gear_pair_edit(
                        target.session(),
                        expected,
                        owned.actor.clone(),
                        pair,
                        parameters,
                    )?
                }
            }
        };
        let members = prepared
            .outputs
            .iter()
            .map(|o| json!({"gear":o.feature,"part":o.body}))
            .collect::<Vec<_>>();
        let result = json!({"pair":prepared.pair,"assembly":prepared.assembly,"members":members,"instances":prepared.instances,"occurrence":prepared.occurrence});
        Ok::<_, ToolError>((prepared.prepared, result))
    })??;
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    let committed = with(host, move |target| commit(target, &owned, prepared))??;
    result["revision"] = json!(committed);
    Ok(result)
}
