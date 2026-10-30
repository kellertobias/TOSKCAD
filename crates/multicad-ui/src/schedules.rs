//! Window-side schedule choices. Schedule rows, CSV text and drawing tables
//! are built natively (`multicad_model::schedules`) behind the drawing and
//! export backends; the UI only chooses a schedule and shows the result.
use crate::egui;
use multicad_core::{
    drawings::{AnnotationKind, DrawingStamp},
    schedules::{ScheduleKind, ScheduleSpec},
};

/// A schedule built natively for one source snapshot, as the linked drawing
/// table `Schedule::placed_table` makes from it (placed at the origin).
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleTable {
    pub spec: ScheduleSpec,
    pub revision: u64,
    pub table: AnnotationKind,
}
impl ScheduleTable {
    /// Whether a placed table still shows exactly these cells, as
    /// `Schedule::check_table` decides.
    pub fn matches(&self, placed: &AnnotationKind) -> bool {
        match (&self.table, placed) {
            (
                AnnotationKind::Table { columns, rows, .. },
                AnnotationKind::Table {
                    columns: c,
                    rows: r,
                    ..
                },
            ) => columns == c && rows == r,
            _ => false,
        }
    }
    /// The fresh table at a placed table's position and row height.
    pub fn placed_at(&self, placed: &AnnotationKind) -> AnnotationKind {
        let mut table = self.table.clone();
        if let (
            AnnotationKind::Table {
                position_mm,
                row_height_mm,
                ..
            },
            AnnotationKind::Table {
                position_mm: p,
                row_height_mm: h,
                ..
            },
        ) = (&mut table, placed)
        {
            *position_mm = *p;
            *row_height_mm = *h;
        }
        table
    }
}
/// Every requested schedule for one source snapshot, each built or failed.
pub struct ScheduleReply {
    pub stamp: DrawingStamp,
    pub tables: Vec<(ScheduleSpec, Result<ScheduleTable, String>)>,
}

/// A schedule kind chooser with the BOM inclusion options. Returns whether
/// the choice changed.
pub fn spec_controls(ui: &mut egui::Ui, salt: &str, spec: &mut ScheduleSpec) -> bool {
    let before = *spec;
    crate::controls::inspector_combo(ui, salt)
        .selected_text(spec.kind.label())
        .show_ui(ui, |ui| {
            for kind in ScheduleKind::ALL {
                ui.selectable_value(&mut spec.kind, kind, kind.label());
            }
        });
    if spec.kind == ScheduleKind::Bom {
        ui.checkbox(&mut spec.include_aids, "Include FDM aid Parts")
            .on_hover_text("Aid Parts are removed after printing and left out by default");
        ui.checkbox(
            &mut spec.include_purchased,
            "Include purchased and imported Parts",
        );
    }
    *spec = spec.normalized();
    *spec != before
}
