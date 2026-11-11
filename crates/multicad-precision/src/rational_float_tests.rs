use super::Rational;

#[test]
fn exact_ratio_rounds_once_with_even_ties_and_significand_carry() {
    let ratio = |n, d| Rational::new(n, d).unwrap().as_f64();
    let two59 = 1_i64 << 59;
    for sign in [-1, 1] {
        assert_eq!(
            ratio(sign * ((1_i64 << 53) + 1), 1_i64 << 54),
            sign as f64 * 0.5
        );
        assert_eq!(
            ratio(sign * ((1_i64 << 53) + 3), 1_i64 << 54),
            sign as f64 * f64::from_bits(0.5_f64.to_bits() + 2)
        );
        assert_eq!(ratio(sign * (two59 + 64), 1), sign as f64 * two59 as f64);
        assert_eq!(
            ratio(sign * (two59 + 192), 1),
            sign as f64 * (two59 + 256) as f64
        );
        assert_eq!(
            ratio(sign * ((1_i64 << 54) - 1), 2),
            sign as f64 * (1_u64 << 53) as f64
        );
        let source = sign as f64 * 3.6285714285714286;
        assert_eq!(
            Rational::parse_decimal(&source.to_string())
                .unwrap()
                .as_f64()
                .to_bits(),
            source.to_bits()
        );
    }
    assert_eq!(ratio(0, 1), 0.);
    assert_eq!(ratio(1, 2), 0.5);
    assert_eq!(ratio(1, 1_000_000_000_000_000_000), 1e-18);
    assert_eq!(ratio(1_000_000_000_000_000_000, 1), 1e18);
}

#[test]
fn retained_decimal_literals_recover_the_original_float_across_the_bounded_range() {
    let mut state = 290_u64;
    let mut admitted = 0;
    for _ in 0..10_000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let exponent = 968 + ((state >> 52) % 110);
        let bits = (state & (1_u64 << 63)) | (exponent << 52) | (state & ((1_u64 << 52) - 1));
        let value = f64::from_bits(bits);
        if let Ok(rational) = Rational::parse_decimal(&value.to_string()) {
            assert_eq!(rational.as_f64().to_bits(), value.to_bits(), "{value}");
            admitted += 1;
        }
    }
    assert!(admitted > 1_000);
}
