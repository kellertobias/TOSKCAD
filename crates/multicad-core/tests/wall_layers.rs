use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{WallLayer, WallLayerKind, WallLayerReference, WallLayerSide, WallLayerStack, length},
    parameters::{self, BoundExpression},
};

fn fixture() -> (Document, WallLayerStack) {
    let material = Entity::new(
        "Brick / finish",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 100, 80, 255],
            section_hatch: None,
        },
    );
    let document = Document::new("Layer construction")
        .unwrap()
        .edited(&[Mutation::Insert(material.clone())])
        .unwrap();
    let mut layers = vec![WallLayer {
        id: EntityId::new(),
        name: "Exterior paint".into(),
        role: None,
        construction: WallLayerKind::Coating {
            material: material.id,
        },
    }];
    for (name, width) in [
        ("Insulation", 80.),
        ("Brick", 120.),
        ("Second insulation", 40.),
        ("Second brick", 100.),
        ("Interior lining", 12.5),
    ] {
        layers.push(WallLayer {
            id: EntityId::new(),
            name: name.into(),
            role: Some("User role".into()),
            construction: WallLayerKind::Solid {
                thickness: length(width),
                material: material.id,
            },
        });
    }
    (document, WallLayerStack { layers })
}
fn resolved(
    document: &Document,
    stack: &WallLayerStack,
) -> multicad_core::model::ResolvedWallLayerStack {
    stack
        .resolve(document, &parameters::evaluate(document).unwrap())
        .unwrap()
}

#[test]
fn requested_layer_order_keeps_repeated_materials_and_has_no_independent_total() {
    let (document, stack) = fixture();
    let result = resolved(&document, &stack);
    assert_eq!(result.thickness_mm, 352.5);
    assert_eq!(result.layers.len(), 6);
    assert_eq!(result.layers[0].thickness_mm, 0.);
    assert!(!result.layers[0].solid);
    let encoded = serde_json::to_value(&stack).unwrap();
    assert!(encoded.get("thickness").is_none());
    assert!(encoded.get("total_thickness").is_none());
    assert_eq!(
        serde_json::from_value::<WallLayerStack>(encoded.clone()).unwrap(),
        stack
    );
    let mut conflicting = encoded;
    conflicting["thickness"] = serde_json::json!(400.);
    assert!(serde_json::from_value::<WallLayerStack>(conflicting).is_err());
    assert_eq!(
        stack.inputs().len(),
        6,
        "Repeated materials retain per-layer canonical links"
    );
    assert!(
        stack
            .inputs()
            .windows(2)
            .all(|w| w[0].target == w[1].target && w[0].role != w[1].role)
    );
}

#[test]
fn cavities_occupy_real_width_without_acquiring_a_material_or_solid() {
    let (document, mut stack) = fixture();
    stack.layers.insert(
        3,
        WallLayer {
            id: EntityId::new(),
            name: "Air cavity".into(),
            role: None,
            construction: WallLayerKind::Cavity {
                thickness: length(30.),
            },
        },
    );
    let result = resolved(&document, &stack);
    assert_eq!(result.thickness_mm, 382.5);
    assert_eq!(result.layers[3].thickness_mm, 30.);
    assert_eq!(result.layers[3].material, None);
    assert!(!result.layers[3].solid);
    let mut encoded = serde_json::to_value(&stack).unwrap();
    encoded["layers"][3]["construction"]["material"] = serde_json::json!(EntityId::new());
    assert!(serde_json::from_value::<WallLayerStack>(encoded).is_err());
}

#[test]
fn reference_boundaries_and_flip_preserve_contiguous_physical_bands_and_layer_order() {
    let (document, stack) = fixture();
    let result = resolved(&document, &stack);
    let reference = WallLayerReference::Boundary {
        layer: stack.layers[2].id,
        side: WallLayerSide::Outside,
    };
    let original = result.bands(reference, false).unwrap();
    let flipped = result.bands(reference, true).unwrap();
    assert_eq!(original[2].outside_mm, 0.);
    assert_eq!(original[0].outside_mm, 80.);
    for i in 0..original.len() {
        assert_eq!(original[i].layer, stack.layers[i].id);
        assert_eq!(flipped[i].layer, original[i].layer);
        assert_eq!(flipped[i].outside_mm, -original[i].outside_mm);
        assert_eq!(flipped[i].inside_mm, -original[i].inside_mm);
        assert_eq!(
            (original[i].outside_mm - original[i].inside_mm).abs(),
            result.layers[i].thickness_mm
        );
        if i > 0 {
            assert_eq!(original[i - 1].inside_mm, original[i].outside_mm);
        }
    }
    for (reference, outside, inside) in [
        (WallLayerReference::Outside, 0., -352.5),
        (WallLayerReference::Center, 176.25, -176.25),
        (WallLayerReference::Inside, 352.5, 0.),
    ] {
        let bands = result.bands(reference, false).unwrap();
        assert_eq!(bands.first().unwrap().outside_mm, outside);
        assert_eq!(bands.last().unwrap().inside_mm, inside);
    }
    let inside = result
        .bands(
            WallLayerReference::Boundary {
                layer: stack.layers[2].id,
                side: WallLayerSide::Inside,
            },
            false,
        )
        .unwrap();
    assert_eq!(inside[2].inside_mm, 0.);
}

#[test]
fn reorder_duplicate_and_remove_preserve_identity_or_require_reference_repair() {
    let (document, mut stack) = fixture();
    let anchor = stack.layers[2].id;
    let reference = WallLayerReference::Boundary {
        layer: anchor,
        side: WallLayerSide::Outside,
    };
    let before = resolved(&document, &stack);
    stack.layers.swap(1, 2);
    assert_eq!(
        resolved(&document, &stack).bands(reference, false).unwrap()[1].outside_mm,
        0.
    );
    assert_eq!(
        resolved(&document, &stack).thickness_mm,
        before.thickness_mm
    );
    let mut duplicate = stack.layers[1].clone();
    duplicate.id = EntityId::new();
    stack.layers.push(duplicate);
    assert_eq!(resolved(&document, &stack).thickness_mm, 472.5);
    stack.layers.retain(|l| l.id != anchor);
    assert!(
        resolved(&document, &stack)
            .bands(reference, false)
            .unwrap_err()
            .to_string()
            .contains("removed")
    );
}

#[test]
fn parameter_bound_thickness_follows_identity_after_rename_and_converts_units() {
    let (mut document, mut stack) = fixture();
    let parameter = Entity::new(
        "insulation",
        EntityData::Parameter {
            expression: "8".into(),
            unit: "cm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document = document
        .edited(&[Mutation::Insert(parameter.clone())])
        .unwrap();
    if let WallLayerKind::Solid { thickness, .. } = &mut stack.layers[1].construction {
        *thickness = Dimension::Expression {
            expression: BoundExpression::bind(&document, "=$insulation", "mm", None).unwrap(),
        };
    }
    assert_eq!(resolved(&document, &stack).thickness_mm, 352.5);
    let mut renamed = parameter.clone();
    renamed.name = "renamed_insulation".into();
    if let EntityData::Parameter { expression, .. } = &mut renamed.data {
        *expression = "10".into();
    }
    document = document.edited(&[Mutation::Replace(renamed)]).unwrap();
    assert_eq!(resolved(&document, &stack).thickness_mm, 372.5);
    assert!(stack.inputs().iter().any(|l| l.target == parameter.id));
}

#[test]
fn invalid_identities_units_materials_and_nonpositive_finite_layers_are_rejected() {
    let (document, stack) = fixture();
    let mut invalid = stack.clone();
    invalid.layers[2].id = invalid.layers[1].id;
    assert!(invalid.validate().is_err());
    invalid = stack.clone();
    invalid.layers[0].id = EntityId::nil();
    assert!(invalid.validate().is_err());
    invalid = stack.clone();
    invalid.layers[1].name = " ".into();
    assert!(invalid.validate().is_err());
    for width in [0., -1., f64::INFINITY, 1e9, 1e10] {
        invalid = stack.clone();
        if let WallLayerKind::Solid { thickness, .. } = &mut invalid.layers[1].construction {
            *thickness = length(width);
        }
        assert!(
            invalid
                .resolve(&document, &parameters::evaluate(&document).unwrap())
                .is_err()
        );
    }
    invalid = stack.clone();
    if let WallLayerKind::Solid { thickness, .. } = &mut invalid.layers[1].construction {
        *thickness = Dimension::Literal {
            value: 1.,
            unit: "deg".into(),
        };
    }
    assert!(invalid.validate().is_err());
    invalid = stack.clone();
    invalid.layers[1].construction = WallLayerKind::Solid {
        thickness: length(10.),
        material: document.id(),
    };
    assert!(
        invalid
            .resolve(&document, &parameters::evaluate(&document).unwrap())
            .is_err()
    );
    invalid = stack.clone();
    invalid
        .layers
        .retain(|l| !matches!(l.construction, WallLayerKind::Solid { .. }));
    assert!(invalid.validate().is_err());
}

#[test]
fn finite_finishes_are_not_inflated_and_coatings_do_not_accept_a_solid_thickness() {
    let (document, mut stack) = fixture();
    if let WallLayerKind::Solid { thickness, .. } = &mut stack.layers[1].construction {
        *thickness = length(0.00001);
    }
    assert_eq!(resolved(&document, &stack).layers[1].thickness_mm, 0.00001);
    let mut encoded = serde_json::to_value(&stack).unwrap();
    encoded["layers"][0]["construction"]["thickness"] = serde_json::to_value(length(10.)).unwrap();
    assert!(serde_json::from_value::<WallLayerStack>(encoded).is_err());
}
