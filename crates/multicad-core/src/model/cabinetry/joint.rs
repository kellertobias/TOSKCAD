//! Corner joints between rectangular panels. Two panels that overlap at a
//! corner share a corner block: the region inside both finished panels,
//! one full thickness of each. A joint decides who keeps which part of that
//! block. Each participating Part gets one `cabinetry.joint_cut` feature per
//! joint pair that removes the other panel's share from it, so both mating
//! sides follow any change to either panel.
use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::Mutation,
    model::{
        BodyInput, CreatedBody, ModelPrepared,
        common::{error, number, revision},
    },
    parameters::EvaluatedParameters,
};

pub const PANEL_JOINT_KIND: &str = "cabinetry.joint";
pub const PANEL_JOINT_CAPABILITY: &str = "cabinetry.joint.1";
pub const PANEL_JOINT_CUT_TYPE: &str = "cabinetry.joint_cut";
pub const PANEL_JOINT_CUT_SCHEMA: u32 = 1;
pub const PANEL_JOINT_CUT_CAPABILITY: &str = "cabinetry.joint_cut.1";
pub const MAX_JOINT_PAIRS: usize = 64;
pub const MAX_FINGERS: u32 = 128;
/// Tools reach this far past the faces they cut through, so no cut face is
/// coplanar with a face it must remove.
const MARGIN_MM: f64 = 1.;
const TOL: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelJointKind {
    /// Panel A runs through the corner; B is shortened to butt against it.
    ButtAThrough,
    /// Panel B runs through the corner; A is shortened to butt against it.
    ButtBThrough,
    /// The corner block is split on its diagonal; each panel keeps its half.
    Miter,
    /// The corner block is split into `count` fingers along the corner,
    /// alternating A, B, A…; `fit` is the total gap at each finger face.
    Finger { count: u32, fit: Dimension },
    /// Loose tenons (Domino style) between panels that meet face to edge:
    /// paired rounded mortises and a tenon Part per pair.
    LooseTenon(Box<LooseTenonSpec>),
}
impl PanelJointKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ButtAThrough | Self::ButtBThrough => "butt joint",
            Self::Miter => "miter joint",
            Self::Finger { .. } => "finger joint",
            Self::LooseTenon(_) => "loose-tenon joint",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelJointPair {
    pub a: EntityId,
    pub b: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelJointDefinition {
    pub kind: PanelJointKind,
    /// Panel Part pairs that share this joint's settings.
    pub pairs: Vec<PanelJointPair>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelJointSide {
    A,
    B,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelJointCutDefinition {
    pub joint: EntityId,
    pub pair: u32,
    pub side: PanelJointSide,
    pub input: BodyInput,
    pub output: BodyOutput,
}

impl PanelJointDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.pairs.is_empty() || self.pairs.len() > MAX_JOINT_PAIRS {
            return Err(invalid(format!(
                "A joint needs 1–{MAX_JOINT_PAIRS} panel pairs"
            )));
        }
        let mut seen = BTreeSet::new();
        for p in &self.pairs {
            if p.a.is_nil() || p.b.is_nil() || p.a == p.b {
                return Err(invalid("Each joint pair needs two different panels"));
            }
            let key = if p.a < p.b { (p.a, p.b) } else { (p.b, p.a) };
            if !seen.insert(key) {
                return Err(invalid("A panel pair appears twice in the joint"));
            }
        }
        if let PanelJointKind::Finger { count, fit } = &self.kind {
            if !(2..=MAX_FINGERS).contains(count) {
                return Err(invalid(format!(
                    "A finger joint needs 2–{MAX_FINGERS} fingers"
                )));
            }
            check_dimension(fit, Unit::Millimeter)?;
        }
        if let PanelJointKind::LooseTenon(spec) = &self.kind {
            spec.validate()?;
        }
        Ok(())
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        match &self.kind {
            PanelJointKind::Finger { fit, .. } => vec![fit],
            PanelJointKind::LooseTenon(spec) => spec.dimensions(),
            _ => vec![],
        }
    }
    /// Links to every panel feature, so a change to either panel rebuilds
    /// both sides.
    fn inputs(&self, document: &Document) -> Result<Vec<Link>, DocumentError> {
        let mut links = parameter_links(self.dimensions());
        for (i, p) in self.pairs.iter().enumerate() {
            for (side, part) in [("a", p.a), ("b", p.b)] {
                let feature = panel_feature(document, part)
                    .ok_or_else(|| invalid(format!("{part} is not a panel")))?;
                links.push(Link::new(feature, format!("{side}/{i}")));
            }
        }
        Ok(links)
    }
    pub(crate) fn entity_data(&self, document: &Document) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(PANEL_JOINT_KIND, self)?,
            inputs: self.inputs(document)?,
        })
    }
}
impl PanelJointCutDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.joint.is_nil()
            || self.input.producer.is_nil()
            || self.output.body != self.input.body
        {
            return Err(invalid("A joint cut modifies its own Part"));
        }
        definition(PANEL_JOINT_CUT_TYPE, self)
    }
    pub fn inputs(&self) -> Vec<Link> {
        vec![
            Link::new(self.input.producer, "source"),
            Link::new(self.joint, "joint"),
        ]
    }
}

pub fn panel_joints(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| is_kind(e, PANEL_JOINT_KIND))
}
pub fn panel_joint_cuts(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == PANEL_JOINT_CUT_TYPE),
    )
}
pub fn read_panel_joint(
    document: &Document,
    id: EntityId,
) -> Result<PanelJointDefinition, DocumentError> {
    let value: PanelJointDefinition = parse(
        &extension(document, id, PANEL_JOINT_KIND)?,
        PANEL_JOINT_KIND,
    )?;
    value.validate()?;
    Ok(value)
}
pub fn read_panel_joint_cut(
    document: &Document,
    feature: EntityId,
) -> Result<PanelJointCutDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a joint cut feature"));
    };
    let value: PanelJointCutDefinition = parse(definition, PANEL_JOINT_CUT_TYPE)?;
    value.definition()?;
    let expected = value.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid(
            "Joint cut source and canonical input links disagree",
        ));
    }
    if let Some(EntityData::Part { features, .. }) =
        document.get(value.output.body).map(|e| &e.data)
        && !features.contains(&feature)
    {
        return Err(invalid("Joint cut Part must contain its feature"));
    }
    Ok(value)
}
/// Joints with a pair that uses a Part.
pub fn panel_joints_of(document: &Document, part: EntityId) -> Vec<EntityId> {
    panel_joints(document)
        .filter(|e| {
            read_panel_joint(document, e.id)
                .is_ok_and(|j| j.pairs.iter().any(|p| p.a == part || p.b == part))
        })
        .map(|e| e.id)
        .collect()
}
pub(super) fn validate_panel_joints(document: &Document) -> Result<(), DocumentError> {
    for e in panel_joints(document) {
        read_panel_joint(document, e.id)?;
    }
    for e in panel_joint_cuts(document) {
        read_panel_joint_cut(document, e.id)?;
    }
    super::tenon::validate_loose_tenons(document)
}

/// A prism removed from one panel: a planar polygon swept along `sweep`.
#[derive(Clone, Debug, PartialEq)]
pub struct CutPrism {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub polygon: Vec<[f64; 2]>,
    /// A circle (center, radius) instead of the polygon, for drilled holes.
    pub circle: Option<([f64; 2], f64)>,
    /// A slot instead of the polygon: the two end-circle centers and their
    /// radius, for rounded mortises.
    pub slot: Option<([f64; 2], [f64; 2], f64)>,
    pub sweep: [f64; 3],
}
/// The resolved corner of one joint pair.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelJointCorner {
    /// Corner block size: A thickness, B thickness and length along the corner.
    pub block_mm: [f64; 3],
    /// Prisms removed from panel A and from panel B.
    pub cut_a: Vec<CutPrism>,
    pub cut_b: Vec<CutPrism>,
}

pub(super) fn panel_box(
    document: &Document,
    parameters: &EvaluatedParameters,
    part: EntityId,
) -> Result<PanelBox, DocumentError> {
    let name = document
        .get(part)
        .map_or("A panel".into(), |e| e.name.clone());
    let feature =
        panel_feature(document, part).ok_or_else(|| invalid(format!("{name} is not a panel")))?;
    match resolve_panel(document, parameters, &read_panel(document, feature)?)?.shape {
        PanelShape::Box(b) => Ok(b),
        PanelShape::Profile { .. } => Err(invalid(format!(
            "{name} is a sketch-region panel; joints need rectangular panels"
        ))),
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Resolve one pair: find the corner block and the prisms each panel loses.
pub fn resolve_panel_joint_pair(
    document: &Document,
    parameters: &EvaluatedParameters,
    joint: &PanelJointDefinition,
    pair: usize,
) -> Result<PanelJointCorner, DocumentError> {
    let p = joint
        .pairs
        .get(pair)
        .ok_or_else(|| invalid("The joint has no such pair"))?;
    if let PanelJointKind::LooseTenon(spec) = &joint.kind {
        return super::tenon::resolve_loose_tenon(document, parameters, spec, p).map(|r| r.0);
    }
    let a = panel_box(document, parameters, p.a)?;
    let b = panel_box(document, parameters, p.b)?;
    let axes = [a.u, a.v, a.normal()];
    // B's box in A's frame; its edges must run along A's axes.
    let to_a = |w: [f64; 3]| {
        let d = [w[0] - a.origin[0], w[1] - a.origin[1], w[2] - a.origin[2]];
        [dot(d, axes[0]), dot(d, axes[1]), dot(d, axes[2])]
    };
    let mut b_axis = [usize::MAX; 3];
    for (k, dir) in [b.u, b.v, b.normal()].into_iter().enumerate() {
        let i = (0..3)
            .find(|&i| dot(dir, axes[i]).abs() > 1. - 1e-9)
            .ok_or_else(|| invalid("Joint panels must be square to each other"))?;
        b_axis[k] = i;
    }
    let b_sizes = [b.length_mm, b.width_mm, b.thickness_mm];
    let b_min_corner = to_a(b.origin);
    let mut b_lo = [0.; 3];
    let mut b_hi = [0.; 3];
    for k in 0..3 {
        let i = b_axis[k];
        let dir = [b.u, b.v, b.normal()][k];
        let extent = b_sizes[k] * dot(dir, axes[i]).signum();
        let (lo, hi) = if extent >= 0. {
            (b_min_corner[i], b_min_corner[i] + extent)
        } else {
            (b_min_corner[i] + extent, b_min_corner[i])
        };
        b_lo[i] = lo;
        b_hi[i] = hi;
    }
    let a_hi = [a.length_mm, a.width_mm, a.thickness_mm];
    let lo: [f64; 3] = std::array::from_fn(|i| b_lo[i].max(0.));
    let hi: [f64; 3] = std::array::from_fn(|i| b_hi[i].min(a_hi[i]));
    if (0..3).any(|i| hi[i] - lo[i] <= TOL) {
        return Err(invalid(
            "The panels do not overlap at a corner; extend both to the outside corner",
        ));
    }
    // x: A's thickness axis; y: B's thickness axis; e: along the corner.
    let (x, y) = (2, b_axis[2]);
    if y == 2 {
        return Err(invalid(
            "Joint panels are parallel; a corner joint needs panels at a right angle",
        ));
    }
    let e = 3 - x - y;
    let ta = a.thickness_mm;
    let tb = b.thickness_mm;
    if (hi[x] - lo[x] - ta).abs() > 1e-6 || (hi[y] - lo[y] - tb).abs() > 1e-6 {
        return Err(invalid(
            "The corner block must be the full thickness of both panels; align the panels at the corner",
        ));
    }
    // Which way each panel continues past the corner block.
    let away = |plo: f64, phi: f64, blo: f64, bhi: f64, name: &str| {
        if phi > bhi + 1e-6 {
            Ok(1.)
        } else if plo < blo - 1e-6 {
            Ok(-1.)
        } else {
            Err(invalid(format!(
                "Panel {name} ends inside the corner block; it must continue past the corner"
            )))
        }
    };
    let sa = away(0., a_hi[y], lo[y], hi[y], "A")?;
    let sb = away(b_lo[x], b_hi[x], lo[x], hi[x], "B")?;
    let (x0, x1, y0, y1, e0, e1) = (lo[x], hi[x], lo[y], hi[y], lo[e], hi[e]);
    let m = MARGIN_MM;
    // The corner block, grown outward wherever the cut panel has no material.
    let butt_b = [
        [x0 - if sb > 0. { m } else { 0. }, y0 - m],
        [x1 + if sb < 0. { m } else { 0. }, y1 + m],
    ];
    let butt_a = [
        [x0 - m, y0 - if sa > 0. { m } else { 0. }],
        [x1 + m, y1 + if sa < 0. { m } else { 0. }],
    ];
    let rect = |r: [[f64; 2]; 2]| {
        vec![
            [r[0][0], r[0][1]],
            [r[1][0], r[0][1]],
            [r[1][0], r[1][1]],
            [r[0][0], r[1][1]],
        ]
    };
    let mut ex = [0.; 3];
    ex[x] = 1.;
    let mut ey = [0.; 3];
    ey[y] = 1.;
    let mut ee = [0.; 3];
    ee[e] = 1.;
    let world = |l: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|k| a.origin[k] + (0..3).map(|i| l[i] * axes[i][k]).sum::<f64>())
    };
    let dir = |l: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|k| (0..3).map(|i| l[i] * axes[i][k]).sum::<f64>())
    };
    let prism = |polygon: Vec<[f64; 2]>, from: f64, to: f64| {
        let mut o = [0.; 3];
        o[e] = from;
        let (u, v) = (dir(ex), dir(ey));
        // Keep the polygon frame right-handed with the sweep.
        let flip = dot(cross(u, v), dir(ee)) < 0.;
        CutPrism {
            origin: world(o),
            u: if flip { v } else { u },
            v: if flip { u } else { v },
            polygon: if flip {
                polygon.into_iter().rev().map(|[p, q]| [q, p]).collect()
            } else {
                polygon
            },
            circle: None,
            slot: None,
            sweep: dir(ee.map(|c| c * (to - from))),
        }
    };
    let full = |polygon| vec![prism(polygon, e0 - m, e1 + m)];
    let (cut_a, cut_b) = match &joint.kind {
        PanelJointKind::ButtAThrough => (vec![], full(rect(butt_b))),
        PanelJointKind::ButtBThrough => (full(rect(butt_a)), vec![]),
        PanelJointKind::Miter => {
            let out = [if sb > 0. { x0 } else { x1 }, if sa > 0. { y0 } else { y1 }];
            let inn = [if sb > 0. { x1 } else { x0 }, if sa > 0. { y1 } else { y0 }];
            let len = ((inn[0] - out[0]).powi(2) + (inn[1] - out[1]).powi(2)).sqrt();
            let d = [(inn[0] - out[0]) / len, (inn[1] - out[1]) / len];
            let out_m = [out[0] - m * d[0], out[1] - m * d[1]];
            let inn_m = [inn[0] + m * d[0], inn[1] + m * d[1]];
            // A keeps the half beside its body; B keeps the other.
            let a_corner = [out[0] - m * d[0].signum(), inn[1] + m * d[1].signum()];
            let b_corner = [inn[0] + m * d[0].signum(), out[1] - m * d[1].signum()];
            (
                full(vec![out_m, b_corner, inn_m]),
                full(vec![out_m, inn_m, a_corner]),
            )
        }
        PanelJointKind::LooseTenon(_) => unreachable!("resolved above"),
        PanelJointKind::Finger { count, fit } => {
            let fit = number(fit, Unit::Millimeter, parameters)?;
            let n = *count as usize;
            let w = (e1 - e0) / n as f64;
            if fit < 0. {
                return Err(invalid("Finger fit cannot be negative"));
            }
            if w - fit <= 0.5 {
                return Err(invalid(format!(
                    "{count} fingers along {:.1} mm leave fingers no wider than the {fit} mm fit; use fewer fingers",
                    e1 - e0
                )));
            }
            let segment = |k: usize| {
                let from = if k == 0 {
                    e0 - m
                } else {
                    e0 + k as f64 * w - fit / 2.
                };
                let to = if k == n - 1 {
                    e1 + m
                } else {
                    e0 + (k + 1) as f64 * w + fit / 2.
                };
                (from, to)
            };
            // Even fingers are A's: B loses them; odd fingers are B's.
            let take = |parity: usize, r: [[f64; 2]; 2]| {
                (0..n)
                    .filter(|k| k % 2 == parity)
                    .map(|k| {
                        let (from, to) = segment(k);
                        prism(rect(r), from, to)
                    })
                    .collect::<Vec<_>>()
            };
            (take(1, butt_a), take(0, butt_b))
        }
    };
    Ok(PanelJointCorner {
        block_mm: [ta, tb, e1 - e0],
        cut_a,
        cut_b,
    })
}

#[derive(Clone, Debug)]
pub struct PanelJointPrepared {
    pub joint: EntityId,
    pub model: ModelPrepared,
}
/// Append one joint cut feature to a panel Part.
fn add_cut(
    document: &mut Document,
    edits: &mut Vec<Mutation>,
    joint: EntityId,
    pair: usize,
    side: PanelJointSide,
    part: EntityId,
) -> Result<CreatedBody, CommandError> {
    let name = document.get(part).map_or(String::new(), |e| e.name.clone());
    super::chain::append_cut(document, edits, part, format!("{name} joint"), |input| {
        let cut = PanelJointCutDefinition {
            joint,
            pair: pair as u32,
            side,
            input,
            output: BodyOutput { body: part },
        };
        Ok((cut.definition()?, cut.inputs()))
    })
}

/// Add the tenon Part of one loose-tenon pair, in the container of panel A.
fn add_tenon(
    document: &mut Document,
    edits: &mut Vec<Mutation>,
    joint: EntityId,
    name: &str,
    pair: usize,
    pairs: usize,
    beside: EntityId,
) -> Result<CreatedBody, CommandError> {
    let (steps, feature, body) = super::tenon::tenon_part(joint, name, pair, pairs)?;
    *document = document.edited(&steps)?;
    edits.extend(steps);
    let index = crate::hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    if let Some(parent) = index.parent(beside).map_err(|e| error(e.to_string()))? {
        let mut state = document.hierarchy().clone();
        state.members.entry(parent).or_default().push(body);
        let step = Mutation::SetHierarchy(state);
        *document = document.edited(std::slice::from_ref(&step))?;
        edits.push(step);
    }
    Ok(CreatedBody { feature, body })
}
/// Create a joint and every panel's cut in one transaction. The corner of
/// each pair is checked against the current panels before anything is
/// staged; a native evaluator still builds the cut solids.
pub fn prepare_panel_joint(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    joint: &PanelJointDefinition,
) -> Result<PanelJointPrepared, CommandError> {
    revision(session, expected_revision)?;
    if name.trim().is_empty() || name.len() > 1000 {
        return Err(error("Joint name must contain 1–1000 bytes"));
    }
    let document = session.document();
    joint.validate()?;
    for i in 0..joint.pairs.len() {
        resolve_panel_joint_pair(document, session.parameters(), joint, i)?;
    }
    let entity = Entity::new(name, joint.entity_data(document)?);
    let id = entity.id;
    let mut edits = vec![Mutation::Insert(entity)];
    let mut candidate = document.edited(&edits)?;
    let mut outputs = vec![];
    for (i, p) in joint.pairs.iter().enumerate() {
        for (side, part) in [(PanelJointSide::A, p.a), (PanelJointSide::B, p.b)] {
            outputs.push(add_cut(&mut candidate, &mut edits, id, i, side, part)?);
        }
    }
    if matches!(joint.kind, PanelJointKind::LooseTenon(_)) {
        for (i, p) in joint.pairs.iter().enumerate() {
            outputs.push(add_tenon(
                &mut candidate,
                &mut edits,
                id,
                name,
                i,
                joint.pairs.len(),
                p.a,
            )?);
        }
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Create {}", joint.kind.label()),
        edits,
    })?;
    Ok(PanelJointPrepared {
        joint: id,
        model: ModelPrepared { prepared, outputs },
    })
}
/// Change a joint's kind, finger count or fit. Its panel pairs and cut
/// features stay; both sides of every pair rebuild.
pub fn prepare_panel_joint_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    kind: &PanelJointKind,
) -> Result<PanelJointPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let mut joint = read_panel_joint(document, id)?;
    if matches!(joint.kind, PanelJointKind::LooseTenon(_))
        != matches!(kind, PanelJointKind::LooseTenon(_))
    {
        return Err(error(
            "A loose-tenon joint and a corner joint need different panel placements; delete the joint and create another",
        ));
    }
    joint.kind = kind.clone();
    joint.validate()?;
    for i in 0..joint.pairs.len() {
        resolve_panel_joint_pair(document, session.parameters(), &joint, i)?;
    }
    let mut entity = document
        .get(id)
        .cloned()
        .ok_or_else(|| error("Joint is missing"))?;
    entity.data = joint.entity_data(document)?;
    let outputs = panel_joint_cuts(document)
        .filter_map(|e| {
            read_panel_joint_cut(document, e.id)
                .ok()
                .filter(|c| c.joint == id)
                .map(|c| CreatedBody {
                    feature: e.id,
                    body: c.output.body,
                })
        })
        .chain(
            super::tenon::joint_tenons(document, id)
                .into_iter()
                .map(|(_, feature, body)| CreatedBody { feature, body }),
        )
        .collect();
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit joint".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(PanelJointPrepared {
        joint: id,
        model: ModelPrepared { prepared, outputs },
    })
}
/// Delete a joint and its cuts. A later joint cut on the same Part is
/// re-attached to the feature before the removed cut; any other later
/// feature must be removed first.
pub fn prepare_panel_joint_delete(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
) -> Result<crate::commands::Prepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    read_panel_joint(document, id)?;
    let mut candidate = document.clone();
    let mut edits = vec![];
    let cuts: Vec<_> = panel_joint_cuts(document)
        .filter(|e| read_panel_joint_cut(document, e.id).is_ok_and(|c| c.joint == id))
        .map(|e| e.id)
        .collect();
    super::chain::remove_cuts(&mut candidate, &mut edits, &cuts, "joint")?;
    for (_, feature, body) in super::tenon::joint_tenons(document, id) {
        let mut state = candidate.hierarchy().clone();
        for members in state.members.values_mut() {
            members.retain(|m| *m != body && *m != feature);
        }
        let mut steps = vec![];
        if &state != candidate.hierarchy() {
            steps.push(Mutation::SetHierarchy(state));
        }
        steps.extend([Mutation::Remove(body), Mutation::Remove(feature)]);
        candidate = candidate.edited(&steps)?;
        edits.extend(steps);
    }
    edits.push(Mutation::Remove(id));
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Delete joint".into(),
        edits,
    })
}
