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
