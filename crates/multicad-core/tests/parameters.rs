use multicad_core::{document::*, parameters::*};
use serde_json::json;
fn parameter(name: &str, expression: &str, unit: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: unit.into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn add(doc: &mut Document, e: &Entity) {
    doc.insert(e.clone()).unwrap();
}
#[test]
fn units_expressions_roundtrip_and_materialize_feature_inputs_from_source() {
    let mut d = Document::new("Cabinet").unwrap();
    let width = parameter("width", "600", "mm");
    let stock = parameter("stock", "18 mm", "mm");
    let gap = parameter("rail_gap", "1.25 cm", "mm");
    let drawer = parameter("drawer_width", "width - 2 * stock - rail_gap", "mm");
    let feature = Entity::new(
        "Drawer extrusion",
        EntityData::Feature {
            definition: Definition {
                type_id: "solid.extrude".into(),
                schema_version: 1,
                data: json!({}),
            },
            inputs: vec![Link::new(drawer.id, "width")],
            suppressed: false,
        },
    );
    for e in [&feature, &drawer, &gap, &width, &stock] {
        add(&mut d, e);
    }
    let evaluated = evaluate(&d).unwrap();
    assert!((evaluated.value(drawer.id, Unit::Millimeter).unwrap() - 551.5).abs() < 1e-9);
    assert!(
        (evaluated
            .feature_input(feature.id, "width", Unit::Millimeter)
            .unwrap()
            - 551.5)
            .abs()
            < 1e-9
    );
    assert_eq!(evaluated.values[&width.id].role, ParameterRole::Driving);
    assert_eq!(evaluated.values[&drawer.id].role, ParameterRole::Derived);
    assert!(evaluated.values[&drawer.id].exposed);
    let mut reopened: Document =
        serde_json::from_slice(&serde_json::to_vec(&evaluated.document).unwrap()).unwrap();
    for number in (600..700).chain([600]) {
        let mut e = reopened.get(width.id).unwrap().clone();
        if let EntityData::Parameter { expression, .. } = &mut e.data {
            *expression = number.to_string();
        }
        reopened.replace(e).unwrap();
        let result = evaluate(&reopened).unwrap();
        assert!(
            (result.value(drawer.id, Unit::Millimeter).unwrap() - (number as f64 - 48.5)).abs()
                < 1e-9
        );
        reopened = result.document;
    }
    assert!(
        (evaluate(&reopened)
            .unwrap()
            .value(drawer.id, Unit::Millimeter)
            .unwrap()
            - 551.5)
            .abs()
            < 1e-9
    );
}
#[test]
fn signed_lengths_angles_counts_ratios_and_unit_conversions_are_explicit() {
    let mut d = Document::new("Units").unwrap();
    let fixtures = [
        ("inch", "1 in", "mm", 25.4),
        ("foot", "1 ft", "mm", 304.8),
        ("angle", "180 deg", "rad", std::f64::consts::PI),
        ("bolts", "4", "count", 4.0),
        ("ratio", "50%", "ratio", 0.5),
        ("percent", "50", "%", 50.0),
        ("negative", "-2 cm + 5 mm", "mm", -15.0),
        ("precedence", "=(2 + 3) * 4", "mm", 20.0),
    ];
    let mut ids = Vec::new();
    for (name, expr, unit, want) in fixtures {
        let e = parameter(name, expr, unit);
        add(&mut d, &e);
        ids.push((e.id, Unit::parse(unit).unwrap(), want));
    }
    let result = evaluate(&d).unwrap();
    for (id, unit, want) in ids {
        assert!((result.value(id, unit).unwrap() - want).abs() < 1e-9);
    }
}
#[test]
fn rename_retains_stable_binding_and_new_same_named_entity_does_not_steal_it() {
    let mut d = Document::new("Rename").unwrap();
    let a = parameter("width", "100", "mm");
    let b = parameter("double_width", "width * 2", "mm");
    add(&mut d, &a);
    add(&mut d, &b);
    let mut d = evaluate(&d).unwrap().document;
    let mut renamed = d.get(a.id).unwrap().clone();
    renamed.name = "cabinet_width".into();
    d.replace(renamed).unwrap();
    add(&mut d, &parameter("width", "900", "mm"));
    let result = evaluate(&d).unwrap();
    assert!((result.value(b.id, Unit::Millimeter).unwrap() - 200.0).abs() < 1e-9);
    assert_eq!(
        result.document.get(b.id).unwrap().references()[0].target,
        a.id
    );
    d.remove(a.id).unwrap();
    let error = evaluate(&d).unwrap_err();
    assert_eq!(error.entity, Some(b.id));
    assert_eq!(error.code, ParameterErrorCode::MissingReference);
}
#[test]
fn errors_are_entity_specific_and_original_document_is_preserved() {
    for (expression, unit, code) in [
        ("1 mm + 2 deg", "mm", ParameterErrorCode::Dimension),
        ("1 / 0", "ratio", ParameterErrorCode::NonFinite),
        ("1e999", "mm", ParameterErrorCode::Syntax),
        ("2.5", "count", ParameterErrorCode::Count),
        ("-1", "count", ParameterErrorCode::Count),
        ("unknown + 1", "mm", ParameterErrorCode::MissingReference),
        ("1", "parsec", ParameterErrorCode::Unit),
        ("system(1)", "mm", ParameterErrorCode::Syntax),
        ("1; print(2)", "mm", ParameterErrorCode::Syntax),
    ] {
        let mut d = Document::new("Failure").unwrap();
        let e = parameter("value", expression, unit);
        add(&mut d, &e);
        let before = d.clone();
        let error = evaluate(&d).unwrap_err();
        assert_eq!(error.code, code, "{expression}");
        assert_eq!(error.entity, Some(e.id));
        assert_eq!(d, before);
    }
    let mut d = Document::new("Names").unwrap();
    add(&mut d, &parameter("same", "1", "mm"));
    add(&mut d, &parameter("same", "2", "mm"));
    assert_eq!(
        evaluate(&d).unwrap_err().code,
        ParameterErrorCode::DuplicateName
    );
    let mut d = Document::new("Names").unwrap();
    add(&mut d, &parameter("two words", "1", "mm"));
    assert_eq!(evaluate(&d).unwrap_err().code, ParameterErrorCode::Name);
}
#[test]
fn cycles_and_complexity_are_bounded_and_dependency_edits_are_atomic_candidates() {
    let mut d = Document::new("Cycles").unwrap();
    let a = parameter("a", "b", "mm");
    let b = parameter("b", "a", "mm");
    add(&mut d, &a);
    add(&mut d, &b);
    assert_eq!(evaluate(&d).unwrap_err().code, ParameterErrorCode::Cycle);
    let mut d = Document::new("Rebind").unwrap();
    let mut a = parameter("a", "b", "mm");
    let mut b = parameter("b", "10", "mm");
    add(&mut d, &a);
    add(&mut d, &b);
    let mut normalized = evaluate(&d).unwrap().document;
    if let EntityData::Parameter { expression, .. } = &mut a.data {
        *expression = "20".into();
    }
    if let EntityData::Parameter { expression, .. } = &mut b.data {
        *expression = "a".into();
    }
    // Preserve the old binding on a while changing the two source expressions.
    if let EntityData::Parameter { inputs, .. } = &mut a.data {
        *inputs = normalized.get(a.id).unwrap().references();
    }
    normalized.replace(a).unwrap();
    normalized.replace(b.clone()).unwrap();
    assert!(
        (evaluate(&normalized)
            .unwrap()
            .value(b.id, Unit::Millimeter)
            .unwrap()
            - 20.0)
            .abs()
            < 1e-9
    );
    let mut d = Document::new("Bounded").unwrap();
    let source = format!("{}1{}", "(".repeat(200), ")".repeat(200));
    add(&mut d, &parameter("deep", &source, "mm"));
    assert_eq!(evaluate(&d).unwrap_err().code, ParameterErrorCode::Syntax);
}
#[test]
fn explicit_formulas_dollar_bindings_comparisons_and_exposed_overrides() {
    let mut doc = Document::new("Formula syntax").unwrap();
    let height = parameter("height", "120", "mm");
    let width = parameter("width", "=600 - $height", "mm");
    let check = parameter("fits", "=$width >= 400 mm", "ratio");
    for e in [&height, &width, &check] {
        add(&mut doc, e);
    }
    let result = evaluate(&doc).unwrap();
    assert!((result.value(width.id, Unit::Millimeter).unwrap() - 480.).abs() < 1e-9);
    assert_eq!(result.value(check.id, Unit::Ratio).unwrap(), 1.);
    let mut renamed = result.document.get(height.id).unwrap().clone();
    renamed.name = "height_new".into();
    let renamed = result
        .document
        .edited(&[Mutation::Replace(renamed)])
        .unwrap();
    assert!(evaluate(&renamed).is_ok());
    let edit = override_exposed(&result.document, width.id, 30., Unit::Centimeter).unwrap();
    let overridden = evaluate(&result.document.edited(&[edit]).unwrap()).unwrap();
    assert_eq!(overridden.value(width.id, Unit::Millimeter).unwrap(), 300.);
    assert!(overridden.values[&width.id].exposed);
    assert!(
        overridden
            .document
            .get(width.id)
            .unwrap()
            .references()
            .is_empty()
    );
    assert_eq!(overridden.value(check.id, Unit::Ratio).unwrap(), 0.);
    assert!(override_exposed(&result.document, width.id, f64::NAN, Unit::Millimeter).is_err());
    assert!(override_exposed(&result.document, width.id, 30., Unit::Degree).is_err());
    assert!(validate_input("600 - height").is_err());
    assert!(validate_input("=600 - height").is_err());
    assert!(validate_input("=600 - $height").is_ok());
    assert!(validate_input("-2.5 mm").is_ok());
    assert_eq!(
        display_input("width - 2 * height"),
        "=($width - (2 * $height))"
    );
    for (formula, expected) in [
        ("=$height < 100 mm", 0.),
        ("=$height != 120 mm", 0.),
        ("=$height <= 120 mm", 1.),
        ("=$height == 120 mm", 1.),
    ] {
        let mut e = check.clone();
        if let EntityData::Parameter { expression, .. } = &mut e.data {
            *expression = formula.into();
        }
        let d = result.document.edited(&[Mutation::Replace(e)]).unwrap();
        assert_eq!(
            evaluate(&d).unwrap().value(check.id, Unit::Ratio).unwrap(),
            expected
        );
    }
}
