//! Sheet-metal tools over the shared sheet API. Each mutation builds the
//! sheet natively before it commits one transaction; records and flat
//! patterns are measured from the evaluated solids.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    materials::{self, LibraryDefinition, Stock, StockKind},
    model::{
        self, BendDirection, BodyOutput, Cutout, CutoutFace, CutoutShape, Flange, FlangeKind,
        PanelPlane, ProfileSource, Relief, SheetBase, SheetDefinition, SheetRequest,
    },
    parameters::EvaluatedParameters,
};
use multicad_kernel::NativeRegionQueries;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetStockArgs {
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
    /// Default inside bend radius for parts of this stock.
    pub inside_bend_radius_mm: f64,
    /// Default neutral-axis position as a fraction of thickness, 0–1.
    pub k_factor: f64,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SheetPlaneArg {
    Xy,
    Xz,
    Yz,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetPartArgs {
    pub expected_revision: u64,
    /// Sheet feature to edit; omit to create a new sheet Part.
    #[serde(default)]
    pub feature: Option<String>,
    /// Name of a new sheet Part.
    #[serde(default)]
    pub name: Option<String>,
    pub stock: String,
    /// Rectangle base: principal plane (u, v = X, Y / X, Z / Y, Z). Give
    /// plane, origin_mm, length and width, or sketch and region.
    #[serde(default)]
    pub plane: Option<SheetPlaneArg>,
    /// Rectangle base: world position of the minimum corner in millimeters.
    #[serde(default)]
    pub origin_mm: Option<[f64; 3]>,
    /// Rectangle base: size along u.
    #[serde(default)]
    pub length: Option<Quantity>,
    /// Rectangle base: size along v.
    #[serde(default)]
    pub width: Option<Quantity>,
    /// Sketch-region base: the sketch.
    #[serde(default)]
    pub sketch: Option<String>,
    /// Sketch-region base: region index reported by inspect_sketch at expected_revision.
    #[serde(default)]
    pub region: Option<usize>,
    /// Put the material on the other side of the base face (default false).
    #[serde(default)]
    pub reverse: bool,
    /// Inside bend radius for this part; omit to use the stock value.
    #[serde(default)]
    pub inside_radius: Option<Quantity>,
    /// K-factor for this part (0–1); omit to use the stock value.
    #[serde(default)]
    pub k_factor: Option<f64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConvertSheetArgs {
    pub expected_revision: u64,
    /// The Part whose current solid is converted; it stays unchanged.
    pub part: String,
    pub stock: String,
    pub name: String,
    /// Report the analysis without creating anything (default false).
    #[serde(default)]
    pub analyze_only: bool,
}

fn q(q: &Quantity, d: &Document, e: &EvaluatedParameters) -> Result<Dimension, ToolError> {
    Ok(q.dimension(d, e)?.0)
}
fn stale(expected: u64, revision: u64) -> ToolError {
    ToolError::new(
        "stale_revision",
        format!("Request is based on revision {expected}, but the document is at {revision}"),
    )
    .at_revision(revision)
}
/// Build the sheet natively, then commit.
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: Prepared,
    revision: u64,
    mut out: Value,
) -> ToolResult {
    let model = compile(prepared.document(), revision, call)?;
    let records = model.sheet_metal(prepared.document())?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    out["revision"] = json!(committed);
    out["sheet_parts"] = json!(records.len());
    Ok(out)
}

pub(super) fn create_sheet_stock(
    host: &dyn Host,
    call: &Request,
    args: SheetStockArgs,
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
                        color_rgba: [176, 180, 186, 255],
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
            specification: StockKind::Sheet {
                thickness_mm: args.thickness_mm,
                length_mm: args.sheet_length_mm,
                width_mm: args.sheet_width_mm,
                inside_bend_radius_mm: args.inside_bend_radius_mm,
                k_factor: args.k_factor,
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

fn base(
    args: &SheetPartArgs,
    document: &Document,
    revision: u64,
    evaluated: &EvaluatedParameters,
    call: &Request,
) -> Result<SheetBase, ToolError> {
    let (d, e) = (document, evaluated);
    match (&args.sketch, args.region, args.plane) {
        (Some(sketch), Some(index), None) => {
            let sketch = id(sketch)?;
            let evaluated_sketch = multicad_solver::evaluate_document_sketch_with_parameters(
                document, revision, sketch, None, evaluated,
            )?;
            let interrupt = || call.cancelled();
            let extraction = call.finish(evaluated_sketch.regions_with_queries_and_interrupt(
                &NativeRegionQueries::default(),
                Some(&interrupt),
            ))?;
            let region = extraction.regions.get(index).ok_or_else(|| {
                ToolError::invalid(format!(
                    "Region {index} does not exist; the sketch has {} regions",
                    extraction.regions.len()
                ))
                .involving(sketch)
            })?;
            Ok(SheetBase::Profile {
                profile: ProfileSource {
                    sketch,
                    region: region.intent(),
                },
            })
        }
        (None, None, Some(plane)) => {
            let (Some(length), Some(width)) = (&args.length, &args.width) else {
                return Err(ToolError::invalid(
                    "A rectangle base needs length and width",
                ));
            };
            Ok(SheetBase::Rectangle {
                plane: match plane {
                    SheetPlaneArg::Xy => PanelPlane::Xy,
                    SheetPlaneArg::Xz => PanelPlane::Xz,
                    SheetPlaneArg::Yz => PanelPlane::Yz,
                },
                origin: args.origin_mm.unwrap_or([0.; 3]).map(model::length),
                length: q(length, d, e)?,
                width: q(width, d, e)?,
            })
        }
        _ => Err(ToolError::invalid(
            "Give either plane, length and width, or sketch and region",
        )),
    }
}

pub(super) fn set_sheet_part(host: &dyn Host, call: &Request, args: SheetPartArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(stale(args.expected_revision, revision));
    }
    let edit = args.feature.as_deref().map(id).transpose()?;
    let keep_base =
        edit.is_some() && args.plane.is_none() && args.sketch.is_none() && args.region.is_none();
    let base = match edit {
        Some(feature) if keep_base => model::read_sheet(&document, feature)?.base,
        _ => base(&args, &document, revision, &evaluated, call)?,
    };
    let output = match edit {
        Some(feature) => model::read_sheet(&document, feature)?.output,
        None => BodyOutput {
            body: EntityId::new(),
        },
    };
    let sheet = SheetDefinition {
        output,
        stock: id(&args.stock)?,
        base,
        reverse: args.reverse,
        inside_radius: args
            .inside_radius
            .as_ref()
            .map(|r| q(r, &document, &evaluated))
            .transpose()?,
        k_factor: args.k_factor.map(|value| Dimension::Literal {
            value,
            unit: "ratio".into(),
        }),
        flanges: vec![],
        cutouts: vec![],
    };
    // An edit keeps the part's flanges and cutouts.
    let mut sheet = sheet;
    if let Some(feature) = edit {
        let previous = model::read_sheet(&document, feature)?;
        sheet.flanges = previous.flanges;
        sheet.cutouts = previous.cutouts;
    }
    let name = args.name.clone();
    let call_owned = call.clone();
    let (prepared, out) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match edit {
            Some(feature) => {
                let p = model::prepare_sheet_edit(session, revision, actor, feature, &sheet)?;
                (
                    p.prepared,
                    json!({ "feature": feature, "part": sheet.output.body }),
                )
            }
            None => {
                let name =
                    name.ok_or_else(|| ToolError::invalid("A new sheet part needs a name"))?;
                let request = SheetRequest::new(name, sheet);
                let out = json!({ "feature": request.feature, "part": request.body });
                (
                    model::prepare_sheet(session, revision, actor, &request)?.prepared,
                    out,
                )
            }
        })
    })??;
    finish(host, call, prepared, revision, out)
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlangeKindArg {
    Bend,
    Hem,
    Jog,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BendDirectionArg {
    Up,
    Down,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlangeArg {
    /// Straight base edge index; rectangle edges are 0: v = 0, 1: u = length, 2: v = width, 3: u = 0.
    pub edge: u32,
    pub kind: FlangeKindArg,
    /// Bend and jog angle in degrees (bend: 0–180, jog: 0–90; default 90).
    #[serde(default)]
    pub angle_deg: Option<f64>,
    /// Hem: gap between the hem leg and the base face.
    #[serde(default)]
    pub gap: Option<Quantity>,
    /// Jog: offset between the base face and the leg's matching face.
    #[serde(default)]
    pub offset: Option<Quantity>,
    /// Up bends toward the material side (default), down away from it.
    #[serde(default)]
    pub direction: Option<BendDirectionArg>,
    /// Straight leg beyond the last bend.
    pub length: Quantity,
    /// Inside radius for this flange; omit to use the part value.
    #[serde(default)]
    pub inside_radius: Option<Quantity>,
    /// Gap from the edge start to the flange (corner or seam gap).
    #[serde(default)]
    pub start_inset: Option<Quantity>,
    /// Gap from the flange to the edge end.
    #[serde(default)]
    pub end_inset: Option<Quantity>,
    /// Rectangular relief at each inset end: width along the edge.
    #[serde(default)]
    pub relief_width: Option<Quantity>,
    /// Relief depth into the base.
    #[serde(default)]
    pub relief_depth: Option<Quantity>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CutoutShapeArg {
    Circle,
    Rectangle,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CutoutArg {
    /// Flange index (zero-based) for a cutout in a flange leg; omit for the base.
    #[serde(default)]
    pub flange: Option<u32>,
    pub shape: CutoutShapeArg,
    /// Center in the face's coordinates: base (u, v), or flange (along the
    /// edge from the flange start, along the leg from its last bend).
    pub center_mm: [f64; 2],
    /// Circle diameter.
    #[serde(default)]
    pub diameter: Option<Quantity>,
    /// Rectangle size: along u (or the edge) and along v (or the leg).
    #[serde(default)]
    pub size: Option<[Quantity; 2]>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetFormingArgs {
    pub expected_revision: u64,
    /// The sheet feature whose flanges and cutouts are replaced.
    pub feature: String,
    #[serde(default)]
    pub flanges: Vec<FlangeArg>,
    #[serde(default)]
    pub cutouts: Vec<CutoutArg>,
}

fn opt(
    v: &Option<Quantity>,
    d: &Document,
    e: &EvaluatedParameters,
) -> Result<Option<Dimension>, ToolError> {
    v.as_ref().map(|v| q(v, d, e)).transpose()
}
fn need(
    v: &Option<Quantity>,
    what: &str,
    d: &Document,
    e: &EvaluatedParameters,
) -> Result<Dimension, ToolError> {
    opt(v, d, e)?.ok_or_else(|| ToolError::invalid(format!("This needs {what}")))
}
fn flange(f: &FlangeArg, d: &Document, e: &EvaluatedParameters) -> Result<Flange, ToolError> {
    let angle = model::angle(f.angle_deg.unwrap_or(90.));
    Ok(Flange {
        edge: f.edge,
        kind: match f.kind {
            FlangeKindArg::Bend => FlangeKind::Bend { angle },
            FlangeKindArg::Hem => FlangeKind::Hem {
                gap: need(&f.gap, "a hem gap", d, e)?,
            },
            FlangeKindArg::Jog => FlangeKind::Jog {
                angle,
                offset: need(&f.offset, "a jog offset", d, e)?,
            },
        },
        direction: match f.direction {
            Some(BendDirectionArg::Down) => BendDirection::Down,
            _ => BendDirection::Up,
        },
        length: q(&f.length, d, e)?,
        inside_radius: opt(&f.inside_radius, d, e)?,
        start_inset: opt(&f.start_inset, d, e)?,
        end_inset: opt(&f.end_inset, d, e)?,
        relief: match (&f.relief_width, &f.relief_depth) {
            (None, None) => None,
            (Some(w), Some(depth)) => Some(Relief {
                width: q(w, d, e)?,
                depth: q(depth, d, e)?,
            }),
            _ => {
                return Err(ToolError::invalid(
                    "A relief needs both relief_width and relief_depth",
                ));
            }
        },
    })
}
fn cutout(c: &CutoutArg, d: &Document, e: &EvaluatedParameters) -> Result<Cutout, ToolError> {
    let center = c.center_mm.map(model::length);
    Ok(Cutout {
        face: match c.flange {
            Some(index) => CutoutFace::Flange { index },
            None => CutoutFace::Base,
        },
        shape: match c.shape {
            CutoutShapeArg::Circle => CutoutShape::Circle {
                center,
                diameter: need(&c.diameter, "a diameter", d, e)?,
            },
            CutoutShapeArg::Rectangle => {
                let [a, b] = c
                    .size
                    .as_ref()
                    .ok_or_else(|| ToolError::invalid("A rectangle cutout needs a size"))?;
                CutoutShape::Rectangle {
                    center,
                    size: [q(a, d, e)?, q(b, d, e)?],
                }
            }
        },
    })
}

pub(super) fn set_sheet_forming(
    host: &dyn Host,
    call: &Request,
    args: SheetFormingArgs,
) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(stale(args.expected_revision, revision));
    }
    let feature = id(&args.feature)?;
    let mut sheet = model::read_sheet(&document, feature)?;
    sheet.flanges = args
        .flanges
        .iter()
        .map(|f| flange(f, &document, &evaluated))
        .collect::<Result<_, _>>()?;
    sheet.cutouts = args
        .cutouts
        .iter()
        .map(|c| cutout(c, &document, &evaluated))
        .collect::<Result<_, _>>()?;
    let out = json!({ "feature": feature, "part": sheet.output.body });
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(
            model::prepare_sheet_edit(
                target.session(),
                revision,
                call_owned.actor.clone(),
                feature,
                &sheet,
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, out)
}

fn analysis_json(a: &multicad_model::ConversionAnalysis) -> Value {
    json!({
        "convertible": a.convertible(),
        "thickness_mm": a.thickness_mm,
        "stationary_face": a.stationary.as_ref().map(|(frame, loops)| json!({
            "origin_mm": frame.origin, "u": frame.u, "v": frame.v,
            "thickness_direction": frame.normal(), "loops": loops.len(),
        })),
        "planar_faces": a.planar_faces,
        "bend_candidates_mm": a.bend_candidates,
        "issues": a.issues,
    })
}

pub(super) fn convert_to_sheet(
    host: &dyn Host,
    call: &Request,
    args: ConvertSheetArgs,
) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(stale(args.expected_revision, revision));
    }
    let part = id(&args.part)?;
    let stock = id(&args.stock)?;
    let spec = model::read_sheet_stock(&document, stock)?;
    let thickness = model::sheet_stock_values(&spec).map(|v| v.0);
    let model = compile(&document, revision, call)?;
    let analysis = model.analyze_sheet_conversion(part, thickness)?;
    let mut out = json!({ "analysis": analysis_json(&analysis) });
    if args.analyze_only {
        out["revision"] = json!(revision);
        return Ok(out);
    }
    let request = analysis.request(&args.name, stock)?;
    out["feature"] = json!(request.feature);
    out["part"] = json!(request.body);
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(
            model::prepare_sheet(
                target.session(),
                revision,
                call_owned.actor.clone(),
                &request,
            )?
            .prepared,
        )
    })??;
    finish(host, call, prepared, revision, out)
}

pub(super) fn sheet_metal(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let records = model.sheet_metal(&document)?;
    Ok(json!({
        "revision": revision,
        "scope": "Measured from the evaluated sheet solids. The flat pattern is the developed blank of the same feature in base-frame millimeters (outer loop counterclockwise, openings clockwise). Bend inputs state whether they come from the stock or the part; they are not machine or tooling data.",
        "sheet_parts": records.iter().map(|r| json!({
            "part": r.part, "feature": r.feature, "name": r.name,
            "material": r.material, "stock": r.stock, "stock_name": r.stock_name,
            "base": r.base, "converted_from": r.converted_from,
            "thickness_mm": r.thickness_mm, "measured_thickness_mm": r.measured_thickness_mm,
            "inside_radius_mm": r.inside_radius_mm, "radius_source": r.radius_source,
            "k_factor": r.k_factor, "k_factor_source": r.k_factor_source,
            "volume_mm3": r.volume_mm3, "solid_count": r.solid_count,
            "flat_size_mm": r.flat_size_mm, "flat_area_mm2": r.flat_area_mm2,
            "flat": r.flat,
            "bends": r.bends.iter().map(|b| json!({
                "flange": b.flange, "bend": b.bend, "edge": b.edge, "direction": b.direction,
                "angle_deg": b.angle_deg, "inside_radius_mm": b.inside_radius_mm,
                "allowance_mm": b.allowance_mm, "zone_mm": b.zone_mm, "line_mm": b.line,
                "length_mm": b.length_mm,
            })).collect::<Vec<_>>(),
            "issues": r.issues,
        })).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnfoldSheetArgs {
    /// The sheet Part to unfold; its folded geometry is kept.
    pub part: String,
}

/// The developed pattern as JSON: the producer contract for flat outputs.
pub(super) fn pattern_json(p: &multicad_model::DevelopedPattern) -> Value {
    json!({
        "part": p.part, "feature": p.feature, "name": p.name, "units": p.units,
        "frame": { "origin_mm": p.frame.origin_mm, "u": p.frame.u, "v": p.frame.v },
        "thickness_mm": p.thickness_mm, "k_factor": p.k_factor,
        "k_factor_source": p.k_factor_source,
        "size_mm": p.size_mm, "area_mm2": p.area_mm2,
        "cut": p.cut,
        "bends": p.bends.iter().map(|b| json!({
            "id": b.id, "order": b.order, "flange": b.record.flange, "edge": b.record.edge,
            "direction": b.record.direction, "angle_deg": b.record.angle_deg,
            "inside_radius_mm": b.record.inside_radius_mm,
            "allowance_mm": b.record.allowance_mm, "zone_mm": b.record.zone_mm,
            "line_mm": b.record.line, "length_mm": b.record.length_mm,
        })).collect::<Vec<_>>(),
        "drills": p.drills.iter().map(|d| json!({
            "id": d.id, "center_mm": d.center, "diameter_mm": d.diameter_mm,
        })).collect::<Vec<_>>(),
        "diagnostics": p.diagnostics.iter().map(|d| json!({
            "kind": d.kind, "scope": d.scope.label(), "subject": d.subject,
            "message": d.message,
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn unfold_sheet(host: &dyn Host, call: &Request, args: UnfoldSheetArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let part = id(&args.part)?;
    let model = compile(&document, revision, call)?;
    let pattern = model.unfold(&document, part)?;
    let mut out = pattern_json(&pattern);
    out["revision"] = json!(revision);
    out["scope"] = json!(
        "Developed from the sheet feature with the stated K-factor and inside radii. Cut loops are closed (outer counterclockwise, openings clockwise) in base-frame millimeters. The bend order follows the feature order and is not a press-brake sequence. Rule-of-thumb diagnostics are not machine or process approval."
    );
    Ok(out)
}
