use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Placement},
    equipment::{self, Kind, MountClass},
    symbols::{self, SymbolPlaceRequest},
};
use multicad_core::{drawings::*, references::ReferenceFrame};
use multicad_model::Compiler;
use multicad_model::DrawingProviders;

#[test]
fn every_generic_equipment_family_compiles_to_its_declared_native_envelope() {
    let actor = Actor::local(Channel::Mcp);
    for kind in Kind::ALL {
        let definition = equipment::generic_definition(kind).unwrap();
        let mut session = Session::new(Document::new("Equipment native fixture").unwrap()).unwrap();
        let imported = symbols::prepare_import(&session, 0, actor.clone(), &definition).unwrap();
        let id = imported.definition;
        session.commit(imported.prepared).unwrap();
        let request = SymbolPlaceRequest::new(
            session.document(),
            id,
            "Equipment",
            Default::default(),
            Placement::default(),
        )
        .unwrap();
        session
            .commit(
                symbols::prepare_place(&session, session.revision(), actor.clone(), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let model = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(session.document(), session.revision())
        .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        let bounds = model.snapshot().occurrences[&request.instance].bounds;
        let preset = kind.preset();
        let [w, d, h] = preset.size_mm;
        let z = match preset.mount {
            MountClass::WallCenter => -h / 2.,
            MountClass::CeilingTop | MountClass::CounterTop => -h,
            _ => 0.,
        };
        let y = if matches!(
            preset.mount,
            MountClass::WallCenter | MountClass::WallBottom
        ) {
            -d
        } else {
            -d / 2.
        };
        for (actual, expected) in bounds
            .min
            .into_iter()
            .zip([-w / 2., y, z])
            .chain(bounds.max.into_iter().zip([w / 2., y + d, z + h]))
        {
            assert!(
                (actual - expected).abs() < 1e-4,
                "{kind:?}: native bound {actual} vs {expected}"
            );
        }
        assert!(!model.snapshot().leaf_occurrences.is_empty(), "{kind:?}");
        for denominator in [50, 100] {
            let view = DrawingView {
                id: multicad_core::document::EntityId::new(),
                name: "Equipment plan".into(),
                source: ViewSource::Model {
                    entities: vec![request.instance],
                },
                frame: ReferenceFrame {
                    origin_mm: [0., 0., 1100.],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
                kind: ViewKind::Projection,
                position_mm: [100., 100.],
                scale: Scale {
                    numerator: 1,
                    denominator,
                },
                hidden_lines: false,
                tangent_edges: false,
                paper_chord_tolerance_mm: 0.01,
                guide: None,
                equipment_categories: None,
            };
            let result = model
                .drawing_view(
                    session.document(),
                    session.revision(),
                    &view,
                    &DrawingProviders::default(),
                )
                .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            let ViewEvaluation::Ready { vectors, .. } = result else {
                panic!("{kind:?}: plan not ready")
            };
            assert!(
                vectors.iter().any(
                    |v| matches!(v.curve, VectorCurve::Line {a,b} if (a[0]-b[0]).abs() > w - 1e-5)
                ),
                "{kind:?}: missing true-size footprint"
            );
            if kind == Kind::PowerOutletTypeF {
                assert!(vectors.iter().any(|v| matches!(v.curve, VectorCurve::Ellipse {major_mm,..} if (major_mm * 2. / denominator as f64 - 3.5).abs() < 1e-7)), "fixed-paper electrical mark");
            }
        }
    }
}
