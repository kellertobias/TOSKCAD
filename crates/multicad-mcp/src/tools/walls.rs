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
        self, BodyOutput, CutoutPlacement, Hinge, Justification, NewType, NewTypeKind, Opening,
        OpeningKind, OpeningTypeDefinition, PlanRectangle, Swing, VerticalCutout, WallDefinition,
        WallFace, WallOpeningChange, WallRequest, WallTypeDefinition, length,
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
    /// Hosting level; the base offset is then measured from its elevation.
    #[serde(default)]
    pub level: Option<String>,
    /// Elevation of the wall base in millimeters above its level or Z = 0 (default 0).
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
        layer_placement: None,
        output: BodyOutput { body },
        wall_type: id(&args.wall_type)?,
        path: args
            .path_mm
            .iter()
            .map(|[x, y]| [length(*x), length(*y)])
            .collect(),
        closed: args.closed,
        level: args.level.as_deref().map(id).transpose()?,
        base_z: length(args.base_z_mm),
        thickness: opt(&args.thickness)?,
        height: opt(&args.height)?,
        justification: args.justification.map(Into::into),
        openings,
        // This tool does not author cutouts; an edit keeps the existing ones.
        cutouts: previous.map(|p| p.cutouts.clone()).unwrap_or_default(),
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

// ---- Single openings: the shared Room-tool operation ---------------------

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FaceArg {
    Left,
    Right,
}
/// A door or window type created in the same transaction as the placement.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewOpeningTypeArg {
    pub name: String,
    pub kind: OpeningKindArg,
    #[serde(default)]
    pub hinge: Option<HingeArg>,
    #[serde(default)]
    pub swing: Option<SwingArg>,
    pub width: Quantity,
    pub height: Quantity,
    pub sill: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WallOpeningArgs {
    pub expected_revision: u64,
    /// The wall feature ID.
    pub wall: String,
    /// An existing door/window ID of this wall to edit; omit to add one.
    #[serde(default)]
    pub opening: Option<String>,
    /// Existing door or window type. Required when adding unless `new_type` is given.
    #[serde(default)]
    pub opening_type: Option<String>,
    /// A new reusable type committed together with the placement.
    #[serde(default)]
    pub new_type: Option<NewOpeningTypeArg>,
    /// Zero-based wall segment index (see inspect_wall).
    pub segment: usize,
    /// Distance along the segment from its start point to the start jamb.
    pub offset: Quantity,
    #[serde(default)]
    pub width: Option<Quantity>,
    #[serde(default)]
    pub height: Option<Quantity>,
    #[serde(default)]
    pub sill: Option<Quantity>,
}
/// A plan rectangle cut from slabs: center in world X/Y, width along the
/// direction `rotation_deg` from +X, depth across it.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanRectangleArg {
    pub center: [Quantity; 2],
    pub width: Quantity,
    pub depth: Quantity,
    #[serde(default)]
    pub rotation_deg: f64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerticalCutoutArgs {
    pub expected_revision: u64,
    /// The wall feature ID to cut. Omit for a slab-only cutout (shaft or floor
    /// penetration); when editing, omitting it removes the cutout from its wall.
    #[serde(default)]
    pub wall: Option<String>,
    /// An existing cutout ID (in a wall or slabs) to edit; omit to add one.
    #[serde(default)]
    pub cutout: Option<String>,
    /// Zero-based wall segment index (see inspect_wall). Required with `wall`.
    #[serde(default)]
    pub segment: Option<usize>,
    /// Distance along the segment from its start point to the cutout's start
    /// side. Required with `wall`.
    #[serde(default)]
    pub offset: Option<Quantity>,
    /// Cutout width along the wall. Required with `wall`.
    #[serde(default)]
    pub width: Option<Quantity>,
    /// Chase depth into the wall from `face`; omit to cut through the full thickness.
    #[serde(default)]
    pub depth: Option<Quantity>,
    /// Face a chase is cut from, looking along the segment (default left).
    #[serde(default)]
    pub face: Option<FaceArg>,
    /// Slab or pedestal feature IDs to cut. An empty list cuts no slab.
    /// Give either `slabs` or `from_level`.
    #[serde(default)]
    pub slabs: Option<Vec<String>>,
    /// Cut every floor slab under the cutout on levels from this level ...
    #[serde(default)]
    pub from_level: Option<String>,
    /// ... up to this level (inclusive; default: `from_level` only).
    #[serde(default)]
    pub to_level: Option<String>,
    /// The rectangle cut from slabs. Required for a slab-only cutout; with a
    /// wall it defaults to the wall cutout's own plan footprint.
    #[serde(default)]
    pub plan: Option<PlanRectangleArg>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveVerticalCutoutArgs {
    pub expected_revision: u64,
    /// The vertical cutout ID; it is removed from its wall and every slab.
    pub cutout: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectVerticalCutoutArgs {
    /// A cutout ID; omit to list every cutout that cuts a slab.
    #[serde(default)]
    pub cutout: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveWallOpeningArgs {
    pub expected_revision: u64,
    pub wall: String,
    /// The door, window or vertical cutout ID to remove.
    pub opening: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectWallArgs {
    pub wall: String,
}

fn opening_kind(
    kind: OpeningKindArg,
    hinge: Option<HingeArg>,
    swing: Option<SwingArg>,
) -> OpeningKind {
    match kind {
        OpeningKindArg::Window => OpeningKind::Window,
        OpeningKindArg::Door => OpeningKind::Door {
            hinge: match hinge {
                Some(HingeArg::End) => Hinge::End,
                _ => Hinge::Start,
            },
            swing: match swing {
                Some(SwingArg::Right) => Swing::Right,
                _ => Swing::Left,
            },
        },
    }
}
fn read_current_wall(
    document: &multicad_core::document::Document,
    feature: EntityId,
) -> Result<WallDefinition, ToolError> {
    model::read_wall(document, feature).map_err(|e| ToolError::from(e).involving(feature))
}
/// Stage one opening change, evaluate the wall natively, then commit.
fn commit_opening_change(
    host: &dyn Host,
    call: &Request,
    revision: u64,
    (feature, body): (EntityId, EntityId),
    new_types: Vec<NewType>,
    change: WallOpeningChange,
) -> ToolResult {
    let target = change.target();
    let call_owned = call.clone();
    let prepared = with(host, move |t| {
        revision_guard(t, revision)?;
        Ok::<_, ToolError>(
            model::prepare_wall_opening(
                t.session(),
                revision,
                call_owned.actor.clone(),
                feature,
                &new_types,
                &change,
            )?
            .prepared,
        )
    })??;
    let mut result = finish(host, call, prepared, revision, feature, body)?;
    result["opening"] = json!(target);
    Ok(result)
}
pub(super) fn set_wall_opening(
    host: &dyn Host,
    call: &Request,
    args: WallOpeningArgs,
) -> ToolResult {
    let feature = id(&args.wall)?;
    let (document, revision, evaluated) = snapshot(host)?;
    revision_matches(revision, args.expected_revision)?;
    let current = read_current_wall(&document, feature)?;
    let q = q_ref(&document, &evaluated);
    let opt = |v: &Option<Quantity>| v.as_ref().map(&q).transpose();
    let previous = match &args.opening {
        Some(o) => {
            let oid = id(o)?;
            Some(
                current
                    .openings
                    .iter()
                    .find(|x| x.id == oid)
                    .cloned()
                    .ok_or_else(|| {
                        ToolError::new(
                            "invalid_request",
                            "This wall has no door or window with that ID",
                        )
                    })?,
            )
        }
        None => None,
    };
    let mut new_types = vec![];
    let opening_type = match (&args.opening_type, &args.new_type) {
        (Some(_), Some(_)) => {
            return Err(ToolError::new(
                "invalid_request",
                "Give either opening_type or new_type, not both",
            ));
        }
        (Some(t), None) => id(t)?,
        (None, Some(t)) => {
            let ty = NewType::new(
                t.name.clone(),
                NewTypeKind::Opening(OpeningTypeDefinition {
                    kind: opening_kind(t.kind, t.hinge, t.swing),
                    width: q(&t.width)?,
                    height: q(&t.height)?,
                    sill: q(&t.sill)?,
                }),
            );
            let tid = ty.id;
            new_types.push(ty);
            tid
        }
        (None, None) => previous.as_ref().map(|p| p.opening_type).ok_or_else(|| {
            ToolError::new(
                "invalid_request",
                "Choose an opening_type or define a new_type for the new opening",
            )
        })?,
    };
    let opening = Opening {
        id: previous.as_ref().map(|p| p.id).unwrap_or_default(),
        opening_type,
        segment: args.segment,
        offset: q(&args.offset)?,
        width: opt(&args.width)?,
        height: opt(&args.height)?,
        sill: opt(&args.sill)?,
    };
    let change = if previous.is_some() {
        WallOpeningChange::ReplaceOpening(opening)
    } else {
        WallOpeningChange::AddOpening(opening)
    };
    commit_opening_change(
        host,
        call,
        revision,
        (feature, current.output.body),
        new_types,
        change,
    )
}
fn wall_cutout(
    args: &VerticalCutoutArgs,
    id: EntityId,
    q: &dyn Fn(&Quantity) -> Result<multicad_core::constraint_sketch::Dimension, ToolError>,
) -> Result<VerticalCutout, ToolError> {
    let need = |what: &str| ToolError::invalid(format!("Give the {what} of the wall cutout"));
    Ok(VerticalCutout {
        id,
        segment: args.segment.ok_or_else(|| need("segment"))?,
        offset: q(args.offset.as_ref().ok_or_else(|| need("offset"))?)?,
        width: q(args.width.as_ref().ok_or_else(|| need("width"))?)?,
        depth: args.depth.as_ref().map(q).transpose()?,
        face: match args.face {
            Some(FaceArg::Right) => WallFace::Right,
            _ => WallFace::Left,
        },
    })
}
pub(super) fn set_vertical_cutout(
    host: &dyn Host,
    call: &Request,
    args: VerticalCutoutArgs,
) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    revision_matches(revision, args.expected_revision)?;
    let q = q_ref(&document, &evaluated);
    let existing = args.cutout.as_deref().map(id).transpose()?;
    let members = existing.map(|c| model::cutout_members(&document, c));
    let slab_mode = args.slabs.is_some()
        || args.from_level.is_some()
        || args.plan.is_some()
        || members.as_ref().is_some_and(|m| !m.slabs.is_empty());
    if !slab_mode {
        // A wall-only cutout: the unchanged CAD-147 wall operation.
        let feature = id(args
            .wall
            .as_deref()
            .ok_or_else(|| ToolError::invalid("Choose a wall, slabs or a level range to cut"))?)?;
        let current = read_current_wall(&document, feature)?;
        if let Some(cid) = existing
            && !current.cutouts.iter().any(|c| c.id == cid)
        {
            return Err(ToolError::new(
                "invalid_request",
                "This wall has no vertical cutout with that ID",
            ));
        }
        let cutout = wall_cutout(&args, existing.unwrap_or_default(), &q)?;
        let change = if existing.is_some() {
            WallOpeningChange::ReplaceCutout(cutout)
        } else {
            WallOpeningChange::AddCutout(cutout)
        };
        return commit_opening_change(
            host,
            call,
            revision,
            (feature, current.output.body),
            vec![],
            change,
        );
    }
    if let (Some(c), Some(m)) = (existing, &members)
        && m.is_empty()
    {
        return Err(ToolError::invalid(format!(
            "No wall or slab has a vertical cutout {c}"
        )));
    }
    let cutout_id = existing.unwrap_or_default();
    let wall = match &args.wall {
        Some(w) => {
            let feature = id(w)?;
            Some((feature, wall_cutout(&args, cutout_id, &q)?))
        }
        None => None,
    };
    let plan = match &args.plan {
        Some(p) => Some(PlanRectangle {
            center: [q(&p.center[0])?, q(&p.center[1])?],
            width: q(&p.width)?,
            depth: q(&p.depth)?,
            rotation_deg: p.rotation_deg,
        }),
        None => None,
    };
    let mut placement = CutoutPlacement {
        id: cutout_id,
        wall,
        plan,
        slabs: vec![],
    };
    placement.slabs = match (&args.slabs, &args.from_level) {
        (Some(_), Some(_)) => {
            return Err(ToolError::invalid(
                "Give either slabs or a level range, not both",
            ));
        }
        (Some(list), None) => list.iter().map(|s| id(s)).collect::<Result<_, _>>()?,
        (None, Some(from)) => {
            let footprint = model::cutout_footprint(&document, &evaluated, &placement)?;
            let found = model::slabs_in_levels(
                &document,
                &evaluated,
                id(from)?,
                args.to_level.as_deref().map(id).transpose()?,
                &footprint.corners(),
            )?;
            if found.is_empty() {
                return Err(ToolError::invalid(
                    "No floor slab lies under the cutout on the chosen levels",
                ));
            }
            found
        }
        (None, None) => members
            .as_ref()
            .map(|m| m.slabs.iter().map(|(s, _)| *s).collect())
            .unwrap_or_default(),
    };
    commit_cutout_change(host, call, revision, model::CutoutChange::place(placement))
}
/// Stage one cutout change, evaluate every touched feature natively, commit.
fn commit_cutout_change(
    host: &dyn Host,
    call: &Request,
    revision: u64,
    change: model::CutoutChange,
) -> ToolResult {
    let target = change.target();
    let call_owned = call.clone();
    let prepared = with(host, move |t| {
        revision_guard(t, revision)?;
        Ok::<_, ToolError>(model::prepare_vertical_cutout(
            t.session(),
            revision,
            call_owned.actor.clone(),
            &change,
        )?)
    })??;
    let model = compile(prepared.prepared.document(), revision, call)?;
    let mut features = vec![];
    for output in &prepared.outputs {
        let shape = model
            .snapshot()
            .shapes
            .get(&output.feature)
            .ok_or_else(|| {
                ToolError::new("model_failed", "A cut wall or slab produced no solid")
                    .involving(output.feature)
            })?;
        features.push(json!({
            "feature": output.feature, "part": output.body,
            "volume_mm3": shape.volume_mm3, "solid_count": shape.solid_count,
        }));
    }
    let members = model::cutout_members(prepared.prepared.document(), target);
    let call = call.clone();
    let committed = with(host, move |t| commit(t, &call, prepared.prepared))??;
    Ok(json!({
        "revision": committed,
        "cutout": target,
        "wall": members.wall.map(|(w, _)| w),
        "slabs": members.slabs.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        "features": features,
    }))
}
pub(super) fn remove_vertical_cutout(
    host: &dyn Host,
    call: &Request,
    args: RemoveVerticalCutoutArgs,
) -> ToolResult {
    let (_, revision, _) = snapshot(host)?;
    revision_matches(revision, args.expected_revision)?;
    commit_cutout_change(
        host,
        call,
        revision,
        model::CutoutChange::Remove(id(&args.cutout)?),
    )
}
pub(super) fn inspect_vertical_cutout(
    host: &dyn Host,
    _: &Request,
    args: InspectVerticalCutoutArgs,
) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let ids = match &args.cutout {
        Some(c) => vec![id(c)?],
        None => model::slab_cutouts(&document),
    };
    let mut cutouts = vec![];
    for cutout in ids {
        let members = model::cutout_members(&document, cutout);
        if members.is_empty() {
            return Err(ToolError::invalid(format!(
                "No wall or slab has a vertical cutout {cutout}"
            )));
        }
        let mut slabs = vec![];
        for (slab, _) in &members.slabs {
            let e = |e: multicad_core::document::DocumentError| ToolError::from(e).involving(*slab);
            let source = model::read_slab(&document, *slab).map_err(e)?;
            let resolved = model::resolve_slab(&document, &evaluated, &source).map_err(e)?;
            let hole = resolved.holes.iter().find(|h| h.id == cutout);
            slabs.push(json!({
                "slab": slab, "level": source.level, "part": source.output.body,
                "bottom_mm": resolved.bottom_mm, "top_mm": resolved.top_mm,
                "corners_mm": hole.map(|h| h.corners),
            }));
        }
        // Bottom level first.
        let bottom = |v: &serde_json::Value| v["bottom_mm"].as_f64().unwrap_or(0.);
        slabs.sort_by(|a, b| bottom(a).total_cmp(&bottom(b)));
        cutouts.push(json!({
            "cutout": cutout,
            "wall": members.wall.as_ref().map(|(w, _)| *w),
            "slabs": slabs,
        }));
    }
    Ok(json!({ "revision": revision, "cutouts": cutouts }))
}
pub(super) fn remove_wall_opening(
    host: &dyn Host,
    call: &Request,
    args: RemoveWallOpeningArgs,
) -> ToolResult {
    let feature = id(&args.wall)?;
    let (document, revision, _) = snapshot(host)?;
    revision_matches(revision, args.expected_revision)?;
    let current = read_current_wall(&document, feature)?;
    let target = id(&args.opening)?;
    if current.cutouts.iter().any(|c| c.id == target)
        && !model::cutout_members(&document, target).slabs.is_empty()
    {
        // The cutout also cuts slabs: remove it everywhere in one transaction.
        return commit_cutout_change(host, call, revision, model::CutoutChange::Remove(target));
    }
    let change = WallOpeningChange::Remove(target);
    commit_opening_change(
        host,
        call,
        revision,
        (feature, current.output.body),
        vec![],
        change,
    )
}
pub(super) fn inspect_wall(host: &dyn Host, _: &Request, args: InspectWallArgs) -> ToolResult {
    let feature = id(&args.wall)?;
    let (document, revision, evaluated) = snapshot(host)?;
    let source = read_current_wall(&document, feature)?;
    let e = |e: multicad_core::document::DocumentError| ToolError::from(e).involving(feature);
    let wall = model::resolve_wall(&document, &evaluated, &source).map_err(e)?;
    let plan = wall.plan().map_err(e)?;
    let diagnostic = wall.check_openings(&plan).err().map(|e| e.to_string());
    let segments: Vec<_> = plan
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "segment": i, "start_mm": s.start, "direction": s.direction,
                "length_mm": s.length_mm,
            })
        })
        .collect();
    let openings: Vec<_> = wall
        .openings
        .iter()
        .zip(&source.openings)
        .map(|(o, s)| {
            json!({
                "opening": o.id, "opening_type": s.opening_type,
                "kind": match o.kind { OpeningKind::Door { .. } => "door", OpeningKind::Window => "window" },
                "segment": o.segment, "offset_mm": o.offset_mm, "width_mm": o.width_mm,
                "height_mm": o.height_mm, "sill_mm": o.sill_mm,
            })
        })
        .collect();
    let cutouts: Vec<_> = wall
        .cutouts
        .iter()
        .map(|c| {
            let slabs: Vec<_> = model::cutout_members(&document, c.id)
                .slabs
                .into_iter()
                .map(|(s, _)| s)
                .collect();
            json!({
                "cutout": c.id, "slabs": slabs, "segment": c.segment, "offset_mm": c.offset_mm,
                "width_mm": c.width_mm, "depth_mm": c.depth_mm, "through": c.through,
                "face": match c.face { WallFace::Left => "left", WallFace::Right => "right" },
            })
        })
        .collect();
    Ok(json!({
        "revision": revision,
        "wall": feature,
        "part": source.output.body,
        "closed": wall.closed,
        "thickness_mm": wall.thickness_mm,
        "height_mm": wall.height_mm,
        "base_z_mm": wall.base_z_mm,
        "segments": segments,
        "openings": openings,
        "cutouts": cutouts,
        "diagnostic": diagnostic,
    }))
}
