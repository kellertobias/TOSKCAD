use super::wheel::tests::{spec, wheel};
use super::wheel::{PIECES, Piece};
use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation};
use crate::manufacturing;
use std::collections::BTreeSet;

fn options(deflection: f64) -> Tessellation {
    Tessellation {
        deflection,
        angle: 0.5,
        triangle_limit: 3_000_000,
    }
}

fn mesh_area(mesh: &DisplayMesh, face: u32) -> f64 {
    let v = |i: u32| {
        let i = i as usize * 3;
        [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
    };
    mesh.indices
        .chunks(3)
        .zip(&mesh.face_indices)
        .filter(|(_, f)| **f == face)
        .map(|(t, _)| norm(cross(sub(v(t[1]), v(t[0])), sub(v(t[2]), v(t[0])))) / 2.)
        .sum()
}

/// Both end points of an edge shape, sampling a curve through `point`.
pub(crate) fn edge_ends(shape: Shape, point: impl Fn(Curve, f64) -> Point) -> [Point; 2] {
    match shape {
        Shape::Line(points) => points,
        Shape::Arc {
            z_mm,
            radius_mm,
            start_rad,
            sweep_rad,
        } => [start_rad, start_rad + sweep_rad]
            .map(|t| [radius_mm * t.cos(), radius_mm * t.sin(), z_mm]),
        Shape::Sampled(curve) => [0., 1.].map(|t| point(curve, t)),
    }
}

#[test]
fn wheels_mesh_watertight_with_exact_volume_areas_and_roles() {
    for (m, teeth, width, bore, thinning) in [
        (1., 32, 1., 0., 0.),
        (2., 40, 2., 6., 0.1),
        (0.5, 64, 0.5, 2., 0.02),
    ] {
        let wheel = wheel(spec(m, teeth, width, bore, thinning));
        let mesh = wheel.mesh(options(0.001 * m)).unwrap();
        let proof = manufacturing::certify(&mesh).unwrap();
        let volume = wheel.volume_mm3();
        assert!(
            (proof.signed_volume_mm3 - volume).abs() / volume < 1e-4,
            "{} {volume}",
            proof.signed_volume_mm3
        );
        let faces = wheel.faces();
        assert_eq!(faces.len(), 9 * teeth as usize + usize::from(bore > 0.));
        let used: BTreeSet<_> = mesh.face_indices.iter().copied().collect();
        assert_eq!(used.len(), faces.len());
        for (index, face) in faces.iter().enumerate().take(9) {
            let measured = mesh_area(&mesh, index as u32);
            assert!(
                (measured - face.area_mm2).abs() / face.area_mm2 < 2e-3,
                "{} {measured} {}",
                face.role,
                face.area_mm2
            );
        }
        let total: f64 = faces.iter().map(|f| f.area_mm2).sum();
        let measured: f64 = (0..faces.len() as u32).map(|f| mesh_area(&mesh, f)).sum();
        assert!(
            (measured - total).abs() / total < 1e-3,
            "{measured} {total}"
        );
        let roles: BTreeSet<_> = faces.iter().map(|f| f.role.clone()).collect();
        assert_eq!(roles.len(), 8 * teeth as usize + usize::from(bore > 0.));
        assert!(roles.contains("worm_wheel/sector/0/root.torus"));
        assert!(roles.contains(&format!("worm_wheel/sector/{}/tip", teeth - 1)));
        let topology = wheel.topology();
        assert_eq!(topology.faces, faces);
        for edge in &topology.edges {
            let [a, b] = edge.vertices.map(|v| topology.vertices_mm[v as usize]);
            if let Shape::Sampled(curve) = edge.shape {
                assert!(wheel.curve_segments(curve, options(0.01)).unwrap() >= 1);
            }
            let ends = edge_ends(edge.shape, |c, t| wheel.curve_point(c, t).unwrap());

            assert!(
                norm(sub(ends[0], a)) < 1e-9 && norm(sub(ends[1], b)) < 1e-9,
                "{edge:?}"
            );
            assert!(edge.faces[0] != edge.faces[1]);
        }
        // Every vertex of the mesh lies within the support bounds.
        for direction in [
            [1., 0., 0.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0.6, -0.8, 0.],
            [0.3, 0.4, 0.866],
        ] {
            let support = wheel.support(direction);
            let best = mesh
                .vertices
                .chunks(3)
                .map(|p| dot([p[0], p[1], p[2]], direction))
                .fold(f64::NEG_INFINITY, f64::max);
            assert!(
                best <= support + 1e-9 && support - best < 0.01 * m,
                "{support} {best}"
            );
        }
        assert!((wheel.support([0., 0., 1.]) - width * m / 2.).abs() < 1e-12);
    }
}

#[test]
fn wheel_classification_follows_spaces_teeth_bore_and_ends() {
    let wheel = wheel(spec(1., 40, 1.5, 4., 0.05));
    let tooth = wheel.span();
    let at = |r: f64, angle: f64, z: f64| [r * angle.cos(), r * angle.sin(), z];
    assert_eq!(wheel.classify(at(20., 0., 0.), 1e-6), Containment::Outside);
    assert_eq!(
        wheel.classify(at(20., tooth, 0.), 1e-6),
        Containment::Inside
    );
    assert_eq!(
        wheel.classify(at(20., tooth + 2. * tooth * 7., 0.5), 1e-6),
        Containment::Inside
    );
    assert_eq!(
        wheel.classify(at(20., tooth, 0.76), 1e-6),
        Containment::Outside
    );
    assert_eq!(
        wheel.classify(at(20., tooth, 0.75), 1e-6),
        Containment::Boundary
    );
    assert_eq!(
        wheel.classify(at(21.2, tooth, 0.), 1e-6),
        Containment::Outside
    );
    assert_eq!(
        wheel.classify(at(3., tooth, 0.), 1e-6),
        Containment::Outside
    );
    assert_eq!(wheel.classify(at(10., 0., 0.), 1e-6), Containment::Inside);
    // Generated flank points are boundary points.
    let section = wheel.section(0.3);
    for piece in PIECES {
        let [x, y] = wheel.piece_point(&section, piece, 0.4);
        assert_eq!(
            wheel.classify([x, y, 0.3], 1e-9),
            Containment::Boundary,
            "{piece:?}"
        );
        assert!(wheel.margin([x * 0.98, y * 0.98, 0.3]) > 0.);
    }
    assert_eq!(wheel.bore_radius_mm(), Some(4.));
    assert!(wheel.curve_point(Curve::WormRail { rail: 0 }, 0.).is_none());
    assert!(
        wheel
            .curve_point(
                Curve::WheelJunction {
                    sector: 0,
                    joint: 0
                },
                0.
            )
            .is_none()
    );
    let outside = Curve::WheelProfile {
        sector: 40,
        end: 0,
        piece: 0,
    };
    assert!(wheel.curve_point(outside, 0.).is_none());
    assert_eq!(
        wheel.curve_segments(Curve::WormRail { rail: 0 }, options(0.1)),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        wheel.curve_segments(
            Curve::WheelJunction {
                sector: 0,
                joint: 1
            },
            options(0.)
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        wheel.mesh(options(f64::NAN)),
        Err(MeshError::InvalidOptions)
    );
    let tight = Tessellation {
        deflection: 0.01,
        angle: 0.5,
        triangle_limit: 1000,
    };
    assert_eq!(wheel.mesh(tight), Err(MeshError::BudgetExceeded));
    let coarse = Tessellation {
        deflection: 1e-9,
        angle: 0.5,
        triangle_limit: 100,
    };
    assert_eq!(wheel.mesh(coarse), Err(MeshError::BudgetExceeded));
    assert_eq!(Piece::FlankPlus.flank_side(), Some(-1.));
}

#[test]
fn wheel_rejects_out_of_envelope_inputs() {
    for (spec, error) in [
        (spec(0.4, 40, 1., 0., 0.), Error::InvalidDimensions),
        (spec(1., 40, 1., 0., 0.2), Error::InvalidDimensions),
        (spec(1., 40, 1., -1., 0.), Error::InvalidDimensions),
        (spec(1., 31, 1., 0., 0.), Error::Limit),
        (spec(1., 65, 1., 0., 0.), Error::Limit),
        (spec(1., 40, 0.4, 0., 0.), Error::Limit),
        (spec(1., 40, 2.1, 0., 0.), Error::Limit),
        (spec(1., 40, 1., 18.75, 0.), Error::InvalidProfile),
    ] {
        assert_eq!(Wheel::new(spec, 1e-3, 1e7), Err(error), "{spec:?}");
    }
    let base = spec(1., 40, 1., 0., 0.);
    assert_eq!(Wheel::new(base, 0., 1e7), Err(Error::InvalidDimensions));
    assert_eq!(Wheel::new(base, 1e-3, 10.), Err(Error::Limit));
    assert_eq!(
        Wheel::new(base, 1e-3, f64::NAN),
        Err(Error::InvalidDimensions)
    );
    // A minimum feature larger than the tip land rejects the profile.
    assert_eq!(Wheel::new(base, 2., 1e7), Err(Error::InvalidProfile));
}
