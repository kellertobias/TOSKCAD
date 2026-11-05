//! Viewport evidence for one selected Interference/Clearance result (CAD-171).
//! Witness endpoints are the exact native extrema points carried in the
//! report. Occurrence boxes come from display meshes and only locate the pair;
//! they never supply a number. A marker exists only while the viewed Scene is
//! the complete model of the report's own evaluated document/revision/hash.
use crate::{
    camera::Camera,
    egui,
    viewport::{Scene, SectionPlane, Selection},
};
use multicad_core::assembly_inspection::{
    InspectionStamp, InspectionWitness, OccurrencePair, PairRelation, PairReport,
};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) const A_COLOR: egui::Color32 = egui::Color32::from_rgb(244, 180, 65);
pub(crate) const B_COLOR: egui::Color32 = egui::Color32::from_rgb(85, 173, 241);

#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceMarker {
    pub stamp: InspectionStamp,
    pub pair: OccurrencePair,
    pub relation: PairRelation,
    pub minimum_clearance_mm: f64,
    pub overlap_volume_mm3: f64,
    pub tolerance_mm: f64,
    pub required_minimum_mm: Option<f64>,
    /// Exact native closest-point pairs, A side first.
    pub witnesses: Vec<InspectionWitness>,
    /// Display-mesh world boxes of the A and B leaf occurrences.
    pub bounds: [Option<[[f64; 3]; 2]>; 2],
    /// Whether the current view draws each occurrence's canonical root.
    pub displayed: [bool; 2],
}

/// The Scene is the complete native model of exactly this evaluated stamp.
pub fn scene_matches(stamp: &InspectionStamp, scene: &Scene) -> bool {
    scene.is_full_model()
        && scene.stamp().document == stamp.document
        && scene.stamp().revision == stamp.revision
        && scene
            .model_evaluation()
            .is_some_and(|m| m.source_hash == stamp.source_hash)
}

pub fn relation_style(relation: PairRelation) -> (&'static str, egui::Color32) {
    match relation {
        PairRelation::Separated => ("Separated", egui::Color32::from_rgb(120, 214, 150)),
        PairRelation::ContactWithinTolerance => (
            "Contact within tolerance",
            egui::Color32::from_rgb(242, 210, 76),
        ),
        PairRelation::Overlapping => ("Interference", egui::Color32::from_rgb(240, 96, 96)),
    }
}

/// A policy result is separate from the exact native geometric relation.
pub(super) fn clearance_status(
    relation: PairRelation,
    measured: f64,
    required: f64,
) -> (String, egui::Color32) {
    if !required.is_finite() || required < 0. || !measured.is_finite() || measured < 0. {
        return (
            "Unknown: invalid clearance requirement or measurement".into(),
            egui::Color32::LIGHT_RED,
        );
    }
    if relation == PairRelation::Overlapping {
        return ("Interference".into(), relation_style(relation).1);
    }
    if measured < required {
        return (
            format!(
                "Below required clearance by {}",
                crate::preferences::format_mm(required - measured)
            ),
            egui::Color32::from_rgb(242, 160, 76),
        );
    }
    if relation == PairRelation::ContactWithinTolerance {
        return (
            "Contact · meets required clearance".into(),
            relation_style(relation).1,
        );
    }
    (
        "Meets required clearance".into(),
        egui::Color32::from_rgb(120, 214, 150),
    )
}

impl ClearanceMarker {
    pub fn build(report: &PairReport, index: usize, scene: &Scene) -> Result<Self, String> {
        if !scene_matches(&report.stamp, scene) {
            return Err(
                "The viewport shows a different model revision than this result; Preview again"
                    .into(),
            );
        }
        let evaluation = report
            .pairs
            .get(index)
            .ok_or("The selected clearance result no longer exists")?;
        let values = evaluation
            .result
            .as_ref()
            .map_err(|e| format!("No viewport evidence for an unknown result: {e}"))?;
        if values.witnesses.is_empty()
            || values
                .witnesses
                .iter()
                .flat_map(|w| w.a_mm.iter().chain(&w.b_mm))
                .any(|v| !v.is_finite())
        {
            return Err("The result carries no finite native witness".into());
        }
        let pair = evaluation.pair.clone();
        Ok(Self {
            stamp: report.stamp,
            relation: values.relation,
            minimum_clearance_mm: values.minimum_clearance_mm,
            overlap_volume_mm3: values.overlap_volume_mm3,
            tolerance_mm: values.tolerance_mm,
            required_minimum_mm: None,
            witnesses: values.witnesses.clone(),
            bounds: [
                scene.leaf_display_bounds(&pair.a),
                scene.leaf_display_bounds(&pair.b),
            ],
            displayed: [
                scene.displays_instance(pair.a.root),
                scene.displays_instance(pair.b.root),
            ],
            pair,
        })
    }
    /// Highlight both physical occurrences. Nested leaves highlight their
    /// drawn root; the A/B boxes and witnesses identify the exact leaf.
    pub fn highlight(&self) -> Selection {
        Selection::Parts(Arc::new(BTreeSet::from([
            self.pair.a.root,
            self.pair.b.root,
        ])))
    }
    pub fn label(&self) -> String {
        if let Some(required) = self.required_minimum_mm {
            let (status, _) = clearance_status(self.relation, self.minimum_clearance_mm, required);
            let geometry = match self.relation {
                PairRelation::Overlapping => {
                    format!(" · {:.3} mm³ overlap", self.overlap_volume_mm3)
                }
                PairRelation::ContactWithinTolerance => {
                    format!(" · contact tolerance {} mm", self.tolerance_mm)
                }
                PairRelation::Separated => String::new(),
            };
            return format!(
                "{status}{geometry} · measured {:.3} mm · required {:.3} mm",
                self.minimum_clearance_mm, required
            );
        }
        match self.relation {
            PairRelation::Separated => format!("{:.3} mm", self.minimum_clearance_mm),
            PairRelation::ContactWithinTolerance => {
                format!("Contact · ≤ {} mm", self.tolerance_mm)
            }
            PairRelation::Overlapping => {
                format!("Interference · {:.3} mm³ overlap", self.overlap_volume_mm3)
            }
        }
    }
    /// Camera framing for both occurrences and every witness. Camera only.
    pub fn fit_points(&self) -> Vec<[f32; 3]> {
        let mut points: Vec<[f32; 3]> = self
            .witnesses
            .iter()
            .flat_map(|w| [w.a_mm, w.b_mm])
            .map(|p| p.map(|v| v as f32))
            .collect();
        for [low, high] in self.bounds.iter().flatten() {
            for corner in 0..8 {
                points.push(std::array::from_fn(|i| {
                    (if corner & (1 << i) == 0 {
                        low[i]
                    } else {
                        high[i]
                    }) as f32
                }));
            }
        }
        points
    }
    pub fn hidden(&self) -> [bool; 2] {
        self.displayed.map(|shown| !shown)
    }
    pub fn paint(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        camera: Camera,
        reveal: bool,
        section: Option<SectionPlane>,
    ) {
        let (_, relation_color) = self.required_minimum_mm.map_or_else(
            || {
                let (label, color) = relation_style(self.relation);
                (label.to_owned(), color)
            },
            |required| clearance_status(self.relation, self.minimum_clearance_mm, required),
        );
        // Explicit reveal draws both occurrence boxes over occluding geometry;
        // a hidden occurrence is always outlined so the pair stays locatable.
        for (side, bounds) in self.bounds.iter().enumerate() {
            if let Some(bounds) = bounds
                && (reveal || !self.displayed[side])
            {
                paint_box(
                    painter,
                    rect,
                    camera,
                    *bounds,
                    if side == 0 { A_COLOR } else { B_COLOR },
                );
            }
        }
        let project = |p: [f64; 3]| {
            let p = camera.project(p.map(|v| v as f32), rect);
            egui::pos2(p[0], p[1])
        };
        for (index, w) in self.witnesses.iter().enumerate() {
            let primary = index == 0;
            let Some([a, b]) =
                section.map_or(Some([w.a_mm, w.b_mm]), |p| p.clip_segment(w.a_mm, w.b_mm))
            else {
                continue;
            };
            let (pa, pb) = (project(a), project(b));
            painter.line_segment(
                [pa, pb],
                egui::Stroke::new(if primary { 2. } else { 0.8 }, relation_color),
            );
            let radius = if primary { 4. } else { 2.5 };
            if section.is_none_or(|p| p.contains(w.a_mm)) {
                painter.circle_filled(pa, radius, A_COLOR);
            }
            if section.is_none_or(|p| p.contains(w.b_mm)) {
                painter.circle_filled(pb, radius, B_COLOR);
            }
            if self.relation != PairRelation::Separated && primary {
                // Coincident endpoints: a ring keeps contact/overlap visible.
                painter.circle_stroke(pa, 9., egui::Stroke::new(2., relation_color));
                if self.relation == PairRelation::Overlapping {
                    let d = 6.;
                    let stroke = egui::Stroke::new(2., relation_color);
                    painter.line_segment([pa + egui::vec2(-d, -d), pa + egui::vec2(d, d)], stroke);
                    painter.line_segment([pa + egui::vec2(-d, d), pa + egui::vec2(d, -d)], stroke);
                }
            }
        }
        let w = self.witnesses[0];
        let middle = std::array::from_fn(|i| (w.a_mm[i] + w.b_mm[i]) * 0.5);
        if section.is_some_and(|p| !p.contains(middle)) {
            return;
        }
        let text = self.label();
        let anchor = project(middle) + egui::vec2(10., -10.);
        let galley = painter.layout(
            text.clone(),
            egui::FontId::proportional(12.),
            egui::Color32::WHITE,
            (rect.width() - 16.).max(1.),
        );
        let desired =
            egui::Rect::from_min_size(anchor - egui::vec2(0., galley.size().y), galley.size())
                .expand(3.);
        let min = egui::pos2(
            desired.min.x.clamp(
                rect.min.x + 4.,
                (rect.max.x - desired.width() - 4.).max(rect.min.x + 4.),
            ),
            desired.min.y.clamp(
                rect.min.y + 4.,
                (rect.max.y - desired.height() - 4.).max(rect.min.y + 4.),
            ),
        );
        let frame = egui::Rect::from_min_size(min, desired.size());
        painter.rect_filled(frame, 3., egui::Color32::from_black_alpha(190));
        painter.rect_stroke(
            frame,
            3.,
            egui::Stroke::new(1., relation_color),
            egui::StrokeKind::Outside,
        );
        painter.galley(frame.min + egui::vec2(3., 3.), galley, egui::Color32::WHITE);
    }
}

fn paint_box(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: Camera,
    [low, high]: [[f64; 3]; 2],
    color: egui::Color32,
) {
    let corner = |c: usize| -> [f64; 3] {
        std::array::from_fn(|i| if c & (1 << i) == 0 { low[i] } else { high[i] })
    };
    for a in 0..8usize {
        for axis in 0..3 {
            let b = a | (1 << axis);
            if b == a {
                continue;
            }
            if let Some([p, q]) = camera.segment(corner(a), corner(b), rect) {
                painter.extend(egui::Shape::dashed_line(
                    &[p, q],
                    egui::Stroke::new(1.2, color),
                    5.,
                    4.,
                ));
            }
        }
    }
}
