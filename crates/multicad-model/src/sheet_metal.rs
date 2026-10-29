//! Native sheet metal: folded solids and developed flat patterns built from
//! the same sheet feature, sheet records measured from the evaluated solids,
//! and the analysis that converts a constant-thickness solid into a sheet
//! part.
use crate::{Model, ModelError, error};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    document::{Document, DocumentError, EntityData, EntityId},
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
    materials,
    model::{
        self, BodyOutput, ModelPrepared, ResolvedBase, SheetBase, SheetDefinition, SheetFrame,
        SheetRequest, ValueSource,
    },
    parameters::EvaluatedParameters,
    references::Origin,
    sketch_regions::RegionIntent,
};
use multicad_kernel::{CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid};

const TOLERANCE_MM: f64 = 1e-3;

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    scale(a, 1. / dot(a, a).sqrt())
}

/// The evaluated base of a sheet feature: the base face lies in `frame`, and
/// the material runs `thickness_mm` along `thickness_direction`.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetLayout {
    pub frame: PlaneFrame,
    pub thickness_direction: [f64; 3],
    pub thickness_mm: f64,
    pub base: CurvedProfile,
}

fn curved(edge: &PlanarEdge) -> CurvedEdge2d {
    match *edge {
        PlanarEdge::Line { a, b } => CurvedEdge2d::Line { a, b },
        PlanarEdge::Arc {
            a,
            b,
            center,
            sweep_rad,
        } if (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-9 => {
            let _ = b;
            CurvedEdge2d::Circle {
                center,
                radius_mm: (a[0] - center[0]).hypot(a[1] - center[1]),
            }
        }
        PlanarEdge::Arc {
            a,
            b,
            center,
            sweep_rad,
        } => CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise: sweep_rad < 0.,
        },
    }
}
fn plane(frame: SheetFrame) -> PlaneFrame {
    PlaneFrame {
        origin: frame.origin,
        u: frame.u,
        v: frame.v,
    }
}
fn roles(id: EntityId, count: usize) -> Vec<Origin> {
    (0..count)
        .map(|i| Origin {
            source: id,
            role: format!("sheet.base.edge.{i}"),
        })
        .collect()
}

/// Build a sheet's folded solid and return it with its layout. `profile`
/// supplies solved sketch regions for profile bases.
pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    profile: impl FnOnce(EntityId, &RegionIntent) -> Result<(CurvedProfile, Vec<Origin>), ModelError>,
) -> Result<(Solid, SheetLayout), ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let source = model::read_sheet(document, id).map_err(e)?;
    let sheet = model::resolve_sheet(document, parameters, &source).map_err(e)?;
    let (base, origins) = match &sheet.base {
        ResolvedBase::Rectangle {
            frame,
            length_mm,
            width_mm,
        } => {
            let (l, w) = (*length_mm, *width_mm);
            let corners = [[0., 0.], [l, 0.], [l, w], [0., w]];
            let base = CurvedProfile {
                frame: plane(*frame),
                outer: (0..4)
                    .map(|i| CurvedEdge2d::Line {
                        a: corners[i],
                        b: corners[(i + 1) % 4],
                    })
                    .collect(),
                holes: vec![],
            };
            (base, roles(id, 4))
        }
        ResolvedBase::Profile { profile: source } => profile(source.sketch, &source.region)?,
        ResolvedBase::Contour { frame, loops } => {
            let outer: Vec<_> = loops[0].edges.iter().map(curved).collect();
            let holes: Vec<Vec<_>> = loops[1..]
                .iter()
                .map(|l| l.edges.iter().map(curved).collect())
                .collect();
            let count = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
            (
                CurvedProfile {
                    frame: plane(*frame),
                    outer,
                    holes,
                },
                roles(id, count),
            )
        }
    };
    let n = base.frame.normal();
    let direction = if sheet.reverse { scale(n, -1.) } else { n };
    let solid = Solid::extrude_curved_profile_vector_named(
        &base,
        scale(direction, sheet.thickness_mm),
        id,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(id), e))?;
    Ok((
        solid,
        SheetLayout {
            frame: base.frame,
            thickness_direction: direction,
            thickness_mm: sheet.thickness_mm,
            base,
        },
    ))
}

/// The developed blank as a solid: the base profile and every developed
/// region, extruded by the thickness on the same side as the folded part.
fn flat_solid(layout: &SheetLayout, policy: GeometryPolicy) -> Result<Solid, String> {
    Solid::extrude_curved_profile_vector(
        &layout.base,
        scale(layout.thickness_direction, layout.thickness_mm),
        policy,
    )
    .map_err(|e| e.to_string())
}
/// Exact developed contours on the base plane, in base-frame coordinates.
fn flat_contours(layout: &SheetLayout, policy: GeometryPolicy) -> Result<PlanarContours, String> {
    let solid = flat_solid(layout, policy)?;
    let outward = scale(layout.thickness_direction, -1.);
    let faces = solid.faces().map_err(|e| e.to_string())?;
    let face = faces
        .iter()
        .find(|f| {
            f.plane.is_some_and(|(origin, normal)| {
                dot(normal, outward) > 1. - 1e-9
                    && dot(
                        [
                            origin[0] - layout.frame.origin[0],
                            origin[1] - layout.frame.origin[1],
                            origin[2] - layout.frame.origin[2],
                        ],
                        outward,
                    )
                    .abs()
                        < TOLERANCE_MM
            })
        })
        .ok_or("The developed blank has no face on the base plane")?;
    let mut contours = solid
        .planar_face_contours(face.index, layout.frame)
        .map_err(|e| e.to_string())?;
    for l in &mut contours.loops {
        l.normalize_winding();
    }
    contours.validate(policy.linear_tolerance_mm)?;
    Ok(contours)
}

/// One sheet part as evaluated: bend inputs with their sources, measured
/// thickness and the developed flat pattern of the same feature.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetRecord {
    pub part: EntityId,
    pub feature: EntityId,
    pub name: String,
    pub material: Option<EntityId>,
    pub stock: EntityId,
    pub stock_name: String,
    pub thickness_mm: f64,
    pub inside_radius_mm: f64,
    pub radius_source: ValueSource,
    pub k_factor: f64,
    pub k_factor_source: ValueSource,
    /// "rectangle", "profile" or "contour".
    pub base: &'static str,
    pub converted_from: Option<EntityId>,
    /// Distance between the outermost faces parallel to the base face.
    pub measured_thickness_mm: f64,
    pub volume_mm3: f64,
    pub solid_count: u32,
    /// Developed blank in base-frame millimeters: outer loop then holes.
    pub flat: PlanarContours,
    pub flat_size_mm: [f64; 2],
    pub flat_area_mm2: f64,
    /// Review findings: thickness or stock assignment that disagree.
    pub issues: Vec<String>,
}

impl Model {
    fn sheet_parts_solid(
        &self,
        feature: EntityId,
        part: EntityId,
    ) -> Option<(&SheetLayout, &Solid)> {
        let layout = self.snapshot().sheet_layouts.get(&feature)?;
        let head = self.snapshot().bodies.get(&part)?;
        Some((layout, self.solid(*head)?))
    }
    /// The developed flat pattern of one evaluated sheet feature.
    pub fn sheet_flat_pattern(&self, feature: EntityId) -> Result<PlanarContours, ModelError> {
        let layout = self
            .snapshot()
            .sheet_layouts
            .get(&feature)
            .ok_or_else(|| error(Some(feature), "The sheet part has not been evaluated"))?;
        flat_contours(layout, self.policy).map_err(|e| error(Some(feature), e))
    }
    /// Records for every evaluated sheet Part, in document order. Parts
    /// consumed by a Boolean are not listed.
    pub fn sheet_metal(&self, document: &Document) -> Result<Vec<SheetRecord>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut out = vec![];
        for entity in model::sheet_parts(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let source = model::read_sheet(document, id).map_err(e)?;
            let part = source.output.body;
            let Some((layout, solid)) = self.sheet_parts_solid(id, part) else {
                continue;
            };
            let sheet = model::resolve_sheet(document, &parameters, &source).map_err(e)?;
            let measured = measured_thickness(solid, layout).map_err(|m| error(Some(id), m))?;
            let flat = flat_contours(layout, self.policy).map_err(|m| error(Some(id), m))?;
            let [min, max] = flat
                .bounds_mm()
                .ok_or_else(|| error(Some(id), "The flat pattern has no bounds"))?;
            let mut issues = vec![];
            if (measured - sheet.thickness_mm).abs() > TOLERANCE_MM {
                issues.push(format!(
                    "Measured thickness {measured:.3} mm differs from the {} mm stock",
                    sheet.thickness_mm
                ));
            }
            if let Some((_, properties)) = materials::part_properties(document, part).map_err(e)?
                && properties.stock.as_ref().map(|s| s.id) != Some(sheet.stock)
            {
                issues.push("The Part's stock assignment differs from its sheet stock".into());
            }
            let Some(EntityData::Part { material, .. }) = document.get(part).map(|e| &e.data)
            else {
                continue;
            };
            out.push(SheetRecord {
                part,
                feature: id,
                name: document
                    .get(part)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                material: *material,
                stock: sheet.stock,
                stock_name: document
                    .get(sheet.stock)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                thickness_mm: sheet.thickness_mm,
                inside_radius_mm: sheet.inside_radius_mm,
                radius_source: sheet.radius_source,
                k_factor: sheet.k_factor,
                k_factor_source: sheet.k_factor_source,
                base: match source.base {
                    SheetBase::Rectangle { .. } => "rectangle",
                    SheetBase::Profile { .. } => "profile",
                    SheetBase::Contour { .. } => "contour",
                },
                converted_from: match source.base {
                    SheetBase::Contour { converted_from, .. } => converted_from,
                    _ => None,
                },
                measured_thickness_mm: measured,
                volume_mm3: solid.volume_mm3().map_err(|e| error(Some(id), e))?,
                solid_count: solid.solid_count().map_err(|e| error(Some(id), e))?,
                flat_size_mm: [max[0] - min[0], max[1] - min[1]],
                flat_area_mm2: flat.area_mm2(),
                flat,
                issues,
            });
        }
        Ok(out)
    }
}
/// The spread of the planar faces parallel to the base face, measured along
/// the thickness direction.
fn measured_thickness(solid: &Solid, layout: &SheetLayout) -> Result<f64, String> {
    let n = layout.thickness_direction;
    let mut offsets = vec![];
    for f in solid.faces().map_err(|e| e.to_string())? {
        if let Some((origin, normal)) = f.plane
            && dot(normal, n).abs() > 1. - 1e-9
        {
            offsets.push(dot(
                [
                    origin[0] - layout.frame.origin[0],
                    origin[1] - layout.frame.origin[1],
                    origin[2] - layout.frame.origin[2],
                ],
                n,
            ));
        }
    }
    let min = offsets.iter().copied().fold(f64::INFINITY, f64::min);
    let max = offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if offsets.len() < 2 {
        return Err("The sheet solid has no faces parallel to its base".into());
    }
    Ok(max - min)
}

/// What conversion found in a solid: the stationary face and its exact
/// loops when the solid is a supported constant-thickness plate, and
/// actionable findings when it is not.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionAnalysis {
    pub part: EntityId,
    /// Measured plate thickness, when two opposite base faces were found.
    pub thickness_mm: Option<f64>,
    /// The stationary face as a sheet frame (thickness along u × v) and its
    /// exact loops, outer first.
    pub stationary: Option<(SheetFrame, Vec<PlanarLoop>)>,
    pub planar_faces: usize,
    /// Centers of curved faces that are not side walls through the thickness:
    /// bend or rounded regions that conversion does not develop yet.
    pub bend_candidates: Vec<[f64; 3]>,
    pub issues: Vec<String>,
}
impl ConversionAnalysis {
    pub fn convertible(&self) -> bool {
        self.issues.is_empty() && self.stationary.is_some()
    }
    /// The sheet request for a convertible solid: its stationary face as a
    /// captured contour base of `stock`. Refused with the findings otherwise.
    pub fn request(&self, name: &str, stock: EntityId) -> Result<SheetRequest, CommandError> {
        let Some((frame, loops)) = self.stationary.clone().filter(|_| self.convertible()) else {
            return Err(refused(format!(
                "The Part cannot be converted: {}",
                self.issues.join("; ")
            )));
        };
        Ok(SheetRequest::new(
            name,
            SheetDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                stock,
                base: SheetBase::Contour {
                    frame,
                    loops,
                    converted_from: Some(self.part),
                },
                reverse: false,
                inside_radius: None,
                k_factor: None,
            },
        ))
    }
}
/// A frame on a face with outward normal `outward`: the thickness (u × v)
/// runs into the material, and u is the world axis closest to the plane.
fn face_frame(origin: [f64; 3], outward: [f64; 3]) -> SheetFrame {
    let w = scale(outward, -1.);
    let axis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
        .into_iter()
        .min_by(|a, b| dot(*a, w).abs().total_cmp(&dot(*b, w).abs()))
        .unwrap_or([1., 0., 0.]);
    let u = unit([
        axis[0] - w[0] * dot(axis, w),
        axis[1] - w[1] * dot(axis, w),
        axis[2] - w[2] * dot(axis, w),
    ]);
    let v = unit(cross(w, u));
    // Snap axis components that are exactly representable.
    let snap = |a: [f64; 3]| a.map(|c| if c.abs() < 1e-15 { 0. } else { c });
    SheetFrame {
        origin,
        u: snap(u),
        v: snap(v),
    }
}

impl Model {
    /// Analyze a Part's current solid for conversion into a sheet part. A
    /// supported solid is one flat plate: two opposite planar base faces of
    /// equal area at a constant distance, with every other face planar and
    /// perpendicular to them. `stock_thickness_mm` adds a stock check.
    pub fn analyze_sheet_conversion(
        &self,
        part: EntityId,
        stock_thickness_mm: Option<f64>,
    ) -> Result<ConversionAnalysis, ModelError> {
        let head = self
            .snapshot()
            .bodies
            .get(&part)
            .ok_or_else(|| error(Some(part), "Choose an evaluated Part to convert"))?;
        let solid = self
            .solid(*head)
            .ok_or_else(|| error(Some(part), "The Part has no current solid"))?;
        let g = |e: multicad_kernel::GeometryError| error(Some(part), e);
        let faces = solid.faces().map_err(g)?;
        let mut analysis = ConversionAnalysis {
            part,
            thickness_mm: None,
            stationary: None,
            planar_faces: faces.iter().filter(|f| f.plane.is_some()).count(),
            bend_candidates: vec![],
            issues: vec![],
        };
        if solid.solid_count().map_err(g)? != 1 {
            analysis
                .issues
                .push("The Part has more than one solid; convert one connected plate".into());
            return Ok(analysis);
        }
        let Some(base) = faces
            .iter()
            .filter(|f| f.plane.is_some())
            .max_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2))
        else {
            analysis.issues.push("The solid has no planar face".into());
            return Ok(analysis);
        };
        let (origin, normal) = base.plane.expect("planar");
        let offset = |p: [f64; 3]| {
            dot(
                [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]],
                normal,
            )
        };
        let mut top = vec![];
        let mut bottom = vec![];
        for f in &faces {
            let Some((o, n)) = f.plane else { continue };
            let c = dot(n, normal);
            if c > 1. - 1e-9 {
                top.push((f, offset(o)));
            } else if c < -1. + 1e-9 {
                bottom.push((f, offset(o)));
            } else if c.abs() > 1e-9 {
                analysis.issues.push(format!(
                    "A face at ({:.3}, {:.3}, {:.3}) is inclined to the base: chamfers and \
                     drafts do not have constant thickness",
                    f.center_mm[0], f.center_mm[1], f.center_mm[2]
                ));
            }
        }
        let thickness = bottom
            .iter()
            .map(|(_, d)| -d)
            .fold(f64::NEG_INFINITY, f64::max);
        if !thickness.is_finite() || thickness <= TOLERANCE_MM {
            analysis
                .issues
                .push("No face opposite the largest planar face was found".into());
            analysis.bend_candidates = faces
                .iter()
                .filter(|f| f.plane.is_none())
                .map(|f| f.center_mm)
                .collect();
            if !analysis.bend_candidates.is_empty() {
                analysis.issues.push(format!(
                    "{} curved face(s) may be bend regions: bent solids are not converted yet; \
                     rebuild the bends as sheet flanges",
                    analysis.bend_candidates.len()
                ));
            }
            return Ok(analysis);
        }
        analysis.thickness_mm = Some(thickness);
        // Curved side walls of a right prism (holes, rounded outline corners)
        // have their area centroid at mid-thickness; any other curved face is
        // a bend or rounded region.
        analysis.bend_candidates = faces
            .iter()
            .filter(|f| {
                f.plane.is_none() && (offset(f.center_mm) + thickness / 2.).abs() > TOLERANCE_MM
            })
            .map(|f| f.center_mm)
            .collect();
        if !analysis.bend_candidates.is_empty() {
            analysis.issues.push(format!(
                "{} curved face(s) are bend or rounded regions: bent solids are not converted \
                 yet; rebuild the bends as sheet flanges or remove the rounds",
                analysis.bend_candidates.len()
            ));
        }
        for (f, d) in top.iter().chain(&bottom) {
            if d.abs() > TOLERANCE_MM && (d + thickness).abs() > TOLERANCE_MM {
                analysis.issues.push(format!(
                    "A face {:.3} mm below the base at ({:.3}, {:.3}, {:.3}) gives varying \
                     thickness (pocket or step); sheet parts have constant thickness",
                    -d, f.center_mm[0], f.center_mm[1], f.center_mm[2]
                ));
            }
        }
        if top
            .iter()
            .any(|(_, d)| (d + thickness).abs() <= TOLERANCE_MM)
            || bottom.iter().any(|(_, d)| d.abs() <= TOLERANCE_MM)
        {
            analysis
                .issues
                .push("Base faces point in both directions on one plane".into());
        }
        let top_faces: Vec<_> = top
            .iter()
            .filter(|(_, d)| d.abs() <= TOLERANCE_MM)
            .collect();
        let bottom_faces: Vec<_> = bottom
            .iter()
            .filter(|(_, d)| (d + thickness).abs() <= TOLERANCE_MM)
            .collect();
        if top_faces.len() != 1 || bottom_faces.len() != 1 {
            analysis.issues.push(
                "The plate needs exactly one face on each side; split or stepped plates are \
                 not converted"
                    .into(),
            );
            return Ok(analysis);
        }
        let area = top_faces[0].0.area_mm2;
        let volume = solid.volume_mm3().map_err(g)?;
        if (bottom_faces[0].0.area_mm2 - area).abs() > 1e-6 * area.max(1.)
            || (volume - area * thickness).abs() > 1e-6 * volume.max(1.)
        {
            analysis.issues.push(
                "The faces on both sides differ: the solid is not a constant-thickness prism"
                    .into(),
            );
        }
        if let Some(stock) = stock_thickness_mm
            && (stock - thickness).abs() > TOLERANCE_MM
        {
            analysis.issues.push(format!(
                "Measured thickness {thickness:.3} mm differs from the {stock} mm sheet stock"
            ));
        }
        // The stationary face is the side facing away from +Z (then +Y, +X).
        let (face, outward) = {
            let key = |n: [f64; 3]| (n[2], n[1], n[0]);
            let (a, b) = (top_faces[0].0, bottom_faces[0].0);
            let na = a.plane.expect("planar").1;
            let nb = b.plane.expect("planar").1;
            if key(na) <= key(nb) { (a, na) } else { (b, nb) }
        };
        let o = face.plane.expect("planar").0;
        let frame = face_frame(o, outward);
        let mut contours = solid
            .planar_face_contours(
                face.index,
                PlaneFrame {
                    origin: frame.origin,
                    u: frame.u,
                    v: frame.v,
                },
            )
            .map_err(g)?;
        for l in &mut contours.loops {
            l.normalize_winding();
        }
        contours.loops.sort_by_key(|l| !l.outer);
        if contours.loops.len() > model::MAX_CONTOUR_LOOPS {
            analysis
                .issues
                .push("The base face has too many openings to capture".into());
        }
        analysis.stationary = Some((frame, contours.loops));
        Ok(analysis)
    }
}

fn refused(message: impl Into<String>) -> CommandError {
    CommandError {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
        involved: vec![],
        parameter: None,
    }
}
/// Create a sheet Part from a converted solid: its stationary face becomes a
/// captured contour base of the given sheet stock. The source Part stays as
/// it is. Refused with the analysis findings when the solid is unsupported.
pub fn prepare_conversion(
    model: &Model,
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    part: EntityId,
    stock: EntityId,
) -> Result<(ModelPrepared, ConversionAnalysis), CommandError> {
    let spec = model::read_sheet_stock(session.document(), stock)?;
    let thickness = model::sheet_stock_values(&spec).map(|v| v.0);
    let analysis = model
        .analyze_sheet_conversion(part, thickness)
        .map_err(|e| refused(e.message))?;
    let request = analysis.request(name, stock)?;
    let prepared = model::prepare_sheet(session, expected_revision, actor, &request)?;
    Ok((prepared, analysis))
}
