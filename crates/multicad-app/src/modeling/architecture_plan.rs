//! Reuse native drawing cuts/symbols on the owning model thread. No drawing
//! book, second building document, or geometry-kernel handle reaches the UI.
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::{DrawingView, Scale, ViewEvaluation, ViewKind, ViewSource},
    model,
    references::ReferenceFrame,
};
use multicad_model::{DrawingProviders, Model};
use multicad_ui::viewport::ArchitecturePlan;

pub(super) fn build(document: &Document, model: &Model) -> Result<ArchitecturePlan, String> {
    let snapshot = model.snapshot();
    let sources = snapshot
        .bodies
        .keys()
        .filter(|id| match document.get(**id).map(|entity| &entity.data) {
            Some(EntityData::Part { features, .. }) => features.iter().any(|feature| {
                model::read_wall(document, *feature).is_ok()
                    || model::read_stair(document, *feature).is_ok()
            }),
            _ => false,
        })
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if sources.is_empty() {
        return Ok(ArchitecturePlan {
            source_hash: snapshot.stamp.source_hash,
            sources,
            vectors: vec![],
        });
    }
    let view = DrawingView {
        id: EntityId::new(),
        name: "Model Architecture plan".into(),
        source: ViewSource::Model {
            entities: sources.iter().copied().collect(),
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [0.; 2],
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    };
    let evaluation = model
        .drawing_view(
            document,
            snapshot.stamp.revision,
            &view,
            &DrawingProviders::default(),
        )
        .map_err(|e| e.to_string())?;
    match evaluation {
        ViewEvaluation::Ready { vectors, .. } => Ok(ArchitecturePlan {
            source_hash: snapshot.stamp.source_hash,
            sources,
            vectors,
        }),
        _ => Err("The native Architecture plan did not produce current geometry".into()),
    }
}
