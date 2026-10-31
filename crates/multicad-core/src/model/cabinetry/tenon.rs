//! Loose-tenon (Domino style) joints between two panels that meet face to
//! edge: one panel's edge face lies flat on the other panel's face. Each
//! tenon gets a rounded mortise in the face panel and a matching one in the
//! edge panel's end, and a `cabinetry.loose_tenon` feature builds the
//! tenons of a pair as their own Part. Everything is resolved from the two
//! current panel boxes, so a panel change rebuilds both mortises and the
//! tenons together.
use super::*;
use crate::{document::Mutation, model::common::number, parameters::EvaluatedParameters};

pub const LOOSE_TENON_TYPE: &str = "cabinetry.loose_tenon";
pub const LOOSE_TENON_SCHEMA: u32 = 1;
pub const LOOSE_TENON_CAPABILITY: &str = "cabinetry.loose_tenon.1";
pub const MAX_TENONS: u32 = 32;
/// The least material a face mortise must leave behind it.
pub const MIN_WALL_MM: f64 = 2.;
const TOL: f64 = 1e-6;
/// How far apart two faces may be and still count as touching.
const CONTACT_MM: f64 = 1e-3;

/// A loose tenon and its two mortises. The tenon is a rounded slot shape:
/// `thickness` across (the cutter diameter), `width` along the joint
/// including its rounded ends, and `length` from mortise to mortise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooseTenonSpec {
    pub thickness: Dimension,
    pub width: Dimension,
    pub length: Dimension,
    /// Tenons along the contact, evenly spaced between the insets.
    pub count: u32,
    /// From each end of the contact to the first and last tenon center.
    pub inset: Dimension,
    /// Tenon center across the edge panel's thickness, from its middle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Dimension>,
    /// Mortise depth into the face panel, and into the edge panel's end.
    pub face_depth: Dimension,
    pub edge_depth: Dimension,
    /// Extra mortise length beyond the tenon width (0 = tight), per panel.
    pub face_play: Dimension,
    pub edge_play: Dimension,
    /// Extra mortise thickness beyond the tenon thickness.
    pub fit: Dimension,
}
impl LooseTenonSpec {
    pub(super) fn validate(&self) -> Result<(), DocumentError> {
        if !(1..=MAX_TENONS).contains(&self.count) {
            return Err(invalid(format!(
                "A loose-tenon joint needs 1–{MAX_TENONS} tenons per pair"
            )));
        }
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        [
            &self.thickness,
            &self.width,
            &self.length,
            &self.inset,
            &self.face_depth,
            &self.edge_depth,
            &self.face_play,
            &self.edge_play,
            &self.fit,
        ]
        .into_iter()
        .chain(&self.offset)
        .collect()
    }
}

/// One tenon in world millimeters: a slot-shaped section centered on the
/// contact plane at `center`, `long` along its width, reaching
/// `into_face` against `axis` (into the face panel) and `into_edge` along
/// it (into the edge panel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTenon {
    pub center: [f64; 3],
    pub axis: [f64; 3],
    pub long: [f64; 3],
    pub width_mm: f64,
    pub thickness_mm: f64,
    pub into_face_mm: f64,
    pub into_edge_mm: f64,
}
/// The resolved face-to-edge contact of one pair.
#[derive(Clone, Debug, PartialEq)]
pub struct LooseTenonLayout {
    /// The pair side whose face receives mortises; the other side's edge does.
    pub face_side: PanelJointSide,
    pub face: MachiningFace,
    pub edge: PanelEdgeName,
    /// Insertion direction: from the face panel into the edge panel.
    pub insertion: [f64; 3],
    /// Mortises cut into the face panel and the edge panel.
    pub face_mortises: Vec<ResolvedRecess>,
    pub edge_mortises: Vec<ResolvedRecess>,
    pub face_frame: MachiningFaceFrame,
    pub edge_frame: MachiningFaceFrame,
    pub tenons: Vec<ResolvedTenon>,
    pub contact_mm: f64,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
const EDGES: [PanelEdgeName; 4] = [
    PanelEdgeName::UStart,
    PanelEdgeName::UEnd,
    PanelEdgeName::VStart,
    PanelEdgeName::VEnd,
];

/// The face of `f` and edge of `e` that touch: the edge face lies on the
/// face plane, facing it, and runs along one of its axes.
fn contact(f: &PanelBox, e: &PanelBox) -> Option<(MachiningFace, PanelEdgeName)> {
    for face in [MachiningFace::Top, MachiningFace::Bottom] {
        let ff = panel_face_frame(f, face);
        for edge in EDGES {
            let ef = panel_face_frame(e, MachiningFace::Edge { edge });
            if dot(ef.inward, ff.inward) > -1. + 1e-9 {
                continue;
            }
            if dot(sub(ef.origin, ff.origin), ff.inward).abs() > CONTACT_MM {
                continue;
            }
            let along = dot(ef.a, ff.a).abs().max(dot(ef.a, ff.b).abs());
            if along < 1. - 1e-9 {
                continue;
            }
            // The edge face must overlap the face rectangle.
            let (p, _) = ff.local(ef.origin);
            let (q, _) = ff.local(ef.point(ef.size));
            let lo = [p[0].min(q[0]), p[1].min(q[1])];
            let hi = [p[0].max(q[0]), p[1].max(q[1])];
            if hi[0] <= lo[0].max(0.) + TOL
                || hi[1] <= lo[1].max(0.) + TOL
                || lo[0] >= ff.size[0] - TOL
                || lo[1] >= ff.size[1] - TOL
            {
                continue;
            }
            return Some((face, edge));
        }
    }
    None
}

/// Resolve the mortises and tenons of one loose-tenon pair.
pub fn resolve_loose_tenon(
    document: &Document,
    parameters: &EvaluatedParameters,
    spec: &LooseTenonSpec,
    pair: &PanelJointPair,
) -> Result<(PanelJointCorner, LooseTenonLayout), DocumentError> {
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let a = super::joint::panel_box(document, parameters, pair.a)?;
    let b = super::joint::panel_box(document, parameters, pair.b)?;
    let name = |part: EntityId| document.get(part).map_or(String::new(), |e| e.name.clone());
    let (face_side, (f, fp), (e, ep), (face, edge)) = if let Some(c) = contact(&a, &b) {
        (PanelJointSide::A, (a, pair.a), (b, pair.b), c)
    } else if let Some(c) = contact(&b, &a) {
        (PanelJointSide::B, (b, pair.b), (a, pair.a), c)
    } else {
        return Err(invalid(format!(
            "{} and {} do not meet face to edge; place one panel's edge flat against the other panel's face",
            name(pair.a),
            name(pair.b)
        )));
    };
    let (fname, ename) = (name(fp), name(ep));
    let (t, w, len) = (mm(&spec.thickness)?, mm(&spec.width)?, mm(&spec.length)?);
    let (inset, offset) = (
        mm(&spec.inset)?,
        spec.offset.as_ref().map(mm).transpose()?.unwrap_or(0.),
    );
    let (fd, ed) = (mm(&spec.face_depth)?, mm(&spec.edge_depth)?);
    let (fplay, eplay, fit) = (mm(&spec.face_play)?, mm(&spec.edge_play)?, mm(&spec.fit)?);
    if t <= TOL || w < t - TOL || len <= TOL {
        return Err(invalid(
            "A tenon needs a positive thickness and length and a width at least its thickness",
        ));
    }
    if fd <= TOL || ed <= TOL || fplay < 0. || eplay < 0. || fit < 0. || inset < 0. {
        return Err(invalid(
            "Mortise depths must be positive; plays, fit and inset cannot be negative",
        ));
    }
    let gap = fd + ed - len;
    if gap < -TOL {
        return Err(invalid(format!(
            "The {fd} + {ed} mm mortises are shallower than the {len} mm tenon; deepen them or use a shorter tenon"
        )));
    }
    let ff = panel_face_frame(&f, face);
    let ef = panel_face_frame(&e, MachiningFace::Edge { edge });
    if fd > ff.depth - MIN_WALL_MM + TOL {
        return Err(invalid(format!(
            "A {fd} mm mortise leaves {:.1} mm of the {} mm {fname}; at least {MIN_WALL_MM} mm must remain",
            ff.depth - fd,
            ff.depth
        )));
    }
    if ed >= ef.depth - TOL {
        return Err(invalid(format!(
            "A {ed} mm end mortise is as deep as {ename} is long there ({} mm)",
            ef.depth
        )));
    }
    // The contact along the edge, in edge-face coordinates.
    let along_face = |s: f64| ff.local(ef.point([s, 0.])).0;
    let (lo, hi) = {
        let k = if dot(ef.a, ff.a).abs() > 0.5 { 0 } else { 1 };
        let p0 = along_face(0.)[k];
        let dir = along_face(1.)[k] - p0;
        // Edge coordinate s where the face coordinate is c: p0 + dir·s = c.
        let s = |c: f64| (c - p0) / dir;
        let (s0, s1) = (s(0.), s(ff.size[k]));
        (s0.min(s1).max(0.), s0.max(s1).min(ef.size[0]))
    };
    let contact_mm = hi - lo;
    let pitch = if spec.count > 1 {
        (contact_mm - 2. * inset) / (spec.count - 1) as f64
    } else {
        0.
    };
    if contact_mm - 2. * inset < -TOL {
        return Err(invalid(format!(
            "The {inset} mm insets do not fit the {contact_mm:.1} mm contact between {fname} and {ename}"
        )));
    }
    let longest = w + fplay.max(eplay);
    if spec.count > 1 && pitch < longest - TOL {
        return Err(invalid(format!(
            "{} tenons {pitch:.1} mm apart overlap; use fewer tenons or smaller insets",
            spec.count
        )));
    }
    let across = ef.size[1] / 2. + offset;
    let mut layout = LooseTenonLayout {
        face_side,
        face,
        edge,
        insertion: ef.inward,
        face_mortises: vec![],
        edge_mortises: vec![],
        face_frame: ff,
        edge_frame: ef,
        tenons: vec![],
        contact_mm,
    };
    let (mut face_cut, mut edge_cut) = (vec![], vec![]);
    let r = (t + fit) / 2.;
    for i in 0..spec.count {
        let s = if spec.count == 1 {
            (lo + hi) / 2.
        } else {
            lo + inset + i as f64 * pitch
        };
        // The edge mortise, in the edge face: long along the edge.
        let half = (w + eplay) / 2. - r;
        let (ea, eb) = ([s - half.max(0.), across], [s + half.max(0.), across]);
        if s - (w + eplay) / 2. < -TOL
            || s + (w + eplay) / 2. > ef.size[0] + TOL
            || across - r < -TOL
            || across + r > ef.size[1] + TOL
        {
            return Err(invalid(format!(
                "Tenon {} leaves the {:.1} × {:.1} mm end of {ename}; move it in or use a smaller tenon",
                i + 1,
                ef.size[0],
                ef.size[1]
            )));
        }
        edge_cut.push(ef.slot_prism(ea, eb, r, ed));
        layout.edge_mortises.push(mortise(ea, eb, r, ed));
        // The face mortise, at the same world point, long along the edge.
        let center = ef.point([s, across]);
        let (c, _) = ff.local(center);
        let (d, _) = ff.local(ef.point([s + 1., across]));
        let dir = [d[0] - c[0], d[1] - c[1]];
        let half = (w + fplay) / 2. - r;
        let fa = [c[0] - dir[0] * half.max(0.), c[1] - dir[1] * half.max(0.)];
        let fb = [c[0] + dir[0] * half.max(0.), c[1] + dir[1] * half.max(0.)];
        // The mortise's extent on the face: half its length along the edge
        // direction, its radius across.
        let reach = (w + fplay) / 2.;
        let off = (0..2).any(|k| {
            let extent = if dir[k].abs() > 0.5 { reach } else { r };
            c[k] - extent < -TOL || c[k] + extent > ff.size[k] + TOL
        });
        if off {
            return Err(invalid(format!(
                "Tenon {}'s mortise leaves the face of {fname}; move it in or use a smaller tenon",
                i + 1
            )));
        }
        face_cut.push(ff.slot_prism(fa, fb, r, fd));
        layout.face_mortises.push(mortise(fa, fb, r, fd));
        layout.tenons.push(ResolvedTenon {
            center,
            axis: ef.inward,
            long: ef.a,
            width_mm: w,
            thickness_mm: t,
            into_face_mm: fd - gap / 2.,
            into_edge_mm: ed - gap / 2.,
        });
    }
    let (cut_a, cut_b) = match face_side {
        PanelJointSide::A => (face_cut, edge_cut),
        PanelJointSide::B => (edge_cut, face_cut),
    };
    Ok((
        PanelJointCorner {
            block_mm: [f.thickness_mm, e.thickness_mm, contact_mm],
            cut_a,
            cut_b,
        },
        layout,
    ))
}
fn mortise(a: [f64; 2], b: [f64; 2], r: f64, depth: f64) -> ResolvedRecess {
    let d = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let n = if d > TOL {
        [-(b[1] - a[1]) / d, (b[0] - a[0]) / d]
    } else {
        [0., 1.]
    };
    let dir = if d > TOL {
        [(b[0] - a[0]) / d, (b[1] - a[1]) / d]
    } else {
        [1., 0.]
    };
    // Outline: a half circle around b from −n through +dir to +n, then one
    // around a from +n through −dir to −n.
    let mut outline = vec![];
    for (center, start) in [(b, -0.5), (a, 0.5)] {
        for k in 0..=8 {
            let phi = std::f64::consts::PI * (start + k as f64 / 8.);
            let (s, c) = phi.sin_cos();
            outline.push([
                center[0] + r * (c * dir[0] + s * n[0]),
                center[1] + r * (c * dir[1] + s * n[1]),
            ]);
        }
    }
    ResolvedRecess {
        kind: "mortise",
        size_mm: [d + 2. * r, 2. * r],
        depth_mm: depth,
        outline,
    }
}

/// The feature building the tenons of one loose-tenon joint pair as a Part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooseTenonDefinition {
    pub joint: EntityId,
    pub pair: u32,
    pub output: BodyOutput,
}
impl LooseTenonDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.joint.is_nil() || self.output.body.is_nil() {
            return Err(invalid("A loose tenon needs its joint and Part"));
        }
        definition(LOOSE_TENON_TYPE, self)
    }
    pub fn inputs(&self) -> Vec<Link> {
        vec![Link::new(self.joint, "joint")]
    }
}
pub fn loose_tenons(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == LOOSE_TENON_TYPE),
    )
}
pub fn read_loose_tenon(
    document: &Document,
    feature: EntityId,
) -> Result<LooseTenonDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a loose tenon feature"));
    };
    let value: LooseTenonDefinition = parse(definition, LOOSE_TENON_TYPE)?;
    value.definition()?;
    if inputs != &value.inputs() {
        return Err(invalid(
            "Loose tenon source and canonical input links disagree",
        ));
    }
    Ok(value)
}
/// The tenons of one pair, for the native evaluator.
pub fn resolve_loose_tenon_feature(
    document: &Document,
    parameters: &EvaluatedParameters,
    feature: EntityId,
) -> Result<Vec<ResolvedTenon>, DocumentError> {
    let t = read_loose_tenon(document, feature)?;
    let joint = read_panel_joint(document, t.joint)?;
    let PanelJointKind::LooseTenon(spec) = &joint.kind else {
        return Err(invalid(
            "The tenon's joint is no longer a loose-tenon joint",
        ));
    };
    let pair = joint
        .pairs
        .get(t.pair as usize)
        .ok_or_else(|| invalid("The tenon's joint pair is missing"))?;
    Ok(resolve_loose_tenon(document, parameters, spec, pair)?
        .1
        .tenons)
}
/// The tenon features and Parts of a joint, by pair.
pub fn joint_tenons(document: &Document, joint: EntityId) -> Vec<(u32, EntityId, EntityId)> {
    loose_tenons(document)
        .filter_map(|e| {
            read_loose_tenon(document, e.id)
                .ok()
                .filter(|t| t.joint == joint)
                .map(|t| (t.pair, e.id, t.output.body))
        })
        .collect()
}
pub(super) fn validate_loose_tenons(document: &Document) -> Result<(), DocumentError> {
    for e in loose_tenons(document) {
        read_loose_tenon(document, e.id)?;
    }
    Ok(())
}
/// Insert one tenon Part and its feature for a pair, beside the panels.
pub(super) fn tenon_part(
    joint: EntityId,
    joint_name: &str,
    pair: usize,
    pairs: usize,
) -> Result<(Vec<Mutation>, EntityId, EntityId), DocumentError> {
    let body = EntityId::new();
    let def = LooseTenonDefinition {
        joint,
        pair: pair as u32,
        output: BodyOutput { body },
    };
    let feature = Entity::new(
        format!("{joint_name} tenon feature"),
        EntityData::Feature {
            definition: def.definition()?,
            inputs: def.inputs(),
            suppressed: false,
        },
    );
    let name = if pairs > 1 {
        format!("{joint_name} tenons {}", pair + 1)
    } else {
        format!("{joint_name} tenons")
    };
    let mut part = Entity::new(
        name,
        EntityData::Part {
            features: vec![feature.id],
            material: None,
            tags: BTreeSet::new(),
        },
    );
    part.id = body;
    let id = feature.id;
    Ok((
        vec![Mutation::Insert(feature), Mutation::Insert(part)],
        id,
        body,
    ))
}
