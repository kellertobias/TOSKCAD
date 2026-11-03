//! Native-checked generic hardware geometry; catalog placement remains separate.
use super::{
    NoArgs, Plane, Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot,
    threads::{ThreadRecipeArgs, parameters as thread_parameters},
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::Document,
    model::{
        self, MechanicalPartParameters, PrimitiveFrame, ThreadParameters, ThreadRepresentation,
        length,
    },
    parameters::EvaluatedParameters,
    sketch::Datum,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MechanicalFrameArgs {
    #[serde(default)]
    pub origin: Option<[Quantity; 3]>,
    #[serde(default)]
    pub plane: Option<Plane>,
    #[serde(default)]
    pub axis_u: Option<[f64; 3]>,
    #[serde(default)]
    pub axis_v: Option<[f64; 3]>,
}
impl MechanicalFrameArgs {
    fn apply(
        &self,
        mut frame: PrimitiveFrame,
        doc: &Document,
        eval: &EvaluatedParameters,
    ) -> Result<PrimitiveFrame, ToolError> {
        if let Some(p) = &self.origin {
            frame.origin = [
                p[0].dimension(doc, eval)?.0,
                p[1].dimension(doc, eval)?.0,
                p[2].dimension(doc, eval)?.0,
            ];
        }
        if self.plane.is_some() && (self.axis_u.is_some() || self.axis_v.is_some()) {
            return Err(ToolError::invalid(
                "Choose a plane or both explicit basis vectors",
            ));
        }
        if let Some(plane) = self.plane {
            let p = Datum::from(plane).frame();
            frame.u = p.u;
            frame.v = p.v;
        }
        match (self.axis_u, self.axis_v) {
            (Some(u), Some(v)) => {
                frame.u = u;
                frame.v = v;
            }
            (None, None) => {}
            _ => {
                return Err(ToolError::invalid(
                    "An explicit basis needs both axis_u and axis_v",
                ));
            }
        }
        frame.validate()?;
        Ok(frame)
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MechanicalRecipeArgs {
    Washer {
        #[serde(default)]
        bore_diameter: Option<Quantity>,
        #[serde(default)]
        outside_diameter: Option<Quantity>,
        #[serde(default)]
        thickness: Option<Quantity>,
        #[serde(flatten)]
        frame: MechanicalFrameArgs,
    },
    HexNut {
        #[serde(default)]
        across_flats: Option<Quantity>,
        /// Thread length is nut height; its frame locates the whole nut.
        #[serde(default)]
        thread: Box<ThreadRecipeArgs>,
    },
    BearingEnvelope {
        #[serde(default)]
        bore_diameter: Option<Quantity>,
        #[serde(default)]
        outside_diameter: Option<Quantity>,
        #[serde(default)]
        width: Option<Quantity>,
        #[serde(flatten)]
        frame: MechanicalFrameArgs,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MechanicalPartArgs {
    pub expected_revision: u64,
    /// Existing generator feature ID, not its output Part ID.
    #[serde(default)]
    pub feature: Option<String>,
    /// Required for creation; edits retain the saved name.
    #[serde(default)]
    pub name: Option<String>,
    /// Kind is explicit even on a partial edit. Omitted values retain source.
    pub recipe: MechanicalRecipeArgs,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectMechanicalPartArgs {
    pub feature: String,
}
fn parameters(
    args: &MechanicalRecipeArgs,
    previous: Option<&MechanicalPartParameters>,
    doc: &Document,
    eval: &EvaluatedParameters,
) -> Result<MechanicalPartParameters, ToolError> {
    let mm = |q: &Option<Quantity>, old: &multicad_core::constraint_sketch::Dimension| {
        q.as_ref()
            .map(|v| v.dimension(doc, eval).map(|d| d.0))
            .transpose()
            .map(|v| v.unwrap_or_else(|| old.clone()))
    };
    let mismatch = || ToolError::invalid("A mechanical Part edit retains its generator family");
    let p = match args {
        MechanicalRecipeArgs::Washer {
            bore_diameter,
            outside_diameter,
            thickness,
            frame,
        } => {
            let default = MechanicalPartParameters::Washer {
                bore_diameter: length(3.2),
                outside_diameter: length(7.),
                thickness: length(0.5),
                frame: Default::default(),
            };
            let MechanicalPartParameters::Washer {
                bore_diameter: b,
                outside_diameter: o,
                thickness: h,
                frame: f,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            MechanicalPartParameters::Washer {
                bore_diameter: mm(bore_diameter, b)?,
                outside_diameter: mm(outside_diameter, o)?,
                thickness: mm(thickness, h)?,
                frame: frame.apply(f.clone(), doc, eval)?,
            }
        }
        MechanicalRecipeArgs::BearingEnvelope {
            bore_diameter,
            outside_diameter,
            width,
            frame,
        } => {
            let default = MechanicalPartParameters::BearingEnvelope {
                bore_diameter: length(10.),
                outside_diameter: length(30.),
                width: length(9.),
                frame: Default::default(),
            };
            let MechanicalPartParameters::BearingEnvelope {
                bore_diameter: b,
                outside_diameter: o,
                width: h,
                frame: f,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            MechanicalPartParameters::BearingEnvelope {
                bore_diameter: mm(bore_diameter, b)?,
                outside_diameter: mm(outside_diameter, o)?,
                width: mm(width, h)?,
                frame: frame.apply(f.clone(), doc, eval)?,
            }
        }
        MechanicalRecipeArgs::HexNut {
            across_flats,
            thread,
        } => {
            let default = MechanicalPartParameters::HexNut {
                across_flats: length(10.),
                thread: Box::new(ThreadParameters {
                    major_diameter: length(6.),
                    pitch: length(1.),
                    length: length(5.2),
                    representation: ThreadRepresentation::CosmeticEnvelope,
                    ..Default::default()
                }),
            };
            let MechanicalPartParameters::HexNut {
                across_flats: a,
                thread: t,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            MechanicalPartParameters::HexNut {
                across_flats: mm(across_flats, a)?,
                thread: Box::new(thread_parameters(thread, Some(t), doc, eval)?),
            }
        }
    };
    p.resolve(eval)?;
    Ok(p)
}
pub(super) fn mechanical_part_profiles(_: &dyn Host, _: &Request, _: NoArgs) -> ToolResult {
    Ok(
        json!({"supported_geometry":["washer","hex_nut","bearing_envelope"],"catalog_status":"Generic parametric geometry only; verified standard-parts catalog is under development","nut_thread":"Uses the shared thread recipe. thread.length is nut height and thread frame locates its body; creation defaults to a cosmetic envelope. Select modeled for manufacturing export.","bearing":"External mounting envelope only; races, rolling elements and seals are not modeled","limits":"No standard compliance, stock availability, fit, strength or lifetime certification"}),
    )
}
fn frame_value(frame: multicad_core::sketch::Frame) -> Value {
    json!({"origin_mm":frame.origin,"axis_u":frame.u,"axis_v":frame.v,"axis_n":frame.n})
}
pub(super) fn inspect_mechanical_part(
    host: &dyn Host,
    _: &Request,
    args: InspectMechanicalPartArgs,
) -> ToolResult {
    let (doc, revision, eval) = snapshot(host)?;
    let feature = id(&args.feature)?;
    let source = model::read_mechanical_part(&doc, feature)?;
    let resolved = match source.parameters.resolve(&eval)? {
        model::ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            frame,
        } => {
            json!({"kind":"washer","bore_diameter_mm":2.*bore_radius_mm,"outside_diameter_mm":2.*outside_radius_mm,"thickness_mm":thickness_mm,"frame":frame_value(frame)})
        }
        model::ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            frame,
        } => {
            json!({"kind":"bearing_envelope","bore_diameter_mm":2.*bore_radius_mm,"outside_diameter_mm":2.*outside_radius_mm,"width_mm":width_mm,"frame":frame_value(frame)})
        }
        model::ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } => {
            json!({"kind":"hex_nut","across_flats_mm":across_flats_mm,"height_mm":thread.length_mm,"nominal_thread_diameter_mm":2.*thread.nominal_major_radius_mm,"nominal_pitch_diameter_mm":thread.nominal_pitch_diameter_mm,"actual_pitch_diameter_mm":thread.actual_pitch_diameter_mm,"fit":thread.fit,"pitch_mm":thread.pitch_mm,"hand":thread.hand,"representation":thread.representation,"frame":frame_value(thread.frame)})
        }
    };
    Ok(
        json!({"revision":revision,"feature":feature,"matches_reference_dimensions":source.matches_reference_dimensions(&eval)?,"source":source,"resolved":resolved,"native_validation":"Not performed by this source inspection; use evaluate_model","catalog_verification":"A retained reference and dimensional match do not establish verified catalog membership or stock availability"}),
    )
}
pub(super) fn set_mechanical_part(
    host: &dyn Host,
    call: &Request,
    args: MechanicalPartArgs,
) -> ToolResult {
    let (doc, revision, eval) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            "Mechanical Part request is based on a stale revision",
        )
        .at_revision(revision));
    }
    let edit = args.feature.as_deref().map(id).transpose()?;
    let previous = edit
        .map(|f| model::read_mechanical_part(&doc, f))
        .transpose()?;
    if edit.is_some() && args.name.is_some() {
        return Err(ToolError::invalid(
            "Mechanical Part edits retain their name and output Part; omit name",
        ));
    }
    let parameters = parameters(
        &args.recipe,
        previous.as_ref().map(|p| &p.parameters),
        &doc,
        &eval,
    )?;
    let request = if edit.is_none() {
        Some(model::MechanicalPartRequest::new(
            args.name
                .ok_or_else(|| ToolError::invalid("A new mechanical Part needs a name"))?,
            parameters.clone(),
        ))
    } else {
        None
    };
    let expected = args.expected_revision;
    let owned = call.clone();
    let (prepared, feature, part) = with(host, move |target| {
        revision_guard(target, expected)?;
        let p = match request {
            Some(r) => {
                model::prepare_mechanical_part(target.session(), expected, owned.actor.clone(), &r)?
            }
            None => model::prepare_mechanical_part_edit(
                target.session(),
                expected,
                owned.actor.clone(),
                edit.expect("mechanical Part edit"),
                &parameters,
            )?,
        };
        let output = p.outputs[0].clone();
        Ok::<_, ToolError>((p.prepared, output.feature, output.body))
    })??;
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    let committed = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":committed,"feature":feature,"part":part}))
}
