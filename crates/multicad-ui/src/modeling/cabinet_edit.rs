//! Cabinet-level editing (CAD-163): find the carcass behind a selected cabinet
//! group, carcass or carcass panel, and draw display-only envelope and shelf
//! dimensions for the Panel / carcass form. The guide never becomes
//! manufacturing geometry; Preview and Apply still go through the native
//! evaluator and the canonical carcass transaction.
use crate::{camera::Camera, egui};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, CarcassDefinition, PanelRole, PanelSource},
    parameters,
};

/// The carcass a selection belongs to, and the panel feature that opens its
/// saved controls through the ordinary feature-edit path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarcassTarget {
    pub carcass: EntityId,
    pub feature: EntityId,
    pub name: String,
}

fn carcass_of_panel(document: &Document, feature: EntityId) -> Option<EntityId> {
    match model::read_panel(document, feature).ok()?.source {
        PanelSource::Carcass { carcass, .. } => Some(carcass),
        _ => None,
    }
}

/// Resolve a cabinet group, carcass, carcass panel Part or panel feature to
/// its carcass. Rectangle and sketch-region panels are not cabinets.
pub fn carcass_target(document: &Document, id: EntityId) -> Option<CarcassTarget> {
    let carcass = if model::read_carcass(document, id).is_ok() {
        Some(id)
    } else {
        match &document.get(id)?.data {
            EntityData::Group { .. } => model::carcasses(document)
                .find(|e| model::read_carcass(document, e.id).is_ok_and(|c| c.group == Some(id)))
                .map(|e| e.id),
            EntityData::Part { .. } => model::panel_feature(document, id)
                .and_then(|feature| carcass_of_panel(document, feature)),
            EntityData::Feature { .. } => carcass_of_panel(document, id),
            _ => None,
        }
    }?;
    let (_, feature, _) = model::carcass_panels(document, carcass)
        .ok()?
        .into_iter()
        .next()?;
    Some(CarcassTarget {
        carcass,
        feature,
        name: document.get(carcass)?.name.clone(),
    })
}

/// How a carcass edit treats the existing panels, shown before Preview so a
/// removed shelf or back is never silent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleChanges {
    /// Panels whose role remains; they keep their Part and feature identity.
    pub kept: usize,
    pub added: Vec<String>,
    /// Removed panels by name, with the joint or machining that blocks the
    /// removal until it is deleted.
    pub removed: Vec<(String, Option<String>)>,
}
impl RoleChanges {
    pub fn blocked(&self) -> bool {
        self.removed.iter().any(|(_, b)| b.is_some())
    }
}
pub fn role_changes(
    document: &Document,
    carcass: EntityId,
    definition: &CarcassDefinition,
) -> RoleChanges {
    let existing = model::carcass_panels(document, carcass).unwrap_or_default();
    let roles = definition.roles();
    let name = |id: EntityId| document.get(id).map_or(String::new(), |e| e.name.clone());
    let mut changes = RoleChanges::default();
    for (role, _, body) in &existing {
        if roles.contains(role) {
            changes.kept += 1;
            continue;
        }
        let blocker = model::panel_joints_of(document, *body)
            .into_iter()
            .find(|id| {
                !model::read_panel_joint(document, *id)
                    .is_ok_and(|j| j.generated_by == Some(carcass))
            })
            .or_else(|| {
                model::machinings_of(document, *body)
                    .into_iter()
                    .find(|id| {
                        !model::read_machining(document, *id)
                            .is_ok_and(|m| m.generated_by == Some(carcass))
                    })
            })
            .map(name);
        changes.removed.push((name(*body), blocker));
    }
    for role in roles {
        if !existing.iter().any(|(r, ..)| *r == role) {
            changes.added.push(role.label());
        }
    }
    changes
}

/// One linear dimension: measured points `a`..`b`, drawn shifted by `offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct GuideDimension {
    pub a: [f64; 3],
    pub b: [f64; 3],
    pub offset: [f64; 3],
    pub label: String,
}
/// World-space envelope wireframe, overall dimensions and shelf outlines.
#[derive(Clone, Debug, PartialEq)]
pub struct CabinetGuide {
    pub edges: Vec<[[f64; 3]; 2]>,
    pub dimensions: Vec<GuideDimension>,
    pub shelves: Vec<GuideShelf>,
    pub notes: Vec<([f64; 3], String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct GuideShelf {
    pub corners: [[f64; 3]; 4],
    pub label: String,
    pub adjustable: Option<bool>,
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
fn mul(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|v| v * k)
}
pub fn format_mm(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{v:.0} mm")
    } else {
        format!("{v:.1} mm")
    }
}

/// Plan the carcass exactly as Apply would and describe its envelope. An
/// invalid draft (no interior, shelves that do not fit) yields the planner's
/// own refusal so the Inspector and the guide never disagree.
pub fn carcass_guide(
    document: &Document,
    definition: &CarcassDefinition,
) -> Result<CabinetGuide, String> {
    let p = parameters::evaluate(document).map_err(|e| e.to_string())?;
    let r = model::resolve_carcass(document, &p, definition).map_err(|e| e.to_string())?;
    let [w, h, d] = r.size_mm;
    let (s, c) = r.rotation_rad.sin_cos();
    let (x, y, z) = ([c, s, 0.], [-s, c, 0.], [0., 0., 1.]);
    let at = |u: f64, v: f64, e: f64| add(add(add(r.origin, mul(x, u)), mul(y, v)), mul(z, e));
    let mut edges = vec![];
    for (u, v) in [(0., 0.), (w, 0.), (w, d), (0., d)] {
        edges.push([at(u, v, 0.), at(u, v, h)]);
    }
    for e in [0., h] {
        edges.push([at(0., 0., e), at(w, 0., e)]);
        edges.push([at(w, 0., e), at(w, d, e)]);
        edges.push([at(w, d, e), at(0., d, e)]);
        edges.push([at(0., d, e), at(0., 0., e)]);
    }
    let gap = w.max(h).max(d) * 0.08;
    let mut dimensions = vec![
        GuideDimension {
            a: at(0., 0., 0.),
            b: at(w, 0., 0.),
            offset: mul(z, -gap),
            label: format!("Width {}", format_mm(w)),
        },
        GuideDimension {
            a: at(0., 0., 0.),
            b: at(0., 0., h),
            offset: mul(x, -gap),
            label: format!(
                "{} {}",
                if definition.cabinet.is_some() {
                    "Floor-to-top"
                } else {
                    "Height"
                },
                format_mm(h)
            ),
        },
        GuideDimension {
            a: at(w, 0., 0.),
            b: at(w, d, 0.),
            offset: mul(x, gap),
            label: format!("Depth {}", format_mm(d)),
        },
    ];
    let mut notes = vec![];
    if let Some(c) = &definition.cabinet {
        if let Some(bottom) = r
            .panels
            .iter()
            .find(|panel| panel.role == PanelRole::Bottom)
        {
            let b = bottom.panel;
            let corners = [
                b.origin,
                add(b.origin, mul(b.u, b.length_mm)),
                add(add(b.origin, mul(b.u, b.length_mm)), mul(b.v, b.width_mm)),
                add(b.origin, mul(b.v, b.width_mm)),
            ];
            for i in 0..4 {
                edges.push([corners[i], corners[(i + 1) % 4]]);
            }
        }
        let toe = c
            .toe_clearance
            .value_in(multicad_core::parameters::Unit::Millimeter, Some(&p))?;
        dimensions.push(GuideDimension {
            a: at(0., 0., 0.),
            b: at(0., 0., toe),
            offset: mul(x, -gap * 0.45),
            label: format!("Toe clearance {} · bottom underside", format_mm(toe)),
        });
        notes.push((at(0., 0., 0.), "Local floor datum · 0 mm".into()));
        notes.push((
            at(w / 2., 0., h),
            format!(
                "Fixed carcass · {}",
                match c.joinery {
                    model::CabinetJoinery::Dowels(_) => "Dowels",
                    model::CabinetJoinery::Domino(_) => "Domino",
                }
            ),
        ));
    }
    let shelves = r
        .panels
        .iter()
        .filter_map(|panel| {
            let PanelRole::Shelf { index } = panel.role else {
                return None;
            };
            let b = panel.panel;
            let top = if definition.cabinet.is_some() {
                b.origin
            } else {
                add(b.origin, mul(b.normal(), b.thickness_mm))
            };
            let lu = mul(b.u, b.length_mm);
            let wv = mul(b.v, b.width_mm);
            let height = top[2] - r.origin[2];
            let explicit = definition
                .cabinet
                .as_ref()
                .and_then(|c| c.shelf(index).map(|s| (c, s)));
            let (label, adjustable) = if let Some((c, s)) = explicit {
                let support = match s.support {
                    model::ShelfSupport::Fixed => format!(
                        "Fixed / {}",
                        match c.joinery {
                            model::CabinetJoinery::Dowels(_) => "Dowels",
                            model::CabinetJoinery::Domino(_) => "Domino",
                        }
                    ),
                    model::ShelfSupport::System32 { level } => format!("System 32 L{level}"),
                };
                (
                    format!(
                        "Shelf {} · {support} · underside {} · depth {}",
                        index + 1,
                        format_mm(height),
                        format_mm(b.width_mm)
                    ),
                    Some(matches!(s.support, model::ShelfSupport::System32 { .. })),
                )
            } else {
                (format!("Shelf {} · {}", index + 1, format_mm(height)), None)
            };
            Some(GuideShelf {
                corners: [top, add(top, lu), add(add(top, lu), wv), add(top, wv)],
                label,
                adjustable,
            })
        })
        .collect();
    Ok(CabinetGuide {
        edges,
        dimensions,
        shelves,
        notes,
    })
}

impl CabinetGuide {
    /// Paint the guide over the viewport; returns the number of drawn labels.
    pub fn paint(&self, painter: &egui::Painter, camera: Camera, rect: egui::Rect) -> usize {
        let envelope = egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 190, 255));
        let measure = egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 200, 80));
        let shelf = egui::Stroke::new(1.5, egui::Color32::from_rgb(140, 230, 140));
        let line = |a, b, stroke| {
            if let Some(points) = camera.segment(a, b, rect) {
                painter.line_segment(points, stroke);
            }
        };
        let mut labels = 0;
        let mut label = |at: [f64; 3], text: &str, color| {
            let p = camera.project(at.map(|v| v as f32), rect);
            let pos = egui::pos2(p[0], p[1]);
            if !rect.contains(pos) {
                return;
            }
            let galley =
                painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.), color);
            let r = egui::Rect::from_center_size(pos, galley.size()).expand(3.);
            painter.rect_filled(r, 3., egui::Color32::from_black_alpha(170));
            painter.galley(r.min + egui::vec2(3., 3.), galley, color);
            labels += 1;
        };
        for [a, b] in &self.edges {
            line(*a, *b, envelope);
        }
        for d in &self.dimensions {
            let (a, b) = (add(d.a, d.offset), add(d.b, d.offset));
            line(d.a, add(d.a, mul(d.offset, 1.15)), measure);
            line(d.b, add(d.b, mul(d.offset, 1.15)), measure);
            line(a, b, measure);
            label(mul(add(a, b), 0.5), &d.label, measure.color);
        }
        for guide in &self.shelves {
            let corners = &guide.corners;
            let stroke = match guide.adjustable {
                Some(false) => measure,
                Some(true) => egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 215, 255)),
                None => shelf,
            };
            for i in 0..4 {
                line(corners[i], corners[(i + 1) % 4], stroke);
            }
            label(
                mul(add(corners[0], corners[2]), 0.5),
                &guide.label,
                stroke.color,
            );
        }
        for (at, text) in &self.notes {
            label(*at, text, measure.color);
        }
        labels
    }
}
