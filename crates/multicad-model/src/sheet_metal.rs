//! Native sheet metal: folded solids and developed flat patterns built from
//! the same sheet feature, sheet records measured from the evaluated solids,
//! and the analysis that converts a constant-thickness solid into a sheet
//! part.
use crate::sheet_forming::{
    PlacedFlange, SheetSpace, developed_rect, flat_cutout, fuse, interfere, polygon_edges,
};
use crate::{Model, ModelError, error};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    document::{Document, DocumentError, EntityData, EntityId},
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
    materials,
    model::{
        self, BodyOutput, CutoutFace, ModelPrepared, ResolvedBase, ResolvedCutout, SheetBase,
        SheetDefinition, SheetFaceFrame, SheetFrame, SheetRequest, StripSegment, ValueSource,
    },
    parameters::EvaluatedParameters,
    references::Origin,
    sketch_regions::RegionIntent,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
};

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
    pub k_factor: f64,
    pub base: CurvedProfile,
    /// Flanges on their base edges, in base-frame coordinates.
    pub flanges: Vec<PlacedFlange>,
    pub cutouts: Vec<ResolvedCutout>,
    pub patterns: Vec<model::ResolvedPattern>,
}
impl SheetLayout {
    pub(crate) fn space(&self) -> SheetSpace {
        self.space_inner()
    }
    fn space_inner(&self) -> SheetSpace {
        SheetSpace {
            frame: self.frame,
            thickness_direction: self.thickness_direction,
            thickness_mm: self.thickness_mm,
        }
    }
}

/// One bend of a flange as developed: its direction and angle, inside
/// radius, bend allowance, and the bend zone and center line in the flat
/// pattern (base-frame millimeters).
#[derive(Clone, Debug, PartialEq)]
pub struct BendRecord {
    /// Flange index (zero-based) and bend index within the flange.
    pub flange: usize,
    pub bend: usize,
    pub edge: usize,
    /// "up" toward the material side, "down" away from it.
    pub direction: &'static str,
    pub angle_deg: f64,
    pub inside_radius_mm: f64,
    pub allowance_mm: f64,
    /// Developed distance from the base edge to the start and end of the
    /// bend zone.
    pub zone_mm: [f64; 2],
    /// The bend center line across the flange width in the flat pattern.
    pub line: [[f64; 2]; 2],
    pub length_mm: f64,
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
    let m = |m: String| error(Some(id), m);
    let g = |e: multicad_kernel::GeometryError| error(Some(id), e);
    let source = model::read_sheet(document, id).map_err(e)?;
    let sheet = model::resolve_sheet(document, parameters, &source).map_err(e)?;
    let (base, origins) = match &sheet.base {
        ResolvedBase::Rectangle {
            frame,
            length_mm,
            width_mm,
        } => {
            let (l, w) = (*length_mm, *width_mm);
            let base = CurvedProfile {
                frame: plane(*frame),
                outer: polygon_edges(&[[0., 0.], [l, 0.], [l, w], [0., w]]),
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
    let t = sheet.thickness_mm;
    let flanges = place_flanges(&base, &sheet.flanges).map_err(m)?;
    let layout = SheetLayout {
        frame: base.frame,
        thickness_direction: direction,
        thickness_mm: t,
        k_factor: sheet.k_factor,
        base: base.clone(),
        flanges,
        cutouts: sheet.cutouts.clone(),
        patterns: sheet.patterns.clone(),
    };
    let space = layout.space();
    let mut solid = Solid::extrude_curved_profile_vector_named(
        &base,
        scale(direction, t),
        id,
        &origins,
        policy,
    )
    .map_err(g)?;
    let mut flange_solids = vec![];
    for f in &layout.flanges {
        flange_solids.push(space.flange_solid(f, id, policy).map_err(m)?);
    }
    for i in 0..flange_solids.len() {
        for j in i + 1..flange_solids.len() {
            if let Some(how) = interfere(&flange_solids[i], &flange_solids[j], policy).map_err(m)? {
                return Err(m(format!(
                    "Flanges {} and {} {how} at a corner or seam; increase their insets",
                    layout.flanges[i].flange.index + 1,
                    layout.flanges[j].flange.index + 1
                )));
            }
        }
    }
    let base_volume = solid.volume_mm3().map_err(g)?;
    let mut flange_volume = 0.;
    for s in &flange_solids {
        flange_volume += s.volume_mm3().map_err(g)?;
    }
    solid = fuse(solid, &flange_solids, policy).map_err(m)?;
    let fused = solid.volume_mm3().map_err(g)?;
    if (fused - base_volume - flange_volume).abs() > 1e-6 * fused.max(1.) {
        return Err(m(
            "A flange runs into the base sheet; check its edge, direction and angle".into(),
        ));
    }
    for f in &layout.flanges {
        for relief in space.reliefs(f) {
            let tool = space.base_prism(&relief, policy).map_err(m)?;
            solid = subtract(solid, &tool, policy)
                .map_err(|r| m(format!("Flange {} relief: {r}", f.flange.index + 1)))?;
        }
    }
    for cutout in &layout.cutouts {
        let tool = space
            .cutout_solid(cutout, &layout.flanges, policy)
            .map_err(m)?;
        let before = solid.volume_mm3().map_err(g)?;
        let which = cutout.source.id();
        solid = subtract(solid, &tool, policy).map_err(|r| m(format!("{which}: {r}")))?;
        let removed = before - solid.volume_mm3().map_err(g)?;
        let expected = cutout.shape.area_mm2() * t;
        if (removed - expected).abs() > 1e-6 * expected.max(1.) {
            return Err(m(format!(
                "{which} is not fully inside its face: it removes {removed:.3} mm³ instead of {expected:.3} mm³"
            )));
        }
    }
    if solid.solid_count().map_err(g)? != 1 {
        return Err(m("The sheet part falls apart into separate pieces".into()));
    }
    Ok((solid, layout))
}
fn subtract(solid: Solid, tool: &Solid, policy: GeometryPolicy) -> Result<Solid, String> {
    solid
        .boolean(tool, BooleanOperation::Subtract, policy)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "it removes all material".into())
}
/// Locate each flange on its straight base edge.
fn place_flanges(
    base: &CurvedProfile,
    flanges: &[model::ResolvedFlange],
) -> Result<Vec<PlacedFlange>, String> {
    // Winding from the edge endpoints and arc midpoints; it decides which
    // side of an edge is outward.
    let mut points = vec![];
    for e in &base.outer {
        match e {
            CurvedEdge2d::Line { a, .. } => points.push(*a),
            CurvedEdge2d::Arc {
                center,
                a,
                b,
                clockwise,
            } => {
                points.push(*a);
                let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                let end = (b[1] - center[1]).atan2(b[0] - center[0]);
                let sweep = if *clockwise {
                    -(start - end).rem_euclid(std::f64::consts::TAU)
                } else {
                    (end - start).rem_euclid(std::f64::consts::TAU)
                };
                let r = (a[0] - center[0]).hypot(a[1] - center[1]);
                let mid = start + sweep / 2.;
                points.push([center[0] + r * mid.cos(), center[1] + r * mid.sin()]);
            }
            _ => {}
        }
    }
    let area: f64 = (0..points.len())
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % points.len()]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum();
    let ccw = area >= 0.;
    let mut out = vec![];
    for f in flanges {
        let which = format!("Flange {}", f.index + 1);
        let Some(edge) = base.outer.get(f.edge) else {
            return Err(format!(
                "{which}: the base has no edge {} (it has {})",
                f.edge,
                base.outer.len()
            ));
        };
        let CurvedEdge2d::Line { a, b } = edge else {
            return Err(format!(
                "{which}: base edge {} is curved; flanges need straight edges",
                f.edge
            ));
        };
        let d = [b[0] - a[0], b[1] - a[1]];
        let length = d[0].hypot(d[1]);
        let along = [d[0] / length, d[1] / length];
        let outward = if ccw {
            [along[1], -along[0]]
        } else {
            [-along[1], along[0]]
        };
        let width = length - f.start_inset_mm - f.end_inset_mm;
        if width <= TOLERANCE_MM {
            return Err(format!(
                "{which}: its insets leave no flange on the {length:.3} mm edge"
            ));
        }
        if let Some([w, _]) = f.relief_mm {
            for inset in [f.start_inset_mm, f.end_inset_mm] {
                if inset > 1e-9 && w > inset + 1e-9 {
                    return Err(format!(
                        "{which}: its {w} mm relief is wider than its {inset} mm inset"
                    ));
                }
            }
        }
        out.push(PlacedFlange {
            flange: f.clone(),
            start: [
                a[0] + along[0] * f.start_inset_mm,
                a[1] + along[1] * f.start_inset_mm,
            ],
            along,
            outward,
            width,
            edge_length: length,
        });
    }
    Ok(out)
}

/// The developed blank as a solid: one profile made of the base outline
/// with every developed flange spliced into its edge, relief notches and
/// cutouts as openings, extruded by the thickness on the same side as the
/// folded part. Returns it with the expected area.
fn flat_solid(layout: &SheetLayout, policy: GeometryPolicy) -> Result<(Solid, f64), String> {
    let space = layout.space();
    let (t, k) = (layout.thickness_mm, layout.k_factor);
    let s = |e: multicad_kernel::GeometryError| e.to_string();
    let base = Solid::extrude_curved_profile_vector(
        &layout.base,
        scale(layout.thickness_direction, t),
        policy,
    )
    .map_err(s)?;
    let mut area = base.volume_mm3().map_err(s)? / t;
    // Developed regions must not overlap each other or the base.
    let mut regions = vec![];
    for f in &layout.flanges {
        let developed = f.flange.developed_mm(t, k);
        regions.push(space.base_prism(&developed_rect(f, developed), policy)?);
        area += developed * f.width;
    }
    let fused = fuse(base, &regions, policy)?.volume_mm3().map_err(s)? / t;
    if (fused - area).abs() > 1e-6 * area.max(1.) {
        return Err(
            "The developed flanges overlap each other or the base in the flat pattern".into(),
        );
    }
    let mut outer = vec![];
    for (i, edge) in layout.base.outer.iter().enumerate() {
        let Some(f) = layout.flanges.iter().find(|f| f.flange.edge == i) else {
            outer.push(edge.clone());
            continue;
        };
        let CurvedEdge2d::Line { a, b } = *edge else {
            return Err("A flange edge is not straight".into());
        };
        let developed = f.flange.developed_mm(t, k);
        let at = |along: f64, out: f64| {
            [
                f.start[0] + f.along[0] * along + f.outward[0] * out,
                f.start[1] + f.along[1] * along + f.outward[1] * out,
            ]
        };
        let mut points = vec![a];
        let relief = f.flange.relief_mm;
        if f.flange.start_inset_mm > 1e-9 {
            if let Some([w, depth]) = relief {
                if w > f.flange.start_inset_mm + 1e-9 {
                    return Err(format!(
                        "Flange {} relief is wider than its start inset",
                        f.flange.index + 1
                    ));
                }
                points.extend([at(-w, 0.), at(-w, -depth), at(0., -depth)]);
                area -= w * depth;
            }
            points.push(at(0., 0.));
        }
        points.extend([at(0., developed), at(f.width, developed)]);
        if f.flange.end_inset_mm > 1e-9 {
            points.push(at(f.width, 0.));
            if let Some([w, depth]) = relief {
                if w > f.flange.end_inset_mm + 1e-9 {
                    return Err(format!(
                        "Flange {} relief is wider than its end inset",
                        f.flange.index + 1
                    ));
                }
                points.extend([
                    at(f.width, -depth),
                    at(f.width + w, -depth),
                    at(f.width + w, 0.),
                ]);
                area -= w * depth;
            }
        }
        points.push(b);
        points.dedup_by(|p, q| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-9);
        outer.extend(
            points
                .windows(2)
                .map(|w| CurvedEdge2d::Line { a: w[0], b: w[1] }),
        );
    }
    let mut holes = layout.base.holes.clone();
    for c in &layout.cutouts {
        holes.push(flat_cutout(c, &layout.flanges, t, k).ok_or("A cutout names a missing flange")?);
        area -= c.shape.area_mm2();
    }
    let solid = Solid::extrude_developed_sheet_profile(
        &CurvedProfile {
            frame: layout.frame,
            outer,
            holes,
        },
        scale(layout.thickness_direction, t),
        policy,
    )
    .map_err(|e| format!("The flat pattern cannot be developed: {e}"))?;
    if solid.solid_count().map_err(s)? != 1 {
        return Err("The flat pattern falls apart into separate pieces".into());
    }
    Ok((solid, area))
}
/// Exact developed contours on the base plane, in base-frame coordinates.
pub(crate) fn flat_contours(
    layout: &SheetLayout,
    policy: GeometryPolicy,
) -> Result<PlanarContours, String> {
    let (solid, area) = flat_solid(layout, policy)?;
    let outward = scale(layout.thickness_direction, -1.);
    let faces = solid.faces().map_err(|e| e.to_string())?;
    let on_base: Vec<_> = faces
        .iter()
        .filter(|f| {
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
        .collect();
    let [face] = on_base.as_slice() else {
        return Err(format!(
            "The developed blank has {} faces on the base plane instead of one",
            on_base.len()
        ));
    };
    let mut contours = solid
        .planar_face_contours(face.index, layout.frame)
        .map_err(|e| e.to_string())?;
    for l in &mut contours.loops {
        l.normalize_winding();
    }
    contours.loops.sort_by_key(|l| !l.outer);
    contours.validate(policy.linear_tolerance_mm)?;
    if (contours.area_mm2() - area).abs() > 1e-6 * area.max(1.) {
        return Err(format!(
            "The flat pattern area {:.3} mm² differs from the developed {area:.3} mm²",
            contours.area_mm2()
        ));
    }
    Ok(contours)
}
/// Bend records of every flange, in flange order.
pub(crate) fn bend_records(layout: &SheetLayout) -> Vec<BendRecord> {
    let (t, k) = (layout.thickness_mm, layout.k_factor);
    let mut out = vec![];
    for f in &layout.flanges {
        let mut offset = 0.;
        let mut bend = 0;
        for segment in &f.flange.segments {
            match *segment {
                StripSegment::Straight { length_mm } => offset += length_mm,
                StripSegment::Bend {
                    angle_rad,
                    radius_mm,
                } => {
                    let allowance = angle_rad.abs() * (radius_mm + k * t);
                    let mid = offset + allowance / 2.;
                    let p = |a: f64| {
                        [
                            f.start[0] + f.along[0] * a + f.outward[0] * mid,
                            f.start[1] + f.along[1] * a + f.outward[1] * mid,
                        ]
                    };
                    out.push(BendRecord {
                        flange: f.flange.index,
                        bend,
                        edge: f.flange.edge,
                        direction: if angle_rad > 0. { "up" } else { "down" },
                        angle_deg: angle_rad.abs().to_degrees(),
                        inside_radius_mm: radius_mm,
                        allowance_mm: allowance,
                        zone_mm: [offset, offset + allowance],
                        line: [p(0.), p(f.width)],
                        length_mm: f.width,
                    });
                    offset += allowance;
                    bend += 1;
                }
            }
        }
    }
    out
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
    /// Every bend of every flange, with its allowance and flat bend line.
    pub bends: Vec<BendRecord>,
    /// Review findings: thickness or stock assignment that disagree.
    pub issues: Vec<String>,
}

impl Model {
    /// The final producer of a sheet or hardware Part: a physical Part's
    /// head, or the definition head of a Part that an Assembly places
    /// (its manufacturing source). Consumed Parts have none.
    pub(crate) fn sheet_head(&self, document: &Document, part: EntityId) -> Option<EntityId> {
        let snapshot = self.snapshot();
        if let Some(head) = snapshot.bodies.get(&part) {
            return Some(*head);
        }
        if snapshot.consumed_bodies.contains(&part)
            || !model::assembly_definition_parts(document)
                .ok()?
                .contains(&part)
        {
            return None;
        }
        snapshot.body_sources.get(&part).copied()
    }
    fn sheet_parts_solid(
        &self,
        document: &Document,
        feature: EntityId,
        part: EntityId,
    ) -> Option<(&SheetLayout, &Solid)> {
        let layout = self.snapshot().sheet_layouts.get(&feature)?;
        let head = self.sheet_head(document, part)?;
        Some((layout, self.solid(head)?))
    }
    fn final_sheet_layouts<'a>(
        &'a self,
        document: &'a Document,
    ) -> impl Iterator<Item = (EntityId, EntityId, &'a SheetLayout)> + 'a {
        let snapshot = self.snapshot();
        let full = snapshot.scope == crate::EvaluationScope::Full
            && snapshot.stamp.document == document.id();
        full.then_some(&snapshot.body_sources)
            .into_iter()
            .flatten()
            .filter_map(move |(&part, &feature)| {
                let layout = snapshot.sheet_layouts.get(&feature)?;
                (self.sheet_head(document, part) == Some(feature))
                    .then_some((part, feature, layout))
            })
    }

    /// Bend schedules for current sheet Parts from the evaluated folded layout.
    /// An unbent sheet has an empty schedule. Scoped evaluations omit schedules.
    pub fn sheet_bend_schedules(&self, document: &Document) -> Vec<(EntityId, Vec<BendRecord>)> {
        self.final_sheet_layouts(document)
            .map(|(part, _, layout)| (part, bend_records(layout)))
            .collect()
    }

    /// Exact base and final-leg cutter frames for current sheet Parts.
    /// Source-only evaluations cannot establish final physical Part ownership.
    pub fn sheet_authoring_faces(&self, document: &Document) -> Vec<SheetFaceFrame> {
        let mut frames = Vec::new();
        for (part, feature, layout) in self.final_sheet_layouts(document) {
            let space = layout.space();
            let faces =
                std::iter::once(CutoutFace::Base).chain(layout.flanges.iter().map(|flange| {
                    CutoutFace::Flange {
                        index: flange.flange.index as u32,
                    }
                }));
            for face in faces {
                if let Ok((frame, normal)) = space.face_frame(face, &layout.flanges) {
                    frames.push(SheetFaceFrame {
                        part,
                        feature,
                        face,
                        origin_mm: frame.origin,
                        u: frame.u,
                        v: frame.v,
                        through_mm: normal.map(|coordinate| coordinate * layout.thickness_mm),
                    });
                }
            }
        }
        frames
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
            let Some((layout, solid)) = self.sheet_parts_solid(document, id, part) else {
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
                bends: bend_records(layout),
                issues,
            });
        }
        Ok(out)
    }
}
/// The base thickness: from the base face, the nearest planar face facing
/// along the thickness direction. Hem legs and jogs lie further away.
fn measured_thickness(solid: &Solid, layout: &SheetLayout) -> Result<f64, String> {
    let n = layout.thickness_direction;
    let mut base_face = false;
    let mut nearest = f64::INFINITY;
    for f in solid.faces().map_err(|e| e.to_string())? {
        let Some((origin, normal)) = f.plane else {
            continue;
        };
        let offset = dot(
            [
                origin[0] - layout.frame.origin[0],
                origin[1] - layout.frame.origin[1],
                origin[2] - layout.frame.origin[2],
            ],
            n,
        );
        let c = dot(normal, n);
        if c < -1. + 1e-9 && offset.abs() < TOLERANCE_MM {
            base_face = true;
        } else if c > 1. - 1e-9 && offset > TOLERANCE_MM {
            nearest = nearest.min(offset);
        }
    }
    if !base_face || !nearest.is_finite() {
        return Err("The sheet solid has no base face and opposite face".into());
    }
    Ok(nearest)
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
                flanges: vec![],
                cutouts: vec![],
                fasteners: vec![],
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
