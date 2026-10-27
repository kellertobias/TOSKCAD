use multicad_core::{document::EntityId, sketch_point::PointPositionRequest};
#[test]
fn point_request_has_explicit_units_and_lossless_typed_serialization() {
    let request = PointPositionRequest {
        point: EntityId::new(),
        position_mm: [12.25, -4.5],
    };
    request.validate().unwrap();
    let json = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<PointPositionRequest>(&json).unwrap(),
        request
    );
    let mut invalid = serde_json::to_value(&request).unwrap();
    invalid["unit"] = serde_json::json!("inches");
    assert!(serde_json::from_value::<PointPositionRequest>(invalid).is_err());
}
#[test]
fn invalid_coordinates_and_nil_identity_are_rejected() {
    for p in [[f64::NAN, 0.], [0., f64::INFINITY], [1e10, 0.]] {
        assert!(
            PointPositionRequest {
                point: EntityId::new(),
                position_mm: p
            }
            .validate()
            .is_err()
        );
    }
    let nil: EntityId = serde_json::from_str("\"00000000-0000-0000-0000-000000000000\"").unwrap();
    assert!(
        PointPositionRequest {
            point: nil,
            position_mm: [0., 0.]
        }
        .validate()
        .is_err()
    );
}
