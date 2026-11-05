use super::{
    Plane, Quantity, Request, ToolResult, commit, compile,
    gears::{GearRecipeArgs, GearScalarArg, patched_parameters as gear_parameters},
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
    /// Creation inherits the first member settings with 30 teeth (24 for ring/rack mates) and opposite hand. Worm pairs use opposite roles with independent axial widths; right hand is shared. Edits preserve omitted fields and mate counts follow opposite original typed teeth.
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
        "resolved":{"center_distance_mm":r.center_distance_mm,"phases_rad":r.phases_rad,"mating_frames":r.mating,"rack_travel":r.rack_travel,"bevel":r.bevel,"worm":r.worm},
        "scope":"Canonical source and resolved placement only. evaluate_model reports actual native pair checks and physical leaves."}),
    )
}
/// Pair-specific omitted fields derive from their typed sources, while explicit
/// conflicting sources fail before native work. Equal evaluated numbers do not
/// make two distinct canonical expressions interchangeable.
fn couple_bevel_intent(
    recipes: [GearParameters; 2],
    supplied: [Option<&GearRecipeArgs>; 2],
) -> Result<[GearParameters; 2], ToolError> {
    if !recipes.iter().any(|r| r.family == model::GearFamily::Bevel) {
        return Ok(recipes);
    }
    if recipes
        .iter()
        .any(|r| r.family != model::GearFamily::Bevel || r.bevel.is_none())
    {
        return Err(ToolError::invalid(
            "A bevel pair needs two spherical-involute straight bevel recipes",
        ));
    }
    for member in 0..2 {
        if supplied[member].is_some_and(|r| r.mate_teeth.is_some())
            && recipes[member]
                .bevel
                .as_ref()
                .expect("checked bevel")
                .mate_teeth
                != recipes[1 - member].teeth
        {
            return Err(ToolError::invalid(
                "Explicit bevel mate_teeth must match the opposite original typed teeth source; omit it to derive that source",
            ));
        }
    }
    let explicit_shaft = supplied.map(|r| r.is_some_and(|r| r.shaft_angle.is_some()));
    let shaft_a = &recipes[0]
        .bevel
        .as_ref()
        .expect("checked bevel")
        .shaft_angle;
    let shaft_b = &recipes[1]
        .bevel
        .as_ref()
        .expect("checked bevel")
        .shaft_angle;
    if explicit_shaft[0] && explicit_shaft[1] && shaft_a != shaft_b {
        return Err(ToolError::invalid(
            "Explicit bevel shaft angles must share one identical typed source; omit either member's shaft_angle to derive it",
        ));
    }
    let shaft = if explicit_shaft[1] && !explicit_shaft[0] {
        shaft_b
    } else {
        shaft_a
    };
    model::couple_bevel_pair_recipes(&recipes, shaft).map_err(Into::into)
}

fn couple_worm_intent(
    recipes: [GearParameters; 2],
    supplied: [Option<&GearRecipeArgs>; 2],
) -> Result<[GearParameters; 2], ToolError> {
    if !recipes.iter().any(|p| p.family == model::GearFamily::Worm) {
        return Ok(recipes);
    }
    if recipes
        .iter()
        .any(|p| p.family != model::GearFamily::Worm || p.worm.is_none())
    {
        return Err(ToolError::invalid(
            "A worm pair needs one ZA worm and one ZA generated wheel",
        ));
    }
    let w = [
        recipes[0].worm.as_ref().expect("checked worm"),
        recipes[1].worm.as_ref().expect("checked worm"),
    ];
    if w[0].role == w[1].role {
        return Err(ToolError::invalid(
            "Worm pair member roles must be opposite",
        ));
    }
    for i in 0..2 {
        if supplied[i].is_some_and(|a| a.mate_teeth.is_some())
            && w[i].mate_teeth != recipes[1 - i].teeth
        {
            return Err(ToolError::invalid(
                "Explicit worm mate_teeth must match the opposite original typed teeth source; omit it to derive",
            ));
        }
    }
    let worm = if w[0].role == model::WormMember::Worm {
        0
    } else {
        1
    };
    let wheel = 1 - worm;
    if supplied[wheel].is_some_and(|a| a.diameter_quotient.is_some())
        && w[wheel].diameter_quotient != w[worm].diameter_quotient
    {
        return Err(ToolError::invalid(
            "Explicit wheel diameter_quotient must match the worm's original typed quotient; omit to derive",
        ));
    }
    model::couple_worm_pair_recipes(&recipes).map_err(Into::into)
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
            value: if args
                .first
                .as_ref()
                .and_then(|r| r.family)
                .is_some_and(|f| model::GearFamily::from(f) == model::GearFamily::Bevel)
            {
                24.
            } else {
                20.
            },
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
    if edit.is_none()
        && first.family == model::GearFamily::Worm
        && args.first.as_ref().is_none_or(|a| a.worm_role.is_none())
    {
        return Err(ToolError::invalid(
            "A new worm pair needs explicit first.worm_role; second defaults to the opposite role",
        ));
    }
    let default_second = GearParameters {
        worm: first.worm.as_ref().map(|w| model::WormDimensions {
            role: if w.role == model::WormMember::Worm {
                model::WormMember::Wheel
            } else {
                model::WormMember::Worm
            },
            tooth_system: w.tooth_system,
            mate_teeth: first.teeth.clone(),
            diameter_quotient: w.diameter_quotient.clone(),
        }),
        face_width: if first.family == model::GearFamily::Worm {
            model::length(
                if first
                    .worm
                    .as_ref()
                    .is_some_and(|w| w.role == model::WormMember::Worm)
                {
                    1.
                } else {
                    24.
                },
            )
        } else {
            first.face_width.clone()
        },
        rack: None,
        family: if matches!(
            first.family,
            model::GearFamily::InternalRing | model::GearFamily::Rack
        ) {
            model::GearFamily::Spur
        } else {
            first.family
        },
        outer_diameter: None,
        bore_diameter: if first.family == model::GearFamily::InternalRing {
            model::length(0.)
        } else {
            first.bore_diameter.clone()
        },
        teeth: Dimension::Literal {
            value: if first.family == model::GearFamily::Worm {
                if first
                    .worm
                    .as_ref()
                    .is_some_and(|w| w.role == model::WormMember::Worm)
                {
                    32.
                } else {
                    1.
                }
            } else if matches!(
                first.family,
                model::GearFamily::InternalRing | model::GearFamily::Rack
            ) {
                24.
            } else {
                30.
            },
            unit: "count".into(),
        },
        hand: if first.family == model::GearFamily::Worm {
            GearHand::Right
        } else {
            match first.hand {
                GearHand::None => GearHand::None,
                GearHand::Left => GearHand::Right,
                GearHand::Right => GearHand::Left,
            }
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
    let [first, second] =
        couple_bevel_intent([first, second], [args.first.as_ref(), args.second.as_ref()])?;
    let [first, second] =
        couple_worm_intent([first, second], [args.first.as_ref(), args.second.as_ref()])?;
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

#[cfg(test)]
mod worm_coupling_tests {
    use super::*;
    use multicad_core::{
        document::{Entity, EntityData, Mutation},
        parameters::BoundExpression,
    };
    #[test]
    fn worm_pair_couples_original_typed_sources_and_refuses_equal_value_conflicts() {
        let d = Document::new("Typed worm pair")
            .unwrap()
            .edited(&[
                Mutation::Insert(Entity::new(
                    "wheel_count",
                    EntityData::Parameter {
                        expression: "40".into(),
                        unit: "count".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
                Mutation::Insert(Entity::new(
                    "worm_q",
                    EntityData::Parameter {
                        expression: "10".into(),
                        unit: "ratio".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
            ])
            .unwrap();
        let s = multicad_core::commands::Session::new(d.clone()).unwrap();
        let parse = |v| serde_json::from_value::<GearRecipeArgs>(v).unwrap();
        let a=gear_parameters(&parse(json!({"family":"worm","worm_role":"worm","diameter_quotient":{"expression":"=$worm_q","unit":"ratio"}})),None,&d,s.parameters()).unwrap();
        let mut b = a.clone();
        b.worm.as_mut().unwrap().role = model::WormMember::Wheel;
        b.teeth = Dimension::Expression {
            expression: BoundExpression::bind(&d, "=$wheel_count", "count", None).unwrap(),
        };
        b.face_width = model::length(1.);
        let coupled = couple_worm_intent([a.clone(), b.clone()], [None, None]).unwrap();
        assert_eq!(coupled[0].worm.as_ref().unwrap().mate_teeth, b.teeth);
        assert_eq!(
            coupled[1].worm.as_ref().unwrap().diameter_quotient,
            a.worm.as_ref().unwrap().diameter_quotient
        );
        let explicit_mate = parse(json!({"mate_teeth":{"value":40.,"unit":"count"}}));
        let mut bad_a = a.clone();
        bad_a.worm.as_mut().unwrap().mate_teeth = Dimension::Literal {
            value: 40.,
            unit: "count".into(),
        };
        assert!(couple_worm_intent([bad_a, b.clone()], [Some(&explicit_mate), None]).is_err());
        let explicit_q = parse(json!({"diameter_quotient":{"value":10.,"unit":"ratio"}}));
        b.worm.as_mut().unwrap().diameter_quotient = model::ratio(10.);
        assert!(couple_worm_intent([a, b], [None, Some(&explicit_q)]).is_err());
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
}
