use super::*;
fn work() -> Work<'static> {
    Work {
        remaining: 100_000,
        interrupt: None,
    }
}
#[test]
fn radial_excess_coefficients_bound_a_real_plane_and_refuse_patches_beyond_the_apex() {
    let points = std::array::from_fn(|a| {
        [
            I::point(2.),
            I::point((a / 10) as f64 / 9.),
            I::point((a % 10) as f64 / 9.),
        ]
    });
    let weights = product_weights();
    let coefficients = excess(&points, I::point(1.), I::point(0.2), &weights, &mut work()).unwrap();
    assert!(coefficients.iter().all(|g| g.lo > 2.5 && g.hi < 4.01));
    assert_eq!(
        excess(&points, I::point(1.), I::point(-2.), &weights, &mut work()).err(),
        Some(Error::Unresolved)
    );
    assert_eq!(
        excess(
            &points,
            I::point(f64::INFINITY),
            I::point(0.),
            &weights,
            &mut work()
        )
        .err(),
        Some(Error::Unresolved)
    );
    assert_eq!(
        excess(
            &points,
            I::point(1.),
            I::point(0.2),
            &weights,
            &mut Work {
                remaining: 1,
                interrupt: None
            }
        )
        .err(),
        Some(Error::Limit)
    );
    assert_eq!(
        excess(
            &points,
            I::point(1.),
            I::point(0.2),
            &weights,
            &mut Work {
                remaining: 100_000,
                interrupt: Some(&|| true)
            }
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert!(excess(&points, I::point(1.), I::point(0.2), &weights, &mut work()).is_ok());
}
#[test]
fn final_tolerance_cannot_widen_prior_cap_collars_or_hide_missing_precision() {
    let ordered = crate::radial_tensor::tests::fixture()
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap()
        .certify_caps(None)
        .unwrap()
        .certify_ordering(None)
        .unwrap();
    assert_eq!(
        ordered.clone().certify_material(1e-30, None).err(),
        Some(Error::Unresolved)
    );
    let exact = ordered.clone().certify_material(1e-7, None).unwrap();
    let coarse = ordered.certify_material(1e7, None).unwrap();
    assert_eq!(coarse.collars(), exact.collars());
}
