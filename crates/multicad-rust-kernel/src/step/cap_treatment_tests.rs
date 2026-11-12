use super::*;
#[test]
fn a_zero_height_wall_with_no_radial_reach_is_not_a_shoulder() {
    let mut file = StepFile::new();
    let normal = [0., 0., 1.];
    let reference = [1., 0., 0.];
    let rim = CircularRim::new(&mut file, [0.; 3], normal, reference, 2.).unwrap();
    assert!(matches!(
        round_wall(&mut file, &rim, &rim, 0., normal, reference, false),
        Err(StepError::InvalidGeometry)
    ));
}
