use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::{self, BoundExpression},
};

fn fixture() -> (Document, FenceConstruction) {
    let material = Entity::new(
        "Timber",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [150, 110, 70, 255],
            section_hatch: None,
        },
    );
    let document = Document::new("Fence source")
        .unwrap()
        .edited(&[Mutation::Insert(material.clone())])
        .unwrap();
    let infill = |name: &str, bottom, height, normal_offset, construction| FenceInfill {
        id: EntityId::new(),
        name: name.into(),
        material: material.id,
        bottom: length(bottom),
        height: length(height),
        normal_offset: length(normal_offset),
        construction,
    };
    (
        document,
        FenceConstruction {
            height: length(1800.),
            post: FencePostProfile::Rectangular {
                width: length(90.),
                depth: length(90.),
            },
            post_material: material.id,
            preferred_post_spacing: length(2000.),
            layout: FenceBayLayout::ShortLast,
            infills: vec![
                infill(
                    "Panel",
                    100.,
                    1600.,
                    0.,
                    FenceInfillKind::Panel {
                        thickness: length(18.),
                        width: FencePanelWidth::Purchased {
                            width: length(1910.),
                            short_bay: FenceShortPanelPolicy::Reject,
                        },
                    },
                ),
                infill(
                    "Rail",
                    100.,
                    60.,
                    45.,
                    FenceInfillKind::Rail { depth: length(40.) },
                ),
                infill(
                    "Pickets",
                    100.,
                    1600.,
                    -20.,
                    FenceInfillKind::Pickets {
                        width: length(70.),
                        depth: length(20.),
                        maximum_gap: length(100.),
                    },
                ),
            ],
        },
    )
}
fn resolve(
    document: &Document,
    construction: &FenceConstruction,
) -> Result<ResolvedFenceConstruction, multicad_core::document::DocumentError> {
    construction.resolve(document, &parameters::evaluate(document).unwrap())
}

#[test]
fn ordered_recipes_keep_distinct_identities_materials_and_exact_dimensions() {
    let (document, construction) = fixture();
    let result = resolve(&document, &construction).unwrap();
    assert_eq!(result.height_mm, 1800.);
    assert_eq!(result.preferred_post_spacing_mm, 2000.);
    assert_eq!(result.infills.len(), 3);
    assert_eq!(result.infills[2].normal_offset_mm, -20.);
    assert!(matches!(
        result.infills[0].construction,
        ResolvedFenceInfillKind::Panel {
            thickness_mm: 18.,
            ..
        }
    ));
    assert!(matches!(
        result.infills[1].construction,
        ResolvedFenceInfillKind::Rail { depth_mm: 40. }
    ));
    assert!(matches!(
        result.infills[2].construction,
        ResolvedFenceInfillKind::Pickets {
            width_mm: 70.,
            depth_mm: 20.,
            maximum_gap_mm: 100.
        }
    ));
    let wire = serde_json::to_value(&construction).unwrap();
    assert!(wire.get("path").is_none());
    assert_eq!(
        serde_json::from_value::<FenceConstruction>(wire).unwrap(),
        construction
    );
    let original: Vec<_> = construction.infills.iter().map(|i| i.id).collect();
    let mut reordered = construction.clone();
    reordered.infills.reverse();
    let ids: Vec<_> = resolve(&document, &reordered)
        .unwrap()
        .infills
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ids, original.into_iter().rev().collect::<Vec<_>>());
    assert_eq!(construction.inputs().len(), 4);
    assert!(
        construction
            .inputs()
            .iter()
            .all(|l| l.target == construction.post_material)
    );
}

#[test]
fn purchased_panel_cuts_are_explicit_and_no_policy_stretches_a_panel() {
    let reject = ResolvedFencePanelWidth::Purchased {
        width_mm: 1910.,
        short_bay: FenceShortPanelPolicy::Reject,
    };
    let cut = ResolvedFencePanelWidth::Purchased {
        width_mm: 1910.,
        short_bay: FenceShortPanelPolicy::Cut,
    };
    assert_eq!(reject.width_for_bay(1910.).unwrap(), 1910.);
    assert!(reject.width_for_bay(910.).is_err());
    assert_eq!(cut.width_for_bay(910.).unwrap(), 910.);
    assert_eq!(cut.width_for_bay(1910.).unwrap(), 1910.);
    assert!(cut.width_for_bay(1911.).is_err());
    assert!(reject.width_for_bay(1911.).is_err());
    assert_eq!(
        ResolvedFencePanelWidth::FitBay
            .width_for_bay(2010.)
            .unwrap(),
        2010.
    );
    for width in [0., -1., f64::NAN, f64::INFINITY, 1e10] {
        assert!(cut.width_for_bay(width).is_err());
        assert!(
            ResolvedFencePanelWidth::FitBay
                .width_for_bay(width)
                .is_err()
        );
    }
}

#[test]
fn post_profiles_and_unit_conversion_do_not_invent_solid_wall_thickness() {
    let (document, mut construction) = fixture();
    construction.post = FencePostProfile::Round {
        diameter: Dimension::Literal {
            value: 9.,
            unit: "cm".into(),
        },
    };
    construction.height = Dimension::Literal {
        value: 1.8,
        unit: "m".into(),
    };
    let result = resolve(&document, &construction).unwrap();
    assert_eq!(
        result.post,
        ResolvedFencePostProfile::Round { diameter_mm: 90. }
    );
    assert_eq!(result.height_mm, 1800.);
    let wire = serde_json::to_value(construction).unwrap();
    assert!(wire.get("thickness").is_none());
}

#[test]
fn bound_post_spacing_retains_parameter_identity_after_rename_and_name_reuse() {
    let (mut document, mut construction) = fixture();
    let parameter = Entity::new(
        "pitch",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "m".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document = document
        .edited(&[Mutation::Insert(parameter.clone())])
        .unwrap();
    construction.preferred_post_spacing = Dimension::Expression {
        expression: BoundExpression::bind(&document, "=$pitch", "mm", None).unwrap(),
    };
    let links = construction.inputs();
    assert!(links.iter().any(|l| l.target == parameter.id));
    assert_eq!(
        resolve(&document, &construction)
            .unwrap()
            .preferred_post_spacing_mm,
        2000.
    );
    let mut renamed = parameter;
    renamed.name = "retained_pitch".into();
    if let EntityData::Parameter { expression, .. } = &mut renamed.data {
        *expression = "1.5".into();
    }
    let replacement = Entity::new(
        "pitch",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "m".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document = document
        .edited(&[Mutation::Replace(renamed), Mutation::Insert(replacement)])
        .unwrap();
    assert_eq!(
        resolve(&document, &construction)
            .unwrap()
            .preferred_post_spacing_mm,
        1500.
    );
    assert_eq!(construction.inputs(), links);
}

#[test]
fn malformed_materials_identities_and_unsupported_source_fields_are_rejected() {
    let (document, construction) = fixture();
    let mut bad = construction.clone();
    bad.infills[1].id = bad.infills[0].id;
    assert!(bad.validate().is_err());
    bad = construction.clone();
    bad.infills[0].id = EntityId::nil();
    assert!(bad.validate().is_err());
    bad = construction.clone();
    bad.infills[0].name = " ".into();
    assert!(bad.validate().is_err());
    bad = construction.clone();
    bad.infills[0].material = document.id();
    assert!(resolve(&document, &bad).is_err());
    bad = construction.clone();
    bad.post_material = EntityId::new();
    assert!(resolve(&document, &bad).is_err());
    bad = construction.clone();
    bad.infills.clear();
    assert!(bad.validate().is_err());
    let mut wire = serde_json::to_value(&construction).unwrap();
    wire["terrain_following"] = serde_json::json!(true);
    assert!(serde_json::from_value::<FenceConstruction>(wire).is_err());
    let mut wire = serde_json::to_value(&construction).unwrap();
    wire["infills"][0]["construction"]["width"]["short_bay"] = serde_json::json!("stretch");
    assert!(serde_json::from_value::<FenceConstruction>(wire).is_err());
}

#[test]
fn invalid_dimensions_and_infill_above_posts_fail_without_clamping() {
    let (document, construction) = fixture();
    for value in [0., -1., f64::NAN, f64::INFINITY, 1e10] {
        let mut bad = construction.clone();
        bad.preferred_post_spacing = length(value);
        assert!(resolve(&document, &bad).is_err());
        let mut bad = construction.clone();
        bad.infills[0].height = length(value);
        assert!(resolve(&document, &bad).is_err());
    }
    let mut bad = construction.clone();
    bad.infills[0].bottom = length(-1.);
    assert!(resolve(&document, &bad).is_err());
    let mut bad = construction.clone();
    bad.infills[0].height = length(1701.);
    assert!(
        resolve(&document, &bad)
            .unwrap_err()
            .to_string()
            .contains("above")
    );
    let mut bad = construction.clone();
    bad.height = Dimension::Literal {
        value: 1800.,
        unit: "deg".into(),
    };
    assert!(resolve(&document, &bad).is_err());
    let mut valid = construction;
    valid.infills[0].bottom = length(0.);
    valid.infills[0].height = length(1800.);
    assert_eq!(resolve(&document, &valid).unwrap().infills[0].bottom_mm, 0.);
}
