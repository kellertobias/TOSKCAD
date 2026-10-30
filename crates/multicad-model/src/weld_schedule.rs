//! Weld schedule, member cut groups and weld-plan drawing annotations. The
//! schedule lists every weld in authored sequence order with the position
//! of its joint resolved from the current members, so it follows path edits.
//! These records are documentation for drawings and schedules (CAD-89), not
//! welding procedures, distortion, strength or heat-input calculations.
use crate::{MemberRecord, Model, ModelError, error};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, DocumentError, EntityId},
    drawings::{Annotation, AnnotationKind, DrawingView, WeldSide, WeldSymbol, view_coordinates},
    model::{self, ConnectionKind, EndCut, ResolvedMember},
    parameters::{EvaluatedParameters, Unit},
};

#[derive(Clone, Debug, PartialEq)]
pub struct WeldRecord {
    pub weld: EntityId,
    pub mark: String,
    /// The sequence holding the weld and its 1-based step, if sequenced.
    pub sequence: Option<EntityId>,
    pub step: Option<usize>,
    pub step_note: String,
    pub connection: EntityId,
    pub connection_name: String,
    pub kind: ConnectionKind,
    /// Member Parts in connection order (terminating or branch first).
    pub members: [EntityId; 2],
    pub member_names: [String; 2],
    /// Where the member paths meet, in world millimeters.
    pub position: [f64; 3],
    /// Outside perimeter of the first member's plane-cut end, measured on
    /// the cut plane; none for copes.
    pub joint_perimeter_mm: Option<f64>,
    pub symbol: WeldSymbol,
    pub side: WeldSide,
    pub size_mm: Option<f64>,
    pub length_mm: Option<f64>,
    pub all_around: bool,
    pub field: bool,
    pub process: String,
    pub notes: String,
}

/// Identical member cuts: same stock, overall length and end treatments.
#[derive(Clone, Debug, PartialEq)]
pub struct CutGroup {
    pub quantity: usize,
    pub parts: Vec<EntityId>,
    pub names: Vec<String>,
    pub stock: EntityId,
    pub designation: String,
    pub length_mm: f64,
    /// End treatments as (kind label, angle), start then end of the first
    /// member in the group.
    pub ends: [(String, f64); 2],
    /// The long point section face of each angled plane-cut end, start then
    /// end of the first member in the group.
    pub long_faces: [Option<&'static str>; 2],
    /// With two angled plane-cut ends: whether their long points lie on the
    /// "same face", "opposite faces" or "adjacent faces" of the section. It
    /// does not change when a member is turned end for end.
    pub long_points: Option<&'static str>,
}
/// End treatment labels with angles in hundredths of a degree.
type EndKey = [(String, i64); 2];
/// How the long points of a member's two ends relate.
fn long_points(r: &MemberRecord) -> Option<&'static str> {
    let (a, b) = (r.ends[0].long_point?, r.ends[1].long_point?);
    let d: f64 = (0..3).map(|i| a.direction[i] * b.direction[i]).sum();
    Some(if d > 0.5 {
        "same face"
    } else if d < -0.5 {
        "opposite faces"
    } else {
        "adjacent faces"
    })
}
/// A sequenced weld's sequence, 1-based step and note.
type Step = Option<(EntityId, usize, String)>;
fn end_key(r: &MemberRecord) -> EndKey {
    let key = |e: &crate::EndRecord| {
        (
            e.kind.map_or("square".to_owned(), |k| k.label().to_owned()),
            (e.angle_deg * 100.).round() as i64,
        )
    };
    let mut ends = [key(&r.ends[0]), key(&r.ends[1])];
    // A member turned end for end is the same cut.
    ends.sort();
    ends
}
/// Group member records into identical cuts with a quantity. Lengths match
/// within 0.01 mm, angles within 0.01° and the long points of two angled
/// ends must relate the same way.
pub fn group_cuts(records: &[MemberRecord]) -> Vec<CutGroup> {
    let mut groups: Vec<(CutGroup, EndKey, i64)> = vec![];
    for r in records {
        let (ends, length) = (end_key(r), (r.length_mm * 100.).round() as i64);
        let relation = long_points(r);
        if let Some((g, ..)) = groups.iter_mut().find(|(g, e, l)| {
            g.stock == r.stock && *e == ends && *l == length && g.long_points == relation
        }) {
            g.quantity += 1;
            g.parts.push(r.part);
            g.names.push(r.name.clone());
            continue;
        }
        let label = |e: &crate::EndRecord| {
            (
                e.kind.map_or("square".to_owned(), |k| k.label().to_owned()),
                e.angle_deg,
            )
        };
        groups.push((
            CutGroup {
                quantity: 1,
                parts: vec![r.part],
                names: vec![r.name.clone()],
                stock: r.stock,
                designation: r.designation.clone(),
                length_mm: r.length_mm,
                ends: [label(&r.ends[0]), label(&r.ends[1])],
                long_faces: r.ends.each_ref().map(|e| e.long_point.map(|p| p.face)),
                long_points: relation,
            },
            ends,
            length,
        ));
    }
    groups.into_iter().map(|(g, ..)| g).collect()
}

fn mm(d: &Option<Dimension>, p: &EvaluatedParameters) -> Result<Option<f64>, DocumentError> {
    d.as_ref()
        .map(|d| {
            d.value_in(Unit::Millimeter, Some(p))
                .map_err(|e| DocumentError::Invalid(e.to_string()))
        })
        .transpose()
}
/// Perimeter of the member's outside where the plane cut meets it: each
/// outline point is moved along the axis onto the cut plane.
fn cut_perimeter(member: &ResolvedMember, point: [f64; 3], normal: [f64; 3], angle: f64) -> f64 {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if member.section.shape == multicad_core::materials::ProfileShape::Round {
        // An ellipse with semi-axes r and r / cos(angle), by Ramanujan's
        // second approximation.
        let a = member.section.width_mm / 2.;
        let b = a / angle.to_radians().cos();
        let h = ((a - b) / (a + b)).powi(2);
        return std::f64::consts::PI * (a + b) * (1. + 3. * h / (10. + (4. - 3. * h).sqrt()));
    }
    let axis = member.frame.axis;
    let s = member.axial(point);
    let projected: Vec<[f64; 3]> = member
        .outline_world(s)
        .into_iter()
        .map(|q| {
            let t = dot([point[0] - q[0], point[1] - q[1], point[2] - q[2]], normal)
                / dot(axis, normal);
            [q[0] + axis[0] * t, q[1] + axis[1] * t, q[2] + axis[2] * t]
        })
        .collect();
    (0..projected.len())
        .map(|i| {
            let (a, b) = (projected[i], projected[(i + 1) % projected.len()]);
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
        })
        .sum()
}

impl Model {
    fn snapshot_member(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        part: EntityId,
    ) -> Result<ResolvedMember, ModelError> {
        let feature = model::member_feature(document, part)
            .ok_or_else(|| error(Some(part), "The welded Part is no longer a member"))?;
        let e = |e: DocumentError| error(Some(feature), e);
        let source = model::read_member(document, feature).map_err(e)?;
        let segment = self
            .snapshot()
            .member_paths
            .get(&feature)
            .ok_or_else(|| error(Some(feature), "The welded member was not evaluated"))?;
        model::resolve_member(document, parameters, &source, *segment).map_err(e)
    }
    /// Every weld in authored order: sequenced welds by sequence and step,
    /// then unsequenced welds by mark.
    pub fn weld_schedule(&self, document: &Document) -> Result<Vec<WeldRecord>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let mut order: Vec<(EntityId, Step)> = vec![];
        for s in model::sequences(document) {
            let sequence =
                model::read_sequence(document, s.id).map_err(|e| error(Some(s.id), e))?;
            for (i, step) in sequence.steps.into_iter().enumerate() {
                order.push((step.weld, Some((s.id, i + 1, step.note))));
            }
        }
        // Unsequenced welds follow, ordered by mark so the schedule does not
        // depend on entity identities.
        let mut rest = vec![];
        for w in model::welds(document) {
            if !order.iter().any(|(id, _)| *id == w.id) {
                let mark = model::read_weld(document, w.id)
                    .map(|weld| weld.mark.trim().to_owned())
                    .unwrap_or_default();
                rest.push((mark, w.id));
            }
        }
        rest.sort();
        order.extend(rest.into_iter().map(|(_, id)| (id, None)));
        let name = |id: EntityId| document.get(id).map(|e| e.name.clone()).unwrap_or_default();
        let mut out = vec![];
        for (id, step) in order {
            let e = |e: DocumentError| error(Some(id), e);
            let weld = model::read_weld(document, id).map_err(e)?;
            let connection = model::read_connection(document, weld.connection).map_err(e)?;
            let a = self.snapshot_member(document, &parameters, connection.members[0])?;
            let b = self.snapshot_member(document, &parameters, connection.members[1])?;
            let gap = connection.gap_mm(&parameters).map_err(e)?;
            let resolved = model::resolve_connection(connection.kind, &a, &b, gap).map_err(e)?;
            let joint_perimeter_mm = match resolved.treatments[0] {
                Some(t) => match t.cut {
                    EndCut::Plane { point, normal } => {
                        Some(cut_perimeter(&a, point, normal, t.angle_deg))
                    }
                    EndCut::Cope { .. } => None,
                },
                None => None,
            };
            let (sequence, step, step_note) = match step {
                Some((s, i, note)) => (Some(s), Some(i), note),
                None => (None, None, String::new()),
            };
            out.push(WeldRecord {
                weld: id,
                mark: weld.mark.trim().to_owned(),
                sequence,
                step,
                step_note,
                connection: weld.connection,
                connection_name: name(weld.connection),
                kind: connection.kind,
                members: connection.members,
                member_names: connection.members.map(name),
                position: resolved.point,
                joint_perimeter_mm,
                symbol: weld.symbol,
                side: weld.side,
                size_mm: mm(&weld.size, &parameters).map_err(e)?,
                length_mm: mm(&weld.length, &parameters).map_err(e)?,
                all_around: weld.all_around,
                field: weld.field,
                process: weld.process,
                notes: weld.notes,
            });
        }
        Ok(out)
    }
}

fn number(v: Option<f64>) -> String {
    v.map_or(String::new(), |v| {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    })
}
fn symbol_label(s: WeldSymbol) -> &'static str {
    match s {
        WeldSymbol::Fillet => "fillet",
        WeldSymbol::SquareGroove => "square groove",
        WeldSymbol::VGroove => "V groove",
        WeldSymbol::BevelGroove => "bevel groove",
        WeldSymbol::Plug => "plug",
        WeldSymbol::Spot => "spot",
        WeldSymbol::Seam => "seam",
    }
}
/// Weld-plan annotations for one drawing view: a weld symbol whose arrow
/// points at each weld's joint projected into the view, tailed with the
/// weld mark and process, and a sequence table at `table_mm`. They are page
/// content placed from the current schedule; they do not follow later
/// geometry edits until the plan is placed again.
pub fn weld_plan_annotations(
    records: &[WeldRecord],
    view: &DrawingView,
    table_mm: [f64; 2],
) -> Vec<Annotation> {
    let mut out: Vec<Annotation> = records
        .iter()
        .map(|r| {
            let p = view.paper_point(view_coordinates(view, r.position));
            let tail = if r.process.is_empty() {
                r.mark.clone()
            } else {
                format!("{} {}", r.mark, r.process)
            };
            Annotation {
                id: EntityId::new(),
                kind: AnnotationKind::Weld {
                    points_mm: vec![p, [p[0] + 8., p[1] - 8.]],
                    symbol: r.symbol,
                    side: r.side,
                    size: number(r.size_mm),
                    length: number(r.length_mm),
                    all_around: r.all_around,
                    field: r.field,
                    tail,
                },
            }
        })
        .collect();
    let rows = records
        .iter()
        .map(|r| {
            vec![
                r.step.map_or("–".into(), |s| s.to_string()),
                r.mark.clone(),
                r.kind.label().into(),
                format!("{} / {}", r.member_names[0], r.member_names[1]),
                symbol_label(r.symbol).into(),
                number(r.size_mm),
                r.process.clone(),
                [r.notes.as_str(), r.step_note.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("; "),
            ]
        })
        .collect();
    out.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Table {
            position_mm: table_mm,
            title: "Weld sequence".into(),
            columns: [
                "Step", "Weld", "Joint", "Members", "Symbol", "Size", "Process", "Notes",
            ]
            .map(String::from)
            .to_vec(),
            rows,
            widths_mm: vec![10., 12., 14., 44., 22., 10., 18., 40.],
            row_height_mm: 6.,
            schedule: None,
        },
    });
    out
}
