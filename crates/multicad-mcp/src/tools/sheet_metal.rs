//! Sheet-metal tools over the shared sheet API. Each mutation builds the
//! sheet natively before it commits one transaction; records and flat
//! patterns are measured from the evaluated solids.
use super::{
    Quantity, Request, ToolResult, commit, compile, id, revision_guard, sketch_support_report,
    snapshot,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    exchange::LengthUnit,
    materials::{self, LibraryDefinition, Stock, StockKind},
    model::{
        self, BendDirection, BodyOutput, Cutout, CutoutFace, CutoutShape, Flange, FlangeKind,
        HardwarePreset, HardwareSide, MountingPattern, PanelPlane, ProfileSource, Relief,
        SheetBase, SheetDefinition, SheetRequest, Thread,
    },
    parameters::EvaluatedParameters,
};
use multicad_kernel::NativeRegionQueries;
use multicad_model::export::{Exporter, sheet_dxf::SheetDxfLayers};
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
            let report = sketch_support_report(document, revision, sketch, call)?;
            let evaluated_sketch = multicad_solver::evaluate_document_sketch_with_parameters(
                document,
                revision,
                sketch,
                report.as_ref(),
                evaluated,
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
        fasteners: vec![],
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
    // Report every invalid bend and touching corner at once, before the
    // native evaluator stops at the first one.
    let issues = model::check_flanges(&document, &evaluated, &sheet)?;
    if !issues.is_empty() {
        return Err(ToolError::new(
            "sheet_flanges_refused",
            issues
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        )
        .involving(feature));
    }
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

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetEdgesArgs {
    /// The sheet feature whose base edges and flanges are reported.
    pub feature: String,
    /// Candidate flanges to review instead of the applied ones; nothing is
    /// changed. Uses the same fields as set_sheet_forming.
    #[serde(default)]
    pub flanges: Option<Vec<FlangeArg>>,
}

fn source_json(s: model::ValueSource) -> &'static str {
    match s {
        model::ValueSource::Stock => "stock",
        model::ValueSource::Part => "part",
    }
}

/// Base edges, flange guides and pre-check findings of one sheet feature,
/// applied or with candidate flanges.
pub(super) fn sheet_edges(host: &dyn Host, _: &Request, args: SheetEdgesArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let feature = id(&args.feature)?;
    let mut sheet = model::read_sheet(&document, feature)?;
    let candidate = args.flanges.is_some();
    if let Some(flanges) = &args.flanges {
        sheet.flanges = flanges
            .iter()
            .map(|f| flange(f, &document, &evaluated))
            .collect::<Result<_, _>>()?;
    }
    let review = model::review_flanges(&document, &evaluated, &sheet)?;
    let edges = review.outline.as_ref().map(|o| {
        o.edges
            .iter()
            .map(|e| {
                json!({
                    "edge": e.index, "label": format!("Edge {}", e.index + 1),
                    "start_mm": e.start_mm, "end_mm": e.end_mm, "length_mm": e.length_mm,
                    "outward": e.outward, "straight": e.straight,
                    "flange": sheet.flanges.iter().position(|f| f.edge as usize == e.index),
                })
            })
            .collect::<Vec<_>>()
    });
    let guides = review.outline.as_ref().map_or(vec![], |o| {
        review
            .guides
            .iter()
            .map(|g| {
                let mid = g.width_mm() / 2.;
                json!({
                    "flange": g.flange, "edge": g.edge, "kind": g.kind,
                    "direction": match g.direction {
                        BendDirection::Up => "up",
                        BendDirection::Down => "down",
                    },
                    "bend_angles_deg": g.bends.iter().map(|b| b.1).collect::<Vec<_>>(),
                    "inside_radius_mm": g.inside_radius_mm, "leg_mm": g.leg_mm,
                    "span_mm": g.span_mm,
                    "leg_start_mm": g.world(o, g.leg[0], mid),
                    "leg_end_mm": g.world(o, g.leg[1], mid),
                    "section_mm": g.section,
                })
            })
            .collect()
    });
    Ok(json!({
        "revision": revision,
        "feature": feature,
        "candidate": candidate,
        "scope": "Base edges are numbered by their index in the base outer loop (the flange `edge`); labels count from 1. Guides are the mid-thickness cross-section of each flange (x outward from the edge, y along the material direction) from the stock thickness and the resolved inside radius; they annotate the folded part and are not manufacturing geometry. Issues are pre-check findings; collisions with the base or distant flanges are found only by native evaluation.",
        "thickness_mm": review.thickness_mm,
        "inside_radius_mm": review.inside_radius_mm,
        "radius_source": source_json(review.radius_source),
        "k_factor": review.k_factor,
        "k_factor_source": source_json(review.k_factor_source),
        "material_direction": review.outline.as_ref().map(|o| o.material),
        "edges": edges,
        "flanges": guides,
        "issues": review.issues.iter().map(|i| json!({
            "flanges": i.flanges, "edges": i.edges, "message": i.message,
        })).collect::<Vec<_>>(),
    }))
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

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DxfUnitArg {
    Mm,
    Cm,
    M,
    In,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetDxfArgs {
    /// The sheet Part whose developed blank is exported.
    pub part: String,
    /// Bare file name ending in .dxf; written inside the export folder.
    pub file_name: String,
    /// File units (default mm). Coordinates are scaled 1:1 into them.
    #[serde(default)]
    pub units: Option<DxfUnitArg>,
}

pub(super) fn export_sheet_dxf(host: &dyn Host, call: &Request, args: SheetDxfArgs) -> ToolResult {
    let directory = host.scope().export_directory.ok_or_else(|| {
        ToolError::new(
            "out_of_scope",
            "No export folder is in scope; start the server with --export-dir",
        )
    })?;
    let name = &args.file_name;
    let valid = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && name.to_ascii_lowercase().ends_with(".dxf");
    if !valid {
        return Err(ToolError::new(
            "out_of_scope",
            "file_name must be a bare name ending in .dxf",
        ));
    }
    let part = id(&args.part)?;
    let units = match args.units {
        Some(DxfUnitArg::Cm) => LengthUnit::Centimeter,
        Some(DxfUnitArg::M) => LengthUnit::Meter,
        Some(DxfUnitArg::In) => LengthUnit::Inch,
        _ => LengthUnit::Millimeter,
    };
    let (document, revision, _) = snapshot(host)?;
    let interrupt = || call.cancelled();
    let file = Exporter::default()
        .developed_sheet_dxf(
            &document,
            revision,
            part,
            units,
            &SheetDxfLayers::default(),
            Some(&interrupt),
        )
        .map_err(|e| ToolError::new("export_failed", e.message.clone()))?;
    // Publish only bytes made from the current source.
    let (current, current_revision, _) = snapshot(host)?;
    file.check_current(&current, current_revision)
        .map_err(|e| ToolError::new("stale_revision", e.message.clone()))?;
    call.check()?;
    let path = directory.join(name);
    let partial = directory.join(format!(".{name}.partial"));
    let written = std::fs::write(&partial, &file.bytes)
        .and_then(|()| std::fs::read(&partial))
        .map_err(|e| ToolError::new("export_failed", e.to_string()))
        .and_then(|back| {
            if back == file.bytes {
                std::fs::rename(&partial, &path)
                    .map_err(|e| ToolError::new("export_failed", e.to_string()))
            } else {
                Err(ToolError::new(
                    "export_failed",
                    "DXF readback differs from the developed output",
                ))
            }
        });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    let mut out = pattern_json(&file.pattern);
    out["revision"] = json!(revision);
    out["path"] = json!(path);
    out["bytes"] = json!(file.bytes.len());
    out["file_units"] = json!(match units {
        LengthUnit::Millimeter => "mm",
        LengthUnit::Centimeter => "cm",
        LengthUnit::Meter => "m",
        LengthUnit::Inch => "in",
    });
    out["layers"] = json!({
        "CUT": "outer cut contour", "CUT_INNER": "openings, holes",
        "BEND_UP": "bend center lines bending toward the material side",
        "BEND_DOWN": "bend center lines bending away from it",
        "DRILL": "circular cutout centers", "LABEL": "bend, cutout and part labels (non-cut)",
    });
    Ok(out)
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HardwareKindArg {
    Screw,
    Standoff,
    Insert,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
pub enum ThreadArg {
    M3,
    M4,
    M5,
    M6,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HardwareSideArg {
    Material,
    Base,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatternArg {
    pub kind: HardwareKindArg,
    /// Screw and standoff thread (ISO metric coarse).
    #[serde(default)]
    pub thread: Option<ThreadArg>,
    /// Screw or standoff length.
    #[serde(default)]
    pub length: Option<Quantity>,
    /// Screw: ISO 7089 washers under the head and the nut.
    #[serde(default)]
    pub washers: bool,
    /// Screw: an ISO 4032 nut on the other side.
    #[serde(default)]
    pub nut: bool,
    /// Standoff width across flats (user value).
    #[serde(default)]
    pub across_flats: Option<Quantity>,
    /// Insert: catalogue designation and the source of its dimensions.
    #[serde(default)]
    pub designation: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub hole_diameter: Option<Quantity>,
    #[serde(default)]
    pub body_diameter: Option<Quantity>,
    #[serde(default)]
    pub height: Option<Quantity>,
    /// Insert: supported sheet thickness range from the catalogue.
    #[serde(default)]
    pub min_sheet: Option<Quantity>,
    #[serde(default)]
    pub max_sheet: Option<Quantity>,
    /// Flange index (zero-based) for a flange leg; omit for the base.
    #[serde(default)]
    pub flange: Option<u32>,
    /// Side of the face the head or body sits on (default material).
    #[serde(default)]
    pub side: Option<HardwareSideArg>,
    /// Points in the face's coordinates, like cutout centers.
    pub points_mm: Vec<[f64; 2]>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetFastenersArgs {
    pub expected_revision: u64,
    /// The sheet feature whose mounting patterns are replaced.
    pub feature: String,
    #[serde(default)]
    pub patterns: Vec<PatternArg>,
}

fn pattern(
    p: &PatternArg,
    d: &Document,
    e: &EvaluatedParameters,
) -> Result<MountingPattern, ToolError> {
    let thread = || {
        p.thread
            .map(|t| match t {
                ThreadArg::M3 => Thread::M3,
                ThreadArg::M4 => Thread::M4,
                ThreadArg::M5 => Thread::M5,
                ThreadArg::M6 => Thread::M6,
            })
            .ok_or_else(|| ToolError::invalid("This hardware needs a thread"))
    };
    let text = |v: &Option<String>, what: &str| {
        v.clone()
            .ok_or_else(|| ToolError::invalid(format!("An insert needs {what}")))
    };
    Ok(MountingPattern {
        preset: match p.kind {
            HardwareKindArg::Screw => HardwarePreset::Screw {
                thread: thread()?,
                length: need(&p.length, "a length", d, e)?,
                washers: p.washers,
                nut: p.nut,
            },
            HardwareKindArg::Standoff => HardwarePreset::Standoff {
                thread: thread()?,
                length: need(&p.length, "a length", d, e)?,
                across_flats: need(&p.across_flats, "a width across flats", d, e)?,
            },
            HardwareKindArg::Insert => HardwarePreset::Insert {
                designation: text(&p.designation, "a designation")?,
                source: text(&p.source, "a catalogue source")?,
                hole_diameter: need(&p.hole_diameter, "a hole diameter", d, e)?,
                body_diameter: need(&p.body_diameter, "a body diameter", d, e)?,
                height: need(&p.height, "a height", d, e)?,
                min_sheet: need(&p.min_sheet, "a minimum sheet thickness", d, e)?,
                max_sheet: need(&p.max_sheet, "a maximum sheet thickness", d, e)?,
            },
        },
        face: match p.flange {
            Some(index) => CutoutFace::Flange { index },
            None => CutoutFace::Base,
        },
        side: match p.side {
            Some(HardwareSideArg::Base) => HardwareSide::Base,
            _ => HardwareSide::Material,
        },
        points: p.points_mm.iter().map(|q| q.map(model::length)).collect(),
    })
}

pub(super) fn set_sheet_fasteners(
    host: &dyn Host,
    call: &Request,
    args: SheetFastenersArgs,
) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(stale(args.expected_revision, revision));
    }
    let feature = id(&args.feature)?;
    let patterns = args
        .patterns
        .iter()
        .map(|p| pattern(p, &document, &evaluated))
        .collect::<Result<Vec<_>, _>>()?;
    let call_owned = call.clone();
    let (prepared, out) = with(host, move |target| {
        revision_guard(target, revision)?;
        let p = model::prepare_sheet_fasteners(
            target.session(),
            revision,
            call_owned.actor.clone(),
            feature,
            &patterns,
        )?;
        let out = json!({
            "feature": feature,
            "hardware": p.hardware.iter().map(|(pattern, item, o)| json!({
                "pattern": pattern, "item": item.label(), "part": o.body, "feature": o.feature,
            })).collect::<Vec<_>>(),
        });
        Ok::<_, ToolError>((p.model.prepared, out))
    })??;
    finish(host, call, prepared, revision, out)
}

pub(super) fn sheet_hardware(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let records = model.sheet_hardware(&document)?;
    Ok(json!({
        "revision": revision,
        "scope": "Purchased hardware Parts attached to sheet parts. Quantities are measured from the hardware solids. Built-in dimensions are nominal values from the named standards; insert dimensions come from the source given. No torque, strength or installation data is provided.",
        "hardware": records.iter().map(|r| json!({
            "part": r.part, "feature": r.feature, "sheet_part": r.sheet_part,
            "pattern": r.pattern, "item": r.item.label(), "designation": r.designation,
            "source": r.source, "quantity": r.quantity, "hole_mm": r.hole_mm,
            "findings": r.findings,
        })).collect::<Vec<_>>(),
    }))
}
