use super::*;
use crate::bevel_gear::tests::spec;

fn gear(bore: f64, teeth: u32, mate: u32) -> Bevel {
    Bevel::new(spec(2., teeth, mate, 6., bore, 0.2), 1e-3, 1e7).unwrap()
}

#[test]
fn bevels_encode_one_closed_shell_with_every_topological_face() {
    let frame = ([10., -20., 30.], [0.8, 0., -0.6], [0., 1., 0.]);
    for (bore, teeth, mate) in [(4., 24, 32), (0., 32, 24), (12.5, 24, 24)] {
        let gear = gear(bore, teeth, mate);
        assert!(supports_bevel(&gear, frame.0, frame.1, frame.2, 1e-6));
        let text = write_bevel(&gear, frame.0, frame.1, frame.2, 1e-6).unwrap();
        let topology = gear.topology();
        assert_eq!(text.matches("ADVANCED_FACE(").count(), topology.faces.len());
        assert_eq!(text.matches("=EDGE_CURVE(").count(), topology.edges.len());
        assert_eq!(
            text.matches("=VERTEX_POINT(").count(),
            topology.vertices_mm.len()
        );
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP(").count(), 1);
        assert_eq!(
            text.matches("=CYLINDRICAL_SURFACE(").count(),
            usize::from(gear.bore_radius_mm().is_some())
        );
        assert_eq!(
            text.matches("=PLANE(").count(),
            if gear.has_meridian() {
                2 * teeth as usize
            } else {
                0
            }
        );
    }
}

#[test]
fn rails_match_the_exact_heel_curves_and_refuse_impossible_budgets() {
    let gear = gear(4., 24, 32);
    for piece in [Piece::FilletLeft, Piece::InvoluteRight] {
        let rail = Rail::fit(&gear, piece, 1e-6).unwrap();
        assert_eq!(rail.poles.len(), 2 * (rail.knots.len() - 1) + 2);
        let first = gear.sample(piece, 0.);
        let exact = gear.point(End::Heel, first.direction, first.gamma);
        assert_eq!(rail.poles[0], exact);
        assert_eq!(rail.multiplicities().matches('4').count(), 2);
        assert!(rail.knot_values().starts_with("0.0000000000000000E0"));
    }
    assert!(matches!(
        Rail::fit(&gear, Piece::InvoluteLeft, 1e-300),
        Err(StepError::Limit)
    ));
    assert!(matches!(
        prepare(&gear, 0.),
        Err(StepError::InvalidGeometry)
    ));
    assert!(!supports_bevel(
        &gear,
        [0.; 3],
        [1., 0., 0.],
        [1., 0., 0.],
        1e-6
    ));
    let dense = Bevel::new(spec(2., 64, 64, 6., 0., 0.2), 1e-3, 1e7).unwrap();
    assert!(matches!(prepare(&dense, 1e-12), Err(StepError::Limit)));
}
