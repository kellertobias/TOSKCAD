use super::*;
#[test]
fn retained_export_requires_the_complete_unchanged_finite_source() {
    let body = crate::radial_tensor::tests::material();
    let text = write_radial_boundary(&body, 1e-7).unwrap();
    verify_candidate(&text, &body, 1e-7).unwrap();
    assert_eq!(
        verify_candidate("not STEP", &body, 1e-7),
        Err(StepError::Unsupported)
    );
    let oversized = " ".repeat(crate::step_read::MAX_PLANAR_STEP_BYTES + 1);
    assert_eq!(
        verify_candidate(&oversized, &body, 1e-7),
        Err(StepError::Limit)
    );
    let tensor = body
        .input()
        .surfaces
        .iter()
        .find_map(|s| {
            if let Surface::Tensor(t) = s {
                Some(t)
            } else {
                None
            }
        })
        .unwrap();
    let p = tensor.poles()[tensor.dimensions()[1] + 1];
    let old = Coordinates(p).to_string();
    let mut p = p;
    p[0] += 0.001;
    let modified = text.replacen(&old, &Coordinates(p).to_string(), 1);
    assert_ne!(modified, text);
    // Abstract ownership still holds; the independent finite-source proof
    // must detect the changed interior pole, not merely compare metrics.
    crate::step_read::read_radial_boundary(&modified, 1e-7).unwrap();
    assert_eq!(
        verify_candidate(&modified, &body, 1e-7),
        Err(StepError::Unsupported)
    );
    verify_candidate(&text, &body, 1e-7).unwrap();
}
