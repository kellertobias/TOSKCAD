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
        self, BodyOutput, PanelPlane, ProfileSource, SheetBase, SheetDefinition, SheetRequest,
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
    };
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
            "issues": r.issues,
        })).collect::<Vec<_>>(),
    }))
}
