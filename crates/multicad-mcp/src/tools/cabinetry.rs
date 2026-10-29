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
        PanelDefinition, PanelEdges, PanelJointDefinition, PanelJointKind, PanelJointPair,
        PanelPlane, PanelRequest, PanelSource, ShelfLayout, Shelves, angle, length,
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
    /// New joints: [panel A Part, panel B Part] pairs that overlap at a corner.
    #[serde(default)]
    pub pairs: Vec<[String; 2]>,
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
    let out = json!({
        "joint": prepared.joint,
        "cuts": prepared.model.outputs.iter().map(|o| json!({ "part": o.body, "feature": o.feature })).collect::<Vec<_>>(),
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

pub(super) fn cut_list(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let records = model.cut_list(&document)?;
    let stacks = model.stacks(&document)?;
    Ok(json!({
        "revision": revision,
        "scope": "Sizes measured from the evaluated panel solids. Core = finished − band thickness + band allowance per banded edge, + oversize. Not a nesting, yield or machining plan.",
        "panels": records.iter().map(|r| json!({
            "part": r.part, "name": r.name, "carcass": r.carcass,
            "role": r.role.map(|role| role.label()), "stack": r.stack, "joints": r.joints,
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
