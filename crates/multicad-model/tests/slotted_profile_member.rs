//! Native geometry and stable datums for the sourced Line 8 member section.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{LibraryDefinition, ProfileShape, Stock, StockKind},
    model::{
        self, AxisAnchor, BodyOutput, MemberDefinition, MemberEnd, MemberPath, MemberRequest,
        ProfileGrooveSide, length,
    },
    project_file::ProjectFile,
    references::TopologyKind,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, Model};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(session: &Session) -> Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(session.document(), session.revision())
    .unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
fn material_at(solid: &Solid, center: [f64; 3]) -> bool {
    let policy = GeometryPolicy::default();
    let side = 0.01;
    let probe = Solid::box_solid([side; 3], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|c| c - side / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&probe, BooleanOperation::Intersect, policy)
        .unwrap()
        .is_some()
}

#[test]
fn line_8_member_has_four_native_open_grooves_through_core_and_stable_datums() {
    let mut session = Session::new(Document::new("Line 8 profile proof").unwrap()).unwrap();
    let material = Entity::new(
        "Aluminum",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 165, 170, 255],
            section_hatch: None,
        },
    );
    let stock_id = EntityId::new();
    let mut stock = Stock {
        material: material.id,
        specification: StockKind::Profile {
            designation: "item 0.0.026.03 · modeled mounting section".into(),
            length_mm: 6000.,
            width_mm: 40.,
            height_mm: 40.,
            wall_mm: None,
            shape: ProfileShape::ItemLine8_40x40,
        },
    };
    let valid_stock = LibraryDefinition::Stock(stock.clone())
        .entity(stock_id, "Line 8 stock")
        .unwrap();
    let StockKind::Profile { wall_mm, .. } = &mut stock.specification else {
        unreachable!()
    };
    *wall_mm = Some(1.);
    assert!(LibraryDefinition::Stock(stock).validate().is_err());
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Author profile stock".into(),
            edits: vec![Mutation::Insert(material), Mutation::Insert(valid_stock)],
        })
        .unwrap();
    let request = MemberRequest::new(
        "Profile member",
        MemberDefinition {
            output: BodyOutput {
                body: EntityId::nil(),
            },
            stock: stock_id,
            path: MemberPath::Points {
                start: [length(0.), length(0.), length(0.)],
                end: [length(0.), length(0.), length(100.)],
            },
            anchor: [AxisAnchor::Min; 2],
            rotation: None,
            extend_start: None,
            extend_end: None,
        },
    );
    let prepared = model::prepare_members(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&request),
        None,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let project = ProjectFile::new(session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(multicad_core::materials::ITEM_LINE_8_PROFILE_CAPABILITY)
    );
    let native = compile(&session);
    let solid = native.solid(request.feature).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    for (actual, expected) in bounds
        .min
        .into_iter()
        .chain(bounds.max)
        .zip([0., 0., 0., 40., 40., 100.])
    {
        near(actual, expected);
    }
    // The chosen straight-flank section has four open voids. Each removes
    // 141.625 mm². The four R4 corners and Ø6.8 through core remove the rest.
    let area = 1600.
        - 4. * 141.625
        - (4. - std::f64::consts::PI) * 16.
        - std::f64::consts::PI * 3.4_f64.powi(2);
    near(solid.volume_mm3().unwrap(), area * 100.);
    let resolved = model::resolve_member(
        session.document(),
        session.parameters(),
        &request.member,
        model::point_path(&request.member.path, session.parameters())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    near(resolved.section.area_mm2(), area);
    for (side, center, outward) in [
        (ProfileGrooveSide::MinX, [0., 20., 25.], [-1., 0., 0.]),
        (ProfileGrooveSide::MaxX, [40., 20., 25.], [1., 0., 0.]),
        (ProfileGrooveSide::MinY, [20., 0., 25.], [0., -1., 0.]),
        (ProfileGrooveSide::MaxY, [20., 40., 25.], [0., 1., 0.]),
    ] {
        let frame = resolved.groove_frame(side, 25.).unwrap();
        assert_eq!(frame.origin_mm, center);
        assert_eq!(frame.u, [0., 0., 1.]);
        assert_eq!(frame.normal(), outward);
        let probe = |cross: f64, depth: f64| {
            std::array::from_fn(|i| center[i] + cross * frame.v[i] - depth * outward[i])
        };
        assert!(!material_at(solid, probe(0., 0.2)), "open mouth {side:?}");
        assert!(material_at(solid, probe(4.1, 0.2)), "8 mm mouth {side:?}");
        assert!(
            material_at(solid, probe(7., 4.4)),
            "front of retention lip {side:?}"
        );
        assert!(
            !material_at(solid, probe(7., 4.6)),
            "under the lip {side:?}"
        );
        assert!(!material_at(solid, probe(0., 12.2)), "open floor {side:?}");
        assert!(
            material_at(solid, probe(0., 12.3)),
            "12.25 mm groove floor {side:?}"
        );
        let topology = &native.snapshot().shapes[&request.feature].topology;
        for role in [
            "land_before",
            "retention_before",
            "floor",
            "retention_after",
            "land_after",
        ] {
            let tag = format!("member.groove.{}.{role}", side.token());
            assert_eq!(ProfileGrooveSide::from_member_role(&tag), Some(side));
            assert!(
                topology
                    .elements
                    .iter()
                    .any(|e| e.key.kind == TopologyKind::Face
                        && e.origins
                            .iter()
                            .any(|o| o.source == request.feature && o.role == tag)),
                "missing {tag}"
            );
        }
    }
    for z in [0.02, 50., 99.98] {
        assert!(!material_at(solid, [23.3, 20., z]));
        assert!(material_at(solid, [23.5, 20., z]));
    }
    assert!(!material_at(solid, [1., 1., 50.]));
    assert!(material_at(solid, [1.5, 1.5, 50.]));
    assert!(
        resolved
            .groove_frame(ProfileGrooveSide::MinY, 101.)
            .is_err()
    );
    assert_eq!(
        resolved
            .slotted_end_frame(MemberEnd::Start)
            .unwrap()
            .normal(),
        [0., 0., -1.]
    );
    assert_eq!(
        resolved
            .slotted_end_frame(MemberEnd::End)
            .unwrap()
            .origin_mm,
        [20., 20., 100.]
    );

    let mut edit = request.member.clone();
    edit.extend_start = Some(length(5.));
    edit.extend_end = Some(length(15.));
    let prepared = model::prepare_member_edit(
        &session,
        session.revision(),
        actor(),
        request.feature,
        &edit,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let updated = compile(&session);
    let current = updated.solid(request.feature).unwrap();
    near(current.volume_mm3().unwrap(), area * 120.);
    near(current.bounds().unwrap().min[2], -5.);
    near(current.bounds().unwrap().max[2], 115.);
    let resolved = model::resolve_member(
        session.document(),
        session.parameters(),
        &edit,
        model::point_path(&edit.path, session.parameters())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        resolved
            .groove_frame(ProfileGrooveSide::MinY, 25.)
            .unwrap()
            .origin_mm,
        [20., 0., 20.]
    );
    assert_eq!(
        resolved
            .slotted_end_frame(MemberEnd::End)
            .unwrap()
            .origin_mm,
        [20., 20., 115.]
    );
}
