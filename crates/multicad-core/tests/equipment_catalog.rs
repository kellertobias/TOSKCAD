use multicad_core::{
    document::Document,
    equipment::{self, Kind, PlanConvention},
};

#[test]
fn every_generic_family_has_repeatable_self_contained_dimensioned_source() {
    for kind in Kind::ALL {
        let a = equipment::generic_definition(kind).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        let b = equipment::generic_definition(kind).unwrap();
        assert_eq!(a, b, "{kind:?}");
        assert_eq!(a.pin().unwrap(), b.pin().unwrap());
        a.validate().unwrap();
        let template = equipment::template_for(&a.template, a.root)
            .unwrap()
            .unwrap();
        assert_eq!(template.kind, kind);
        assert_eq!(template.convention, PlanConvention::ToskCadGenericV1);
        assert_eq!(a.plan.as_ref().unwrap().sketch, template.footprint);
        for id in [
            template.width_parameter,
            template.depth_parameter,
            template.height_parameter,
        ] {
            assert!(a.template.get(id).is_some());
        }
        let restored: Document =
            serde_json::from_slice(&serde_json::to_vec(&a.template).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, a.template);
    }
}

#[test]
fn junction_box_is_a_distinct_wall_enclosure_with_a_generic_plan_mark() {
    use multicad_core::{
        constraint_sketch::source::Source,
        equipment::{MountClass, PowerConnector},
    };
    let definition = equipment::generic_definition(Kind::JunctionBox).unwrap();
    let template = equipment::template_for(&definition.template, definition.root)
        .unwrap()
        .unwrap();
    assert_eq!(template.kind, Kind::JunctionBox);
    assert_eq!(template.mount, MountClass::WallCenter);
    assert_eq!(template.power_connector, None::<PowerConnector>);
    assert_eq!(template.mark_paper_mm, Some(3.5));
    let multicad_core::document::EntityData::Sketch {
        definition: source, ..
    } = &definition
        .template
        .get(template.mark.unwrap())
        .unwrap()
        .data
    else {
        panic!("authored electrical mark sketch")
    };
    let mark = Source::from_definition(source).unwrap();
    assert!(mark.sketch.texts.iter().any(|text| text.style.text == "JB"));
    for name in ["Junction box enclosure", "Junction box cover"] {
        assert!(
            definition
                .template
                .entities()
                .any(|entity| entity.name == name)
        );
    }
    let socket = equipment::generic_definition(Kind::PowerOutletTypeF).unwrap();
    assert_ne!(definition.symbol, socket.symbol);
    assert_ne!(definition.pin().unwrap(), socket.pin().unwrap());
}
