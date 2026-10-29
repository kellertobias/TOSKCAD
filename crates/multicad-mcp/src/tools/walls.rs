//! Architectural wall tools over the shared wall/opening type API. Each
//! mutation is checked natively before it commits one transaction.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::EntityId,
    model::{
        self, BodyOutput, Hinge, Justification, Opening, OpeningKind, OpeningTypeDefinition, Swing,
        WallDefinition, WallRequest, WallTypeDefinition, length,
    },
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum JustificationArg {
    Center,
    Left,
    Right,
}
impl From<JustificationArg> for Justification {
    fn from(j: JustificationArg) -> Self {
        match j {
            JustificationArg::Center => Self::Center,
            JustificationArg::Left => Self::Left,
            JustificationArg::Right => Self::Right,
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WallTypeArgs {
    pub expected_revision: u64,
    pub name: String,
    pub thickness: Quantity,
    pub height: Quantity,
    /// Side of the drawn path the wall occupies (default center).
    #[serde(default)]
    pub justification: Option<JustificationArg>,
    /// Material entity assigned to new walls of this type.
    #[serde(default)]
    pub material: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditWallTypeArgs {
    pub expected_revision: u64,
    pub wall_type: String,
    pub thickness: Quantity,
    pub height: Quantity,
    #[serde(default)]
    pub justification: Option<JustificationArg>,
    #[serde(default)]
    pub material: Option<String>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OpeningKindArg {
    Door,
    Window,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HingeArg {
    Start,
    End,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SwingArg {
    Left,
    Right,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpeningTypeArgs {
    pub expected_revision: u64,
    pub name: String,
    pub kind: OpeningKindArg,
    /// Doors: the jamb carrying the hinge, along the wall segment (default start).
    #[serde(default)]
    pub hinge: Option<HingeArg>,
    /// Doors: the side of the wall path the leaf opens to (default left).
    #[serde(default)]
    pub swing: Option<SwingArg>,
    pub width: Quantity,
    pub height: Quantity,
    pub sill: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpeningArg {
    pub opening_type: String,
    /// Zero-based wall segment index.
    pub segment: usize,
    /// Distance along the segment from its start point to the opening's start jamb.
    pub offset: Quantity,
    #[serde(default)]
    pub width: Option<Quantity>,
    #[serde(default)]
    pub height: Option<Quantity>,
    #[serde(default)]
    pub sill: Option<Quantity>,
}
#[derive(Deserialize, JsonSchema)]
pub struct WallArgs {
    pub expected_revision: u64,
    pub wall_type: String,
    /// Path vertices in world X/Y millimeters.
    pub path_mm: Vec<[f64; 2]>,
    /// Close the path into a loop (a room).
    #[serde(default)]
    pub closed: bool,
    /// Elevation of the wall base in millimeters (default 0).
    #[serde(default)]
    pub base_z_mm: f64,
    /// Instance overrides of the type values.
    #[serde(default)]
    pub thickness: Option<Quantity>,
    #[serde(default)]
    pub height: Option<Quantity>,
    #[serde(default)]
    pub justification: Option<JustificationArg>,
    #[serde(default)]
    pub openings: Vec<OpeningArg>,
}
#[derive(Deserialize, JsonSchema)]
pub struct CreateWallArgs {
    pub name: String,
    #[serde(flatten)]
    pub wall: WallArgs,
}
#[derive(Deserialize, JsonSchema)]
pub struct EditWallArgs {
    /// The wall feature ID returned by create_wall.
    pub wall: String,
    #[serde(flatten)]
    pub source: WallArgs,
}

fn quantity(
    q: &Quantity,
    document: &multicad_core::document::Document,
    evaluated: &multicad_core::parameters::EvaluatedParameters,
) -> Result<multicad_core::constraint_sketch::Dimension, ToolError> {
    Ok(q.dimension(document, evaluated)?.0)
}
fn wall_type_value(
    thickness: &Quantity,
    height: &Quantity,
    justification: Option<JustificationArg>,
    material: Option<&str>,
    document: &multicad_core::document::Document,
    evaluated: &multicad_core::parameters::EvaluatedParameters,
) -> Result<WallTypeDefinition, ToolError> {
    Ok(WallTypeDefinition {
        thickness: quantity(thickness, document, evaluated)?,
        height: quantity(height, document, evaluated)?,
        justification: justification.map(Into::into).unwrap_or_default(),
        material: material.map(id).transpose()?,
    })
}
fn wall_value(
    args: &WallArgs,
    body: EntityId,
    document: &multicad_core::document::Document,
    evaluated: &multicad_core::parameters::EvaluatedParameters,
    previous: Option<&WallDefinition>,
) -> Result<WallDefinition, ToolError> {
    let q = |q: &Quantity| quantity(q, document, evaluated);
    let opt = |q: &Option<Quantity>| q.as_ref().map(q_ref(document, evaluated)).transpose();
    let mut openings = vec![];
    for (k, o) in args.openings.iter().enumerate() {
        // An edit keeps the identity of the opening at the same position.
        let id_ = previous
            .and_then(|p| p.openings.get(k))
            .map(|p| p.id)
            .unwrap_or_default();
        openings.push(Opening {
            id: id_,
            opening_type: id(&o.opening_type)?,
            segment: o.segment,
            offset: q(&o.offset)?,
            width: opt(&o.width)?,
            height: opt(&o.height)?,
            sill: opt(&o.sill)?,
        });
    }
    Ok(WallDefinition {
        output: BodyOutput { body },
        wall_type: id(&args.wall_type)?,
        path: args
            .path_mm
            .iter()
            .map(|[x, y]| [length(*x), length(*y)])
            .collect(),
        closed: args.closed,
        base_z: length(args.base_z_mm),
        thickness: opt(&args.thickness)?,
        height: opt(&args.height)?,
        justification: args.justification.map(Into::into),
        openings,
    })
}
fn q_ref<'a>(
    document: &'a multicad_core::document::Document,
    evaluated: &'a multicad_core::parameters::EvaluatedParameters,
) -> impl Fn(&Quantity) -> Result<multicad_core::constraint_sketch::Dimension, ToolError> + 'a {
    move |q| quantity(q, document, evaluated)
}

pub(super) fn create_wall_type(host: &dyn Host, call: &Request, args: WallTypeArgs) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let value = wall_type_value(
            &args.thickness,
            &args.height,
            args.justification,
            args.material.as_deref(),
            session.document(),
            session.parameters(),
        )?;
        let (wall_type, prepared) = model::prepare_wall_type(
            session,
            args.expected_revision,
            call.actor.clone(),
            &args.name,
            &value,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "wall_type": wall_type }))
    })?
}
pub(super) fn edit_wall_type(
    host: &dyn Host,
    call: &Request,
    args: EditWallTypeArgs,
) -> ToolResult {
    let ty = id(&args.wall_type)?;
    let (document, revision, evaluated) = snapshot(host)?;
    revision_matches(revision, args.expected_revision)?;
    let value = wall_type_value(
        &args.thickness,
        &args.height,
        args.justification,
        args.material.as_deref(),
        &document,
        &evaluated,
    )?;
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(model::prepare_wall_type_edit(
            target.session(),
            revision,
            call_owned.actor.clone(),
            ty,
            &value,
        )?)
    })??;
    // Every wall of the type rebuilds natively before the edit may commit.
    compile(prepared.document(), revision, call)?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    Ok(json!({ "revision": committed, "wall_type": ty }))
}
pub(super) fn create_opening_type(
    host: &dyn Host,
    call: &Request,
    args: OpeningTypeArgs,
) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let (doc, ev) = (session.document(), session.parameters());
        let value = OpeningTypeDefinition {
            kind: match args.kind {
                OpeningKindArg::Window => OpeningKind::Window,
                OpeningKindArg::Door => OpeningKind::Door {
                    hinge: match args.hinge {
                        Some(HingeArg::End) => Hinge::End,
                        _ => Hinge::Start,
                    },
                    swing: match args.swing {
                        Some(SwingArg::Right) => Swing::Right,
                        _ => Swing::Left,
                    },
                },
            },
            width: quantity(&args.width, doc, ev)?,
            height: quantity(&args.height, doc, ev)?,
            sill: quantity(&args.sill, doc, ev)?,
        };
        let (opening_type, prepared) = model::prepare_opening_type(
            session,
            args.expected_revision,
            call.actor.clone(),
            &args.name,
            &value,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "opening_type": opening_type }))
    })?
}
fn revision_matches(current: u64, expected: u64) -> Result<(), ToolError> {
    if current == expected {
        Ok(())
    } else {
        Err(ToolError::new(
            "stale_revision",
            format!("Request is based on revision {expected}, but the document is at {current}"),
        )
        .at_revision(current))
    }
}
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: multicad_core::commands::Prepared,
    revision: u64,
    feature: EntityId,
    body: EntityId,
) -> ToolResult {
    let model = compile(prepared.document(), revision, call)?;
    let shape = model
        .snapshot()
        .shapes
        .get(&feature)
        .ok_or_else(|| ToolError::new("model_failed", "The wall produced no solid"))?;
    let (volume, solids) = (shape.volume_mm3, shape.solid_count);
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    Ok(json!({
        "revision": committed,
        "wall": feature,
        "part": body,
        "volume_mm3": volume,
        "solid_count": solids,
    }))
}
pub(super) fn create_wall(host: &dyn Host, call: &Request, args: CreateWallArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    revision_matches(revision, args.wall.expected_revision)?;
    let request = WallRequest::new(
        args.name,
        wall_value(&args.wall, EntityId::new(), &document, &evaluated, None)?,
    );
    let (feature, body) = (request.feature, request.body);
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(
            model::prepare_wall(
                target.session(),
                revision,
                call_owned.actor.clone(),
                &request,
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, feature, body)
}
pub(super) fn edit_wall(host: &dyn Host, call: &Request, args: EditWallArgs) -> ToolResult {
    let feature = id(&args.wall)?;
    let (document, revision, evaluated) = snapshot(host)?;
    revision_matches(revision, args.source.expected_revision)?;
    let current =
        model::read_wall(&document, feature).map_err(|e| ToolError::from(e).involving(feature))?;
    let wall = wall_value(
        &args.source,
        current.output.body,
        &document,
        &evaluated,
        Some(&current),
    )?;
    let body = current.output.body;
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(
            model::prepare_wall_edit(
                target.session(),
                revision,
                call_owned.actor.clone(),
                feature,
                &wall,
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, feature, body)
}
