//! Cabinetry tools over the shared panel, carcass and stack API. Each
//! mutation builds every affected panel natively before it commits one
//! transaction; the cut list measures the evaluated solids.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    materials::{self, Grain, LibraryDefinition, Stock, StockKind},
    model::{
        self, Back, Band, BodyOutput, CarcassDefinition, CarcassRequest, Construction, GrainAxis,
        HardwareHole, HardwarePatternDefinition, MachiningDefinition, MachiningFace, MachiningKind,
        MaterialAllowance, PanelAxis, PanelDefinition, PanelEdgeName, PanelEdges,
        PanelJointDefinition, PanelJointKind, PanelJointPair, PanelPlane, PanelRequest,
        PanelSource, ShelfLayout, Shelves, angle, length,
    },
    parameters::EvaluatedParameters,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrainArg {
    None,
    AlongLength,
    AcrossLength,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelStockArgs {
    pub expected_revision: u64,
    pub name: String,
    /// Existing Material entity. Omit to create a new Material named `material_name`.
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub material_name: Option<String>,
    pub thickness_mm: f64,
    /// Stock sheet size.
    pub sheet_length_mm: f64,
    pub sheet_width_mm: f64,
    /// Grain of the sheet relative to its length (default none).
    #[serde(default)]
    pub grain: Option<GrainArg>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BandArg {
    pub thickness: Quantity,
    /// Extra core per banded edge, trimmed after banding (default 0).
    #[serde(default)]
    pub allowance: Option<Quantity>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackKindArg {
    None,
    Inset,
    Overlay,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstructionArg {
    SidesFullHeight,
    TopBottomFullWidth,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CarcassArgs {
    pub expected_revision: u64,
    /// Carcass ID to edit; omit to create a new carcass.
    #[serde(default)]
    pub carcass: Option<String>,
    /// Name of a new carcass; its panels are named after it.
    #[serde(default)]
    pub name: Option<String>,
    /// World position of the front-left-bottom corner in millimeters (default 0,0,0).
    #[serde(default)]
    pub origin_mm: Option<[f64; 3]>,
    /// Rotation about +Z in degrees (default 0). Unrotated, the front faces −Y.
    #[serde(default)]
    pub rotation_deg: f64,
    pub width: Quantity,
    pub height: Quantity,
    pub depth: Quantity,
    /// Panel stock for the sides, top, bottom and shelves.
    pub stock: String,
    #[serde(default)]
    pub construction: Option<ConstructionArg>,
    pub back: BackKindArg,
    /// Back panel stock, required for an inset or overlay back.
    #[serde(default)]
    pub back_stock: Option<String>,
    /// Inset backs: distance in from the rear (default 0).
    #[serde(default)]
    pub back_recess: Option<Quantity>,
    /// Evenly spaced shelves; ignored when shelf_offsets is given.
    #[serde(default)]
    pub shelf_count: u32,
    /// Distances from the interior bottom to each shelf's underside, bottom to top.
    #[serde(default)]
    pub shelf_offsets: Option<Vec<Quantity>>,
    /// Shelf front setback from the cabinet front (default 0).
    #[serde(default)]
    pub shelf_setback: Option<Quantity>,
    /// Gap at each shelf end (default 0).
    #[serde(default)]
    pub shelf_clearance: Option<Quantity>,
    /// Banding on the front edges of the sides, top, bottom and shelves.
    #[serde(default)]
    pub front_band: Option<BandArg>,
    /// New carcasses: hold the panels in a model group (default true).
    #[serde(default)]
    pub group: Option<bool>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlaneArg {
    Xy,
    Xz,
    Yz,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrainAxisArg {
    U,
    V,
    Free,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelArgs {
    pub expected_revision: u64,
    pub name: String,
    pub stock: String,
    /// Principal plane: u, v = X, Y (xy), X, Z (xz) or Y, Z (yz). Thickness runs along u × v.
    pub plane: PlaneArg,
    /// World position of the minimum corner in millimeters.
    pub origin_mm: [f64; 3],
    /// Size along u.
    pub length: Quantity,
    /// Size along v.
    pub width: Quantity,
    /// The panel axis the grain runs along (default u).
    #[serde(default)]
    pub grain: Option<GrainAxisArg>,
    #[serde(default)]
    pub band_u_start: Option<BandArg>,
    #[serde(default)]
    pub band_u_end: Option<BandArg>,
    #[serde(default)]
    pub band_v_start: Option<BandArg>,
    #[serde(default)]
    pub band_v_end: Option<BandArg>,
    /// Extra core added to both cut dimensions.
    #[serde(default)]
    pub oversize: Option<Quantity>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StackArgs {
    pub expected_revision: u64,
    pub name: String,
    /// Panel Parts in stacking order.
    pub layers: Vec<String>,
    #[serde(default)]
    pub adhesive: Option<String>,
    /// Move the layers into a model group named after the stack (default false).
    #[serde(default)]
    pub group: bool,
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
fn band(
    b: &Option<BandArg>,
    d: &Document,
    e: &EvaluatedParameters,
) -> Result<Option<Band>, ToolError> {
    b.as_ref()
        .map(|b| {
            Ok(Band {
                thickness: q(&b.thickness, d, e)?,
                allowance: opt(&b.allowance, d, e)?,
            })
        })
        .transpose()
}
/// Build every affected panel natively, then commit.
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: Prepared,
    revision: u64,
    mut out: Value,
) -> ToolResult {
    let model = compile(prepared.document(), revision, call)?;
    let records = model.cut_list(prepared.document())?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    out["revision"] = json!(committed);
    out["panels"] = json!(records.len());
    Ok(out)
}

pub(super) fn create_panel_stock(
    host: &dyn Host,
    call: &Request,
    args: PanelStockArgs,
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
                        color_rgba: [214, 186, 140, 255],
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
            specification: StockKind::Panel {
                thickness_mm: args.thickness_mm,
                length_mm: args.sheet_length_mm,
                width_mm: args.sheet_width_mm,
                grain: match args.grain {
                    Some(GrainArg::AlongLength) => Grain::AlongLength,
                    Some(GrainArg::AcrossLength) => Grain::AcrossLength,
                    _ => Grain::None,
                },
            },
        })
        .entity(EntityId::new(), args.name.clone())
        .map_err(ToolError::from)?;
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

pub(super) fn set_carcass(host: &dyn Host, call: &Request, args: CarcassArgs) -> ToolResult {
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
    let (d, e) = (&document, &evaluated);
    let back_stock = || {
        args.back_stock
            .as_deref()
            .map(id)
            .transpose()?
            .ok_or_else(|| ToolError::invalid("This back needs back_stock"))
    };
    let origin = args.origin_mm.unwrap_or([0.; 3]);
    let definition = CarcassDefinition {
        cabinet: None,
        origin: origin.map(length),
        rotation: angle(args.rotation_deg),
        width: q(&args.width, d, e)?,
        height: q(&args.height, d, e)?,
        depth: q(&args.depth, d, e)?,
        stock: id(&args.stock)?,
        construction: match args.construction {
            Some(ConstructionArg::TopBottomFullWidth) => Construction::TopBottomFullWidth,
            _ => Construction::SidesFullHeight,
        },
        back: match args.back {
            BackKindArg::None => Back::None,
            BackKindArg::Inset => Back::Inset {
                stock: back_stock()?,
                recess: opt(&args.back_recess, d, e)?.unwrap_or_else(|| length(0.)),
            },
            BackKindArg::Overlay => Back::Overlay {
                stock: back_stock()?,
            },
        },
        shelves: Shelves {
            layout: match &args.shelf_offsets {
                Some(offsets) => ShelfLayout::Offsets {
                    offsets: offsets
                        .iter()
                        .map(|o| q(o, d, e))
                        .collect::<Result<_, _>>()?,
                },
                None => ShelfLayout::Even {
                    count: args.shelf_count,
                },
            },
            setback: opt(&args.shelf_setback, d, e)?.unwrap_or_else(|| length(0.)),
            clearance: opt(&args.shelf_clearance, d, e)?.unwrap_or_else(|| length(0.)),
        },
        front_band: band(&args.front_band, d, e)?,
        group: None,
    };
    let edit = args.carcass.as_deref().map(id).transpose()?;
    let name = args.name.clone();
    let group = args.group.unwrap_or(true);
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match edit {
            Some(carcass) => {
                model::prepare_carcass_edit(session, revision, actor, carcass, &definition)?
            }
            None => {
                let name = name.ok_or_else(|| ToolError::invalid("A new carcass needs a name"))?;
                model::prepare_carcass(
                    session,
                    revision,
                    actor,
                    &CarcassRequest {
                        group,
                        ..CarcassRequest::new(name, definition)
                    },
                )?
            }
        })
    })??;
    let out = json!({
        "carcass": prepared.carcass,
        "parts": prepared.panels.iter().map(|(role, o)| json!({
            "role": role.label(), "part": o.body, "feature": o.feature,
        })).collect::<Vec<_>>(),
    });
    finish(host, call, prepared.model.prepared, revision, out)
}

pub(super) fn create_panel(host: &dyn Host, call: &Request, args: PanelArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let (d, e) = (&document, &evaluated);
    let panel = PanelDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        source: PanelSource::Rectangle {
            stock: id(&args.stock)?,
            plane: match args.plane {
                PlaneArg::Xy => PanelPlane::Xy,
                PlaneArg::Xz => PanelPlane::Xz,
                PlaneArg::Yz => PanelPlane::Yz,
            },
            origin: args.origin_mm.map(length),
            length: q(&args.length, d, e)?,
            width: q(&args.width, d, e)?,
        },
        grain: match args.grain {
            Some(GrainAxisArg::V) => GrainAxis::V,
            Some(GrainAxisArg::Free) => GrainAxis::Free,
            _ => GrainAxis::U,
        },
        edges: PanelEdges {
            u_start: band(&args.band_u_start, d, e)?,
            u_end: band(&args.band_u_end, d, e)?,
            v_start: band(&args.band_v_start, d, e)?,
            v_end: band(&args.band_v_end, d, e)?,
        },
        oversize: opt(&args.oversize, d, e)?,
    };
    let request = PanelRequest::new(args.name, panel);
    let out = json!({ "part": request.body, "feature": request.feature });
    let expected = args.expected_revision;
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, expected)?;
        Ok::<_, ToolError>(
            model::prepare_panel(
                target.session(),
                expected,
                call_owned.actor.clone(),
                &request,
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, out)
}

pub(super) fn create_panel_stack(host: &dyn Host, call: &Request, args: StackArgs) -> ToolResult {
    let layers = args
        .layers
        .iter()
        .map(|l| id(l))
        .collect::<Result<Vec<_>, _>>()?;
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let (stack, prepared) = model::prepare_stack(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            &args.name,
            &layers,
            args.adhesive.clone(),
            args.group,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "stack": stack }))
    })?
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JointKindArg {
    ButtAThrough,
    ButtBThrough,
    Miter,
    Finger,
    /// Domino-style loose tenons between panels that meet face to edge.
    LooseTenon,
}
/// Loose-tenon settings; omitted values use the example Festool DOMINO
/// 8 × 22 × 40 mm tenon, 15 mm face and 28 mm end mortises, 6 mm face play,
/// two tenons 60 mm from the ends.
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TenonArgs {
    #[serde(default)]
    pub thickness: Option<Quantity>,
    #[serde(default)]
    pub width: Option<Quantity>,
    #[serde(default)]
    pub length: Option<Quantity>,
    /// Tenons per pair (1–32).
    #[serde(default)]
    pub count: Option<u32>,
    /// From each end of the contact to the first and last tenon center.
    #[serde(default)]
    pub inset: Option<Quantity>,
    /// Tenon center across the edge panel's thickness, from its middle.
    #[serde(default)]
    pub offset: Option<Quantity>,
    #[serde(default)]
    pub face_depth: Option<Quantity>,
    #[serde(default)]
    pub edge_depth: Option<Quantity>,
    /// Extra mortise length beyond the tenon width (0 = tight).
    #[serde(default)]
    pub face_play: Option<Quantity>,
    #[serde(default)]
    pub edge_play: Option<Quantity>,
    /// Extra mortise thickness beyond the tenon thickness.
    #[serde(default)]
    pub fit: Option<Quantity>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelJointArgs {
    pub expected_revision: u64,
    /// Joint ID to change; omit to create a new joint.
    #[serde(default)]
    pub joint: Option<String>,
    /// Name of a new joint.
    #[serde(default)]
    pub name: Option<String>,
    pub kind: JointKindArg,
    /// Finger joints: number of fingers along the corner (2–128).
    #[serde(default)]
    pub finger_count: Option<u32>,
    /// Finger joints: total gap at each finger face (default 0).
    #[serde(default)]
    pub fit: Option<Quantity>,
    /// New joints: [panel A Part, panel B Part] pairs that overlap at a
    /// corner, or (loose tenons) meet face to edge.
    #[serde(default)]
    pub pairs: Vec<[String; 2]>,
    /// Loose tenons: the tenon, spacing, mortise depths and fits.
    #[serde(default)]
    pub tenon: Option<TenonArgs>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteJointArgs {
    pub expected_revision: u64,
    pub joint: String,
}

pub(super) fn set_panel_joint(host: &dyn Host, call: &Request, args: PanelJointArgs) -> ToolResult {
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
    let kind = match args.kind {
        JointKindArg::ButtAThrough => PanelJointKind::ButtAThrough,
        JointKindArg::ButtBThrough => PanelJointKind::ButtBThrough,
        JointKindArg::Miter => PanelJointKind::Miter,
        JointKindArg::Finger => PanelJointKind::Finger {
            count: args
                .finger_count
                .ok_or_else(|| ToolError::invalid("A finger joint needs finger_count"))?,
            fit: opt(&args.fit, &document, &evaluated)?.unwrap_or_else(|| length(0.)),
        },
        JointKindArg::LooseTenon => {
            let t = args.tenon.unwrap_or_default();
            let (d, e) = (&document, &evaluated);
            let or = |v: &Option<Quantity>, default: f64| {
                opt(v, d, e).map(|v| v.unwrap_or_else(|| length(default)))
            };
            PanelJointKind::LooseTenon(Box::new(model::LooseTenonSpec {
                thickness: or(&t.thickness, 8.)?,
                width: or(&t.width, 22.)?,
                length: or(&t.length, 40.)?,
                count: t.count.unwrap_or(2),
                inset: or(&t.inset, 60.)?,
                offset: opt(&t.offset, d, e)?,
                face_depth: or(&t.face_depth, 15.)?,
                edge_depth: or(&t.edge_depth, 28.)?,
                face_play: or(&t.face_play, 6.)?,
                edge_play: or(&t.edge_play, 0.)?,
                fit: or(&t.fit, 0.)?,
            }))
        }
    };
    let existing = args.joint.as_deref().map(id).transpose()?;
    let pairs = args
        .pairs
        .iter()
        .map(|[a, b]| {
            Ok(PanelJointPair {
                a: id(a)?,
                b: id(b)?,
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    let name = args.name.clone();
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match existing {
            Some(joint) => model::prepare_panel_joint_edit(session, revision, actor, joint, &kind)?,
            None => model::prepare_panel_joint(
                session,
                revision,
                actor,
                name.as_deref()
                    .ok_or_else(|| ToolError::invalid("A new joint needs a name"))?,
                &PanelJointDefinition { kind, pairs },
            )?,
        })
    })??;
    let staged = prepared.model.prepared.document();
    let tenons: Vec<_> = model::joint_tenons(staged, prepared.joint)
        .into_iter()
        .map(|(pair, feature, part)| json!({ "pair": pair, "part": part, "feature": feature }))
        .collect();
    let out = json!({
        "joint": prepared.joint,
        "cuts": prepared.model.outputs.iter()
            .filter(|o| !tenons.iter().any(|t| t["feature"] == json!(o.feature)))
            .map(|o| json!({ "part": o.body, "feature": o.feature })).collect::<Vec<_>>(),
        "tenons": tenons,
    });
    finish(host, call, prepared.model.prepared, revision, out)
}

pub(super) fn delete_panel_joint(
    host: &dyn Host,
    call: &Request,
    args: DeleteJointArgs,
) -> ToolResult {
    let joint = id(&args.joint)?;
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        Ok::<_, ToolError>(model::prepare_panel_joint_delete(
            target.session(),
            args.expected_revision,
            call_owned.actor.clone(),
            joint,
        )?)
    })??;
    finish(
        host,
        call,
        prepared,
        args.expected_revision,
        json!({ "deleted": joint }),
    )
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeArg {
    UStart,
    UEnd,
    VStart,
    VEnd,
}
impl From<EdgeArg> for PanelEdgeName {
    fn from(e: EdgeArg) -> Self {
        match e {
            EdgeArg::UStart => Self::UStart,
            EdgeArg::UEnd => Self::UEnd,
            EdgeArg::VStart => Self::VStart,
            EdgeArg::VEnd => Self::VEnd,
        }
    }
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaceArg {
    Top,
    Bottom,
    Edge,
    /// Each panel's top or bottom face toward the carcass interior (or
    /// toward the other targets).
    Inside,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RowEdgeArg {
    Start,
    End,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShelfPinRowArg {
    /// The edge across the rows the offset is measured from.
    pub from: RowEdgeArg,
    pub offset: Quantity,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MachiningKindArg {
    Groove,
    Rabbet,
    Pocket,
    Drill,
    ShelfPins,
    Hardware,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MachiningArgs {
    pub expected_revision: u64,
    /// Operation ID to change; omit to create one.
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub kind: MachiningKindArg,
    /// top, bottom, inside or edge (drilling and hardware only; then give `edge`).
    pub face: FaceArg,
    /// Rabbets: the edge; edge faces: the edge drilled into.
    #[serde(default)]
    pub edge: Option<EdgeArg>,
    /// Panel Parts that receive the operation (fixed after creation).
    pub targets: Vec<String>,
    /// Grooves: "u" or "v", the axis the groove runs along. Shelf pins: the
    /// axis the rows run along (default v).
    #[serde(default)]
    pub along: Option<GrainAxisArg>,
    /// Shelf pins: the rows, each an offset from the start or end edge across them.
    #[serde(default)]
    pub rows: Vec<ShelfPinRowArg>,
    /// Shelf pins: hole pitch along a row (default 32 mm, System 32).
    #[serde(default)]
    pub pitch: Option<Quantity>,
    /// Shelf pins: first hole center from the start edge along the row.
    #[serde(default)]
    pub start: Option<Quantity>,
    /// Shelf pins: no hole center closer than this to the end edge.
    #[serde(default)]
    pub end: Option<Quantity>,
    /// Shelf pins: holes per row; omit to fill the range.
    #[serde(default)]
    pub count: Option<u32>,
    /// Grooves: distance from the start edge of the other axis.
    #[serde(default)]
    pub offset: Option<Quantity>,
    /// Grooves and rabbets: width.
    #[serde(default)]
    pub width: Option<Quantity>,
    /// Blind depth; omit for through holes.
    #[serde(default)]
    pub depth: Option<Quantity>,
    /// Pockets: center and size in face coordinates (mm).
    #[serde(default)]
    pub center_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub size_mm: Option<[f64; 2]>,
    /// Drilling: hole centers in face coordinates (mm).
    #[serde(default)]
    pub holes_mm: Vec<[f64; 2]>,
    #[serde(default)]
    pub diameter: Option<Quantity>,
    /// Hardware: pattern ID, placement and 0–3 quarter turns.
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub at_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub quarter_turns: u8,
    /// Keep the operation and its cuts but remove no material (default false).
    #[serde(default)]
    pub suppressed: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HardwareHoleArg {
    pub at_mm: [f64; 2],
    pub diameter_mm: f64,
    /// Omit for a through hole.
    #[serde(default)]
    pub depth_mm: Option<f64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllowanceArg {
    pub material: String,
    pub diameter_mm: f64,
    pub depth_mm: f64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HardwareArgs {
    pub expected_revision: u64,
    /// Pattern ID to change; omit to create one.
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub holes: Vec<HardwareHoleArg>,
    /// Per-material diameter and depth additions.
    #[serde(default)]
    pub allowances: Vec<AllowanceArg>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteMachiningArgs {
    pub expected_revision: u64,
    pub operation: String,
}

fn stale(revision: u64, expected: u64) -> Result<(), ToolError> {
    if revision == expected {
        return Ok(());
    }
    Err(ToolError::new(
        "stale_revision",
        format!("Request is based on revision {expected}, but the document is at {revision}"),
    )
    .at_revision(revision))
}

pub(super) fn set_hardware_pattern(
    host: &dyn Host,
    call: &Request,
    args: HardwareArgs,
) -> ToolResult {
    let pattern = HardwarePatternDefinition {
        holes: args
            .holes
            .iter()
            .map(|h| HardwareHole {
                at: h.at_mm.map(length),
                diameter: length(h.diameter_mm),
                depth: h.depth_mm.map(length),
            })
            .collect(),
        allowances: args
            .allowances
            .iter()
            .map(|a| {
                Ok(MaterialAllowance {
                    material: id(&a.material)?,
                    diameter: length(a.diameter_mm),
                    depth: length(a.depth_mm),
                })
            })
            .collect::<Result<_, ToolError>>()?,
    };
    let existing = args.pattern.as_deref().map(id).transpose()?;
    let call_owned = call.clone();
    let (pattern_id, prepared) = with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match existing {
            Some(p) => (
                p,
                model::prepare_hardware_edit(session, args.expected_revision, actor, p, &pattern)?,
            ),
            None => model::prepare_hardware(
                session,
                args.expected_revision,
                actor,
                args.name
                    .as_deref()
                    .ok_or_else(|| ToolError::invalid("A new pattern needs a name"))?,
                &pattern,
            )?,
        })
    })??;
    finish(
        host,
        call,
        prepared,
        args.expected_revision,
        json!({ "pattern": pattern_id }),
    )
}

pub(super) fn set_machining(host: &dyn Host, call: &Request, args: MachiningArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    stale(revision, args.expected_revision)?;
    let (d, e) = (&document, &evaluated);
    let need = |v: &Option<Quantity>, what: &str| {
        v.as_ref()
            .ok_or_else(|| ToolError::invalid(format!("This operation needs {what}")))
            .and_then(|v| q(v, d, e))
    };
    let edge = || {
        args.edge
            .map(PanelEdgeName::from)
            .ok_or_else(|| ToolError::invalid("Give the edge"))
    };
    let kind = match args.kind {
        MachiningKindArg::Groove => MachiningKind::Groove {
            along: match args.along {
                Some(GrainAxisArg::V) => PanelAxis::V,
                _ => PanelAxis::U,
            },
            offset: need(&args.offset, "an offset")?,
            width: need(&args.width, "a width")?,
            depth: need(&args.depth, "a depth")?,
        },
        MachiningKindArg::Rabbet => MachiningKind::Rabbet {
            edge: edge()?,
            width: need(&args.width, "a width")?,
            depth: need(&args.depth, "a depth")?,
        },
        MachiningKindArg::Pocket => MachiningKind::Pocket {
            center: args
                .center_mm
                .ok_or_else(|| ToolError::invalid("A pocket needs center_mm"))?
                .map(length),
            size: args
                .size_mm
                .ok_or_else(|| ToolError::invalid("A pocket needs size_mm"))?
                .map(length),
            depth: need(&args.depth, "a depth")?,
        },
        MachiningKindArg::Drill => MachiningKind::Drill {
            holes: args.holes_mm.iter().map(|h| h.map(length)).collect(),
            diameter: need(&args.diameter, "a diameter")?,
            depth: opt(&args.depth, d, e)?,
        },
        MachiningKindArg::ShelfPins => MachiningKind::ShelfPins {
            along: match args.along {
                Some(GrainAxisArg::U) => PanelAxis::U,
                _ => PanelAxis::V,
            },
            rows: args
                .rows
                .iter()
                .map(|r| {
                    Ok(model::ShelfPinRow {
                        from: match r.from {
                            RowEdgeArg::Start => model::RowEdge::Start,
                            RowEdgeArg::End => model::RowEdge::End,
                        },
                        offset: q(&r.offset, d, e)?,
                    })
                })
                .collect::<Result<_, ToolError>>()?,
            pitch: opt(&args.pitch, d, e)?.unwrap_or_else(|| length(model::SYSTEM32_PITCH_MM)),
            start: need(&args.start, "a start")?,
            end: need(&args.end, "an end limit")?,
            count: args.count,
            diameter: need(&args.diameter, "a diameter")?,
            depth: need(&args.depth, "a depth")?,
        },
        MachiningKindArg::Hardware => MachiningKind::Hardware {
            pattern: id(args
                .pattern
                .as_deref()
                .ok_or_else(|| ToolError::invalid("Hardware needs a pattern"))?)?,
            at: args
                .at_mm
                .ok_or_else(|| ToolError::invalid("Hardware needs at_mm"))?
                .map(length),
            quarter_turns: args.quarter_turns,
        },
    };
    let machining = MachiningDefinition {
        suppressed: args.suppressed,
        kind,
        face: match args.face {
            FaceArg::Top => MachiningFace::Top,
            FaceArg::Bottom => MachiningFace::Bottom,
            FaceArg::Edge => MachiningFace::Edge { edge: edge()? },
            FaceArg::Inside => MachiningFace::Inside,
        },
        targets: args
            .targets
            .iter()
            .map(|t| id(t))
            .collect::<Result<_, _>>()?,
    };
    let existing = args.operation.as_deref().map(id).transpose()?;
    let name = args.name.clone();
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match existing {
            Some(op) => model::prepare_machining_edit(session, revision, actor, op, &machining)?,
            None => model::prepare_machining(
                session,
                revision,
                actor,
                name.as_deref()
                    .ok_or_else(|| ToolError::invalid("A new operation needs a name"))?,
                &machining,
            )?,
        })
    })??;
    // Carcasses whose sides carry shelf pins report where a shelf can rest.
    let staged = prepared.model.prepared.document();
    let carcasses: std::collections::BTreeSet<EntityId> = prepared
        .model
        .outputs
        .iter()
        .filter_map(|o| model::panel_feature(staged, o.body))
        .filter_map(|f| model::read_panel(staged, f).ok())
        .filter_map(|p| match p.source {
            PanelSource::Carcass { carcass, .. } => Some(carcass),
            _ => None,
        })
        .collect();
    let supports = multicad_core::parameters::evaluate(staged)
        .ok()
        .map(|parameters| {
            carcasses
                .iter()
                .filter_map(|c| {
                    model::shelf_support_offsets(staged, &parameters, *c)
                        .ok()
                        .filter(|l| !l.is_empty())
                        .map(|l| json!({ "carcass": c, "shelf_offsets_mm": l }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let out = json!({
        "operation": prepared.operation,
        "cuts": prepared.model.outputs.iter().map(|o| json!({ "part": o.body, "feature": o.feature })).collect::<Vec<_>>(),
        "shelf_supports": supports,
    });
    finish(host, call, prepared.model.prepared, revision, out)
}

pub(super) fn delete_machining(
    host: &dyn Host,
    call: &Request,
    args: DeleteMachiningArgs,
) -> ToolResult {
    let op = id(&args.operation)?;
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        Ok::<_, ToolError>(model::prepare_machining_delete(
            target.session(),
            args.expected_revision,
            call_owned.actor.clone(),
            op,
        )?)
    })??;
    finish(
        host,
        call,
        prepared,
        args.expected_revision,
        json!({ "deleted": op }),
    )
}

pub(super) fn cut_list(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let records = model.cut_list(&document)?;
    let stacks = model.stacks(&document)?;
    let machining = model.machining(&document)?;
    Ok(json!({
        "revision": revision,
        "scope": "Sizes measured from the evaluated panel solids. Core = finished − band thickness + band allowance per banded edge, + oversize. Not a nesting, yield or machining plan.",
        "panels": records.iter().map(|r| json!({
            "part": r.part, "name": r.name, "carcass": r.carcass,
            "role": r.role.map(|role| role.label()), "stack": r.stack, "joints": r.joints,
            "operations": r.operations,
            "material": r.material, "stock": r.stock, "stock_name": r.stock_name,
            "thickness_mm": r.cut.thickness_mm,
            "finished_mm": r.cut.finished_mm, "core_mm": r.cut.core_mm,
            "grain_fixed": r.cut.grain_fixed,
            "bands": r.bands.iter().map(|(edge, b)| json!({
                "edge": edge, "thickness_mm": b.thickness_mm, "allowance_mm": b.allowance_mm,
            })).collect::<Vec<_>>(),
            "oversize_mm": r.oversize_mm, "volume_mm3": r.volume_mm3,
            "fits_stock": r.fits_stock, "issues": r.issues,
        })).collect::<Vec<_>>(),
        "machining": machining.iter().map(|m| json!({
            "part": m.part, "operation": m.operation, "name": m.name, "kind": m.kind,
            "face": m.face, "tenon": m.tenon,
            "material": m.material, "stock": m.stock, "removed_mm3": m.removed_mm3,
            "holes": m.holes.iter().map(|h| json!({
                "face_mm": h.at,
                "center_mm": h.center, "axis": h.axis, "diameter_mm": h.diameter_mm,
                "depth_mm": h.depth_mm, "through": h.through,
            })).collect::<Vec<_>>(),
            "recesses": m.recesses.iter().map(|r| json!({
                "kind": r.kind, "size_mm": r.size_mm, "depth_mm": r.depth_mm,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "stacks": stacks.iter().map(|s| json!({
            "stack": s.stack, "name": s.name, "adhesive": s.adhesive,
            "face_to_face": s.face_to_face, "total_thickness_mm": s.total_thickness_mm,
            "layers": s.layers.iter().map(|l| json!({
                "part": l.part, "material": l.material, "stock": l.stock,
                "thickness_mm": l.thickness_mm,
            })).collect::<Vec<_>>(),
            "issues": s.issues,
        })).collect::<Vec<_>>(),
    }))
}
