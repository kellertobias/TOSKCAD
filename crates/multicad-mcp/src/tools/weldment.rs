//! Weldment tools over the shared member API. Each mutation builds every
//! member natively before it commits one transaction; cut records measure
//! the evaluated solids.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    materials::{self, LibraryDefinition, ProfileShape, Stock, StockKind},
    model::{
        self, AxisAnchor, BodyOutput, ConnectionDefinition, ConnectionKind, MemberDefinition,
        MemberPath, MemberRequest, angle,
    },
    parameters::EvaluatedParameters,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileShapeArg {
    Rectangular,
    Round,
    Angle,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileStockArgs {
    pub expected_revision: u64,
    /// Stock name, also used as its designation, e.g. "SHS 40×40×2".
    pub name: String,
    /// Existing Material entity. Omit to create a new Material named `material_name`.
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub material_name: Option<String>,
    /// rectangular (SHS/RHS tube or flat bar), round (CHS tube or round bar) or angle.
    pub shape: ProfileShapeArg,
    /// Section width; for round stock the outside diameter.
    pub width_mm: f64,
    /// Section height; for round stock equal to the width.
    pub height_mm: f64,
    /// Tube wall or angle leg thickness. Omit for a solid bar; required for an angle.
    #[serde(default)]
    pub wall_mm: Option<f64>,
    /// Bar length the stock is supplied in (default 6000).
    #[serde(default)]
    pub bar_length_mm: Option<f64>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnchorArg {
    Min,
    Center,
    Max,
}
impl From<AnchorArg> for AxisAnchor {
    fn from(a: AnchorArg) -> Self {
        match a {
            AnchorArg::Min => Self::Min,
            AnchorArg::Center => Self::Center,
            AnchorArg::Max => Self::Max,
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SegmentArg {
    pub name: String,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MembersArgs {
    pub expected_revision: u64,
    /// Profile stock for every new member.
    pub stock: String,
    /// Sketch whose solid lines become members (one Part per line).
    #[serde(default)]
    pub sketch: Option<String>,
    /// Only these lines of the sketch (default: every solid line).
    #[serde(default)]
    pub lines: Option<Vec<String>>,
    /// Sketch members are named "<name_prefix> N" (default "Member").
    #[serde(default)]
    pub name_prefix: Option<String>,
    /// Explicit point-pair members instead of a sketch.
    #[serde(default)]
    pub segments: Option<Vec<SegmentArg>>,
    /// Which point of the section lies on the path across its width (default center).
    #[serde(default)]
    pub anchor_x: Option<AnchorArg>,
    /// ... and across its height (default center).
    #[serde(default)]
    pub anchor_y: Option<AnchorArg>,
    /// Section rotation about the member axis in degrees (default 0).
    #[serde(default)]
    pub rotation_deg: Option<f64>,
    /// Extra length beyond each path start / end; negative trims.
    #[serde(default)]
    pub extend_start: Option<Quantity>,
    #[serde(default)]
    pub extend_end: Option<Quantity>,
    /// Hold the new members in a model group with this name.
    #[serde(default)]
    pub group: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditMemberArgs {
    pub expected_revision: u64,
    /// Member Part or member feature ID.
    pub member: String,
    #[serde(default)]
    pub stock: Option<String>,
    /// New point-pair path (replaces a sketch-line path too).
    #[serde(default)]
    pub start_mm: Option<[f64; 3]>,
    #[serde(default)]
    pub end_mm: Option<[f64; 3]>,
    #[serde(default)]
    pub anchor_x: Option<AnchorArg>,
    #[serde(default)]
    pub anchor_y: Option<AnchorArg>,
    #[serde(default)]
    pub rotation_deg: Option<f64>,
    #[serde(default)]
    pub extend_start: Option<Quantity>,
    #[serde(default)]
    pub extend_end: Option<Quantity>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKindArg {
    Miter,
    Butt,
    Cope,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectArgs {
    pub expected_revision: u64,
    /// Connection ID to edit; omit to create a new connection.
    #[serde(default)]
    pub connection: Option<String>,
    /// Name of a new connection.
    #[serde(default)]
    pub name: Option<String>,
    /// miter (both members end at the joint, cut on the bisector plane), butt (first ends square against a flat face of the second, which continues) or cope (first is fitted to the actual outside of the second).
    pub kind: ConnectionKindArg,
    /// Member Parts (or member features): miter either order; butt terminating then continuing; cope branch then receiving.
    pub members: [String; 2],
    /// Clearance between the cut end and the face it meets (default 0).
    #[serde(default)]
    pub gap: Option<Quantity>,
}

fn q(q: &Quantity, d: &Document, e: &EvaluatedParameters) -> Result<Dimension, ToolError> {
    Ok(q.dimension(d, e)?.0)
}
fn opt(
    v: &Option<Quantity>,
    d: &Document,
    e: &EvaluatedParameters,
) -> Result<Option<Dimension>, ToolError> {
    v.as_ref().map(|v| q(v, d, e)).transpose()
}
fn stale(expected: u64, revision: u64) -> Result<(), ToolError> {
    if expected != revision {
        return Err(ToolError::new(
            "stale_revision",
            format!("Request is based on revision {expected}, but the document is at {revision}"),
        )
        .at_revision(revision));
    }
    Ok(())
}
/// Build every member natively, then commit.
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: Prepared,
    revision: u64,
    mut out: Value,
) -> ToolResult {
    let model = compile(prepared.document(), revision, call)?;
    let records = model.member_cut_list(prepared.document())?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    out["revision"] = json!(committed);
    out["members"] = json!(records.len());
    Ok(out)
}
/// The member feature for a Part or feature ID.
fn member_id(document: &Document, value: &str) -> Result<EntityId, ToolError> {
    let id = id(value)?;
    if model::members(document).any(|e| e.id == id) {
        return Ok(id);
    }
    model::member_feature(document, id)
        .ok_or_else(|| ToolError::invalid(format!("{value} is not a weldment member")))
}

pub(super) fn create_profile_stock(
    host: &dyn Host,
    call: &Request,
    args: ProfileStockArgs,
) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let mut entities = vec![];
        let material = match (&args.material, &args.material_name) {
            (Some(m), None) => id(m)?,
            (None, Some(name)) => {
                let m = Entity::new(
                    name.clone(),
                    EntityData::Material {
                        density_kg_m3: None,
                        color_rgba: [120, 124, 130, 255],
                        section_hatch: None,
                    },
                );
                let id = m.id;
                entities.push(m);
                id
            }
            _ => {
                return Err(ToolError::invalid(
                    "Give exactly one of material or material_name",
                ));
            }
        };
        let stock = LibraryDefinition::Stock(Stock {
            material,
            specification: StockKind::Profile {
                designation: args.name.clone(),
                length_mm: args.bar_length_mm.unwrap_or(6000.),
                width_mm: args.width_mm,
                height_mm: args.height_mm,
                wall_mm: args.wall_mm,
                shape: match args.shape {
                    ProfileShapeArg::Rectangular => ProfileShape::Rectangular,
                    ProfileShapeArg::Round => ProfileShape::Round,
                    ProfileShapeArg::Angle => ProfileShape::Angle,
                },
            },
        })
        .entity(EntityId::new(), args.name.clone())
        .map_err(|_| {
            ToolError::invalid(
                "Invalid profile stock: sizes must be positive, round stock needs width = height, an angle needs wall_mm, and twice the wall must fit the section",
            )
        })?;
        let stock_id = stock.id;
        entities.push(stock);
        let prepared = materials::prepare_library(
            session,
            args.expected_revision,
            call.actor.clone(),
            &entities,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "stock": stock_id, "material": material }))
    })?
}

pub(super) fn create_members(host: &dyn Host, call: &Request, args: MembersArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    stale(args.expected_revision, revision)?;
    let (d, e) = (&document, &evaluated);
    let template = MemberDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock: id(&args.stock)?,
        path: MemberPath::Points {
            start: [0.; 3].map(model::length),
            end: [1., 0., 0.].map(model::length),
        },
        anchor: [
            args.anchor_x.map_or(AxisAnchor::Center, Into::into),
            args.anchor_y.map_or(AxisAnchor::Center, Into::into),
        ],
        rotation: args.rotation_deg.map(angle),
        extend_start: opt(&args.extend_start, d, e)?,
        extend_end: opt(&args.extend_end, d, e)?,
    };
    let requests = match (&args.sketch, &args.segments) {
        (Some(sketch), None) => {
            let sketch = id(sketch)?;
            let prefix = args.name_prefix.as_deref().unwrap_or("Member");
            let mut requests = model::frame_member_requests(d, sketch, prefix, &template)?;
            if let Some(lines) = &args.lines {
                let wanted = lines.iter().map(|l| id(l)).collect::<Result<Vec<_>, _>>()?;
                for line in &wanted {
                    model::check_sketch_line(d, sketch, *line)?;
                }
                requests.retain(|r| {
                    matches!(r.member.path, MemberPath::SketchLine { line, .. } if wanted.contains(&line))
                });
            }
            requests
        }
        (None, Some(segments)) => segments
            .iter()
            .map(|s| {
                MemberRequest::new(
                    s.name.clone(),
                    MemberDefinition {
                        path: MemberPath::Points {
                            start: s.start_mm.map(model::length),
                            end: s.end_mm.map(model::length),
                        },
                        ..template.clone()
                    },
                )
            })
            .collect(),
        _ => {
            return Err(ToolError::invalid("Give exactly one of sketch or segments"));
        }
    };
    let out = json!({
        "parts": requests.iter().map(|r| json!({
            "name": r.name, "part": r.body, "feature": r.feature,
            "line": match r.member.path { MemberPath::SketchLine { line, .. } => Some(line), _ => None },
        })).collect::<Vec<_>>(),
    });
    let group = args.group.clone();
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(
            model::prepare_members(
                target.session(),
                revision,
                call_owned.actor.clone(),
                &requests,
                group.as_deref(),
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, out)
}

pub(super) fn edit_member(host: &dyn Host, call: &Request, args: EditMemberArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    stale(args.expected_revision, revision)?;
    let (d, e) = (&document, &evaluated);
    let feature = member_id(d, &args.member)?;
    let mut member = model::read_member(d, feature).map_err(ToolError::from)?;
    if let Some(stock) = &args.stock {
        member.stock = id(stock)?;
    }
    match (args.start_mm, args.end_mm) {
        (Some(start), Some(end)) => {
            member.path = MemberPath::Points {
                start: start.map(model::length),
                end: end.map(model::length),
            }
        }
        (None, None) => {}
        _ => return Err(ToolError::invalid("Give both start_mm and end_mm")),
    }
    if let Some(a) = args.anchor_x {
        member.anchor[0] = a.into();
    }
    if let Some(a) = args.anchor_y {
        member.anchor[1] = a.into();
    }
    if let Some(r) = args.rotation_deg {
        member.rotation = Some(angle(r));
    }
    if args.extend_start.is_some() {
        member.extend_start = opt(&args.extend_start, d, e)?;
    }
    if args.extend_end.is_some() {
        member.extend_end = opt(&args.extend_end, d, e)?;
    }
    let out = json!({ "part": member.output.body, "feature": feature });
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(
            model::prepare_member_edit(
                target.session(),
                revision,
                call_owned.actor.clone(),
                feature,
                &member,
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, out)
}

pub(super) fn connect_members(host: &dyn Host, call: &Request, args: ConnectArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    stale(args.expected_revision, revision)?;
    let (d, e) = (&document, &evaluated);
    let part = |value: &str| -> Result<EntityId, ToolError> {
        let feature = member_id(d, value)?;
        Ok(model::read_member(d, feature)
            .map_err(ToolError::from)?
            .output
            .body)
    };
    let connection = ConnectionDefinition {
        kind: match args.kind {
            ConnectionKindArg::Miter => ConnectionKind::Miter,
            ConnectionKindArg::Butt => ConnectionKind::Butt,
            ConnectionKindArg::Cope => ConnectionKind::Cope,
        },
        members: [part(&args.members[0])?, part(&args.members[1])?],
        gap: opt(&args.gap, d, e)?,
    };
    let edit = args.connection.as_deref().map(id).transpose()?;
    let name = args.name.clone();
    let call_owned = call.clone();
    let (connection_id, prepared) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match edit {
            Some(id) => (
                id,
                model::prepare_connection_edit(session, revision, actor, id, &connection)?,
            ),
            None => {
                let name =
                    name.ok_or_else(|| ToolError::invalid("A new connection needs a name"))?;
                model::prepare_connection(session, revision, actor, &name, &connection)?
            }
        })
    })??;
    finish(
        host,
        call,
        prepared,
        revision,
        json!({ "connection": connection_id }),
    )
}

fn end_json(e: &multicad_model::EndRecord) -> Value {
    json!({
        "treatment": e.kind.map_or("square", |k| k.label()),
        "connection": e.connection, "partner": e.partner,
        "angle_deg": e.angle_deg, "terminates": e.terminates,
    })
}

pub(super) fn weldment_cut_list(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let records = model.member_cut_list(&document)?;
    Ok(json!({
        "revision": revision,
        "scope": "Lengths measured from the evaluated member solids along each member axis (overall length, farthest point to farthest point; span is the measured axial range from the path start). End angles: plane cuts from square (0 = square, 45 = right-angle miter); copes give the branch angle to the receiving axis. Sections are modeled with sharp corners. A geometric record, not a nesting plan, certified procedure or structural check.",
        "members": records.iter().map(|r| json!({
            "part": r.part, "feature": r.feature, "name": r.name,
            "material": r.material, "stock": r.stock, "stock_name": r.stock_name,
            "designation": r.designation, "shape": format!("{:?}", r.shape).to_lowercase(),
            "section_mm": r.section_mm, "wall_mm": r.wall_mm,
            "path": match &r.path {
                MemberPath::SketchLine { sketch, line } => json!({ "sketch": sketch, "line": line }),
                MemberPath::Points { .. } => json!({ "points": true }),
            },
            "length_mm": r.length_mm, "nominal_length_mm": r.nominal_length_mm,
            "rotation_deg": r.rotation_deg,
            "anchor": r.anchor.map(|a| format!("{a:?}").to_lowercase()),
            "axis": r.frame.axis, "width_direction": r.frame.x, "height_direction": r.frame.y,
            "span_mm": r.span_mm,
            "start": end_json(&r.ends[0]), "end": end_json(&r.ends[1]),
            "volume_mm3": r.volume_mm3, "fits_stock": r.fits_stock, "issues": r.issues,
        })).collect::<Vec<_>>(),
    }))
}
