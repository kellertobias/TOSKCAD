use super::*;
use crate::{display::Tessellation, manufacturing};

/// Mirror of the canonical spherical-involute resolver for a 90° pair.
pub(crate) fn spec(module: f64, teeth: u32, mate: u32, width: f64, bore: f64, fillet: f64) -> Spec {
    let (z, mate) = (f64::from(teeth), f64::from(mate));
    let alpha = 20_f64.to_radians();
    let delta = 1_f64.atan2(mate / z);
    let distance = module * z / (2. * delta.sin());
    let beta = (delta.sin() * alpha.cos()).asin();
    let inv = |g: f64| {
        let sigma = (g.cos() / beta.cos()).acos();
        sigma / beta.sin() - sigma.sin().atan2(beta.sin() * sigma.cos())
    };
    let thickness = PI * module / 2. - 0.05;
    Spec {
        teeth,
        pitch_cone_angle_rad: delta,
        base_cone_angle_rad: beta,
        tip_cone_angle_rad: delta + (module / distance).atan(),
        root_cone_angle_rad: delta - (1.25 * module / distance).atan(),
        outer_cone_distance_mm: distance,
        face_width_mm: width,
        bore_radius_mm: bore,
        half_base_tooth_angle_rad: thickness / (module * z) + inv(delta),
        root_fillet_radius_mm: module * fillet,
    }
}

fn options(deflection: f64) -> Tessellation {
    Tessellation {
        deflection,
        angle: 0.5,
        triangle_limit: 2_000_000,
    }
}

fn gear(spec: Spec) -> Bevel {
    Bevel::new(spec, 1e-3, 1e7).unwrap()
}

fn mesh_area(mesh: &crate::display::DisplayMesh, face: Option<u32>) -> f64 {
    let v = |i: u32| {
        let i = i as usize * 3;
        [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
    };
    mesh.indices
        .chunks(3)
        .zip(&mesh.face_indices)
        .filter(|(_, f)| face.is_none_or(|face| **f == face))
        .map(|(t, _)| norm(cross(sub(v(t[1]), v(t[0])), sub(v(t[2]), v(t[0])))) / 2.)
        .sum()
}

#[test]
fn both_root_branches_mesh_watertight_with_exact_volume_and_areas() {
    for (spec, meridian) in [
        (spec(1., 24, 64, 2., 0., 0.2), true),
        (spec(2., 32, 24, 6., 4., 0.2), false),
        (spec(1.5, 64, 24, 5., 3., 0.05), false),
    ] {
        let gear = gear(spec);
        assert_eq!(gear.has_meridian(), meridian, "{spec:?}");
        let mesh = gear.mesh(options(0.002)).unwrap();
        let proof = manufacturing::certify(&mesh).unwrap();
        let volume = gear.volume_mm3();
        let relative = (proof.signed_volume_mm3 - volume).abs() / volume;
        assert!(relative < 2e-4, "{relative} {volume}");
        let topology = gear.topology();
        let exact: f64 = topology
            .faces
            .iter()
            .map(|face| gear.face_metrics(*face).1)
            .sum();
        let measured = mesh_area(&mesh, None);
        assert!(
            (measured - exact).abs() / exact < 2e-4,
            "{measured} {exact}"
        );
        for (index, face) in topology.faces.iter().enumerate().take(40) {
            let area = mesh_area(&mesh, Some(index as u32));
            let expected = gear.face_metrics(*face).1;
            assert!(
                (area - expected).abs() <= 6e-3 * expected + 1e-6,
                "{face:?} {area} {expected}"
            );
        }
        // Refinement converges toward the exact integral.
        let fine = gear.mesh(options(0.0005)).unwrap();
        let fine_volume = manufacturing::certify(&fine).unwrap().signed_volume_mm3;
        assert!((fine_volume - volume).abs() <= (proof.signed_volume_mm3 - volume).abs() + 1e-9);
    }
}

#[test]
fn analytic_volume_matches_an_independent_cone_integral() {
    let spec = spec(1., 24, 24, 3., 2., 0.2);
    let gear = gear(spec);
    // Integrate r² over the direction domain with a midpoint grid.
    let n = 4000;
    let mut total = 0.;
    let pitch = TAU / f64::from(spec.teeth);
    for i in 0..n {
        let gamma = gear.tip_gamma() * (i as f64 + 0.5) / n as f64;
        let width = if gamma <= gear.root_gamma() {
            pitch / 2.
        } else {
            gear.half_width(gamma)
        };
        let [heel, toe] = [End::Heel, End::Toe].map(|e| gear.end_distance_along(e, gamma));
        total += 2. * width * gamma.sin() * (heel.powi(3) - toe.powi(3)) / 3. * gear.tip_gamma()
            / n as f64;
    }
    total *= f64::from(spec.teeth);
    total -= PI * 4. * 3. / spec.pitch_cone_angle_rad.cos();
    assert!(
        (total - gear.volume_mm3()).abs() / total < 1e-4,
        "{total} {}",
        gear.volume_mm3()
    );
}

#[test]
fn classification_separates_teeth_gaps_bore_and_back_cones() {
    let spec = spec(2., 24, 32, 6., 4., 0.2);
    let gear = gear(spec);
    let pitch = TAU / 24.;
    for fraction in [0.2, 0.5, 0.8] {
        let r = spec.outer_cone_distance_mm - spec.face_width_mm * fraction;
        for tooth in 0..24 {
            let center = polar(spec.pitch_cone_angle_rad, pitch * f64::from(tooth));
            let gap = polar(spec.pitch_cone_angle_rad, pitch * (f64::from(tooth) + 0.5));
            assert_eq!(gear.classify(scale(center, r), 1e-6), Containment::Inside);
            assert_eq!(gear.classify(scale(gap, r), 1e-6), Containment::Outside);
        }
    }
    let heel = gear.point(
        End::Heel,
        polar(spec.pitch_cone_angle_rad, 0.),
        spec.pitch_cone_angle_rad,
    );
    assert_eq!(gear.classify(heel, 1e-6), Containment::Boundary);
    assert_eq!(gear.classify(scale(heel, 1.01), 1e-6), Containment::Outside);
    let axis = [0., 0., gear.back_cone_apex_mm(End::Heel) - 1.];
    assert_eq!(gear.classify(axis, 1e-6), Containment::Outside);
    assert_eq!(gear.classify([0.; 3], 1e-6), Containment::Outside);
    let solid = Bevel::new(
        Spec {
            bore_radius_mm: 0.,
            ..spec
        },
        1e-3,
        1e7,
    )
    .unwrap();
    assert_eq!(solid.classify(axis, 1e-6), Containment::Inside);
    // Mesh vertices lie on the boundary within the tessellation tolerance.
    let mesh = gear.mesh(options(0.01)).unwrap();
    for point in mesh.vertices.chunks(3).step_by(7) {
        let point = [point[0], point[1], point[2]];
        assert_eq!(
            gear.classify(point, 1e-6),
            Containment::Boundary,
            "{point:?}"
        );
    }
}

#[test]
fn support_bounds_enclose_every_mesh_vertex_tightly() {
    for spec in [spec(2., 24, 32, 6., 4., 0.2), spec(1., 32, 24, 3., 0., 0.1)] {
        let gear = gear(spec);
        let mesh = gear.mesh(options(0.005)).unwrap();
        for direction in [
            [1., 0., 0.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
            [0.6, -0.48, 0.64],
        ] {
            let support = gear.support(direction);
            let measured = mesh
                .vertices
                .chunks(3)
                .map(|p| dot([p[0], p[1], p[2]], direction))
                .fold(f64::NEG_INFINITY, f64::max);
            assert!(
                support >= measured - 1e-9 && support - measured < 0.01,
                "{support} {measured}"
            );
        }
    }
}

#[test]
fn topology_is_a_closed_manifold_with_native_roles() {
    for spec in [spec(2., 24, 32, 6., 4., 0.2), spec(1., 32, 24, 3., 0., 0.1)] {
        let gear = gear(spec);
        let topology = gear.topology();
        let mut uses = std::collections::BTreeMap::<u32, usize>::new();
        for edge in &topology.edges {
            for face in edge.faces {
                *uses.entry(face).or_default() += 1;
            }
            // Analytic and sampled curves both end on their vertices.
            for (s, vertex) in [(0., edge.vertices[0]), (1., edge.vertices[1])] {
                let p = gear.curve_point(&topology, edge, s);
                let q = topology.vertices_mm[vertex as usize];
                assert!(norm(sub(p, q)) < 1e-9, "{edge:?} {p:?} {q:?}");
            }
        }
        assert!(uses.len() == topology.faces.len() && uses.values().all(|n| *n >= 2));
        // Closed orientable surface: V - E + F = 2 - 2g, genus one with a bore.
        let euler = topology.vertices_mm.len() as i64 - topology.edges.len() as i64
            + topology.faces.len() as i64;
        assert_eq!(
            euler,
            if gear.bore_radius_mm().is_some() {
                0
            } else {
                2
            }
        );
        let roles: std::collections::BTreeSet<_> = topology.faces.iter().map(Face::role).collect();
        assert!(roles.contains("bevel/tooth/0/spherical_involute.left"));
        assert!(roles.contains("bevel/heel/sector/3/outer"));
        assert!(roles.contains("bevel/toe/sector/0/core"));
        assert_eq!(
            roles.contains("bevel/bore"),
            gear.bore_radius_mm().is_some()
        );
        let planes = topology
            .faces
            .iter()
            .filter(|face| gear.face_metrics(**face).2.is_some())
            .count();
        assert_eq!(
            planes,
            if gear.has_meridian() {
                2 * spec.teeth as usize
            } else {
                0
            }
        );
    }
}

#[test]
fn a_bore_wider_than_the_inner_arc_removes_one_or_both_cores() {
    let mut spec = spec(2., 24, 24, 6., 0., 0.2);
    let gear0 = gear(spec);
    let toe_inner = gear0.end_radius(End::Toe, gear0.inner_gamma());
    let heel_inner = gear0.end_radius(End::Heel, gear0.inner_gamma());
    spec.bore_radius_mm = (toe_inner + heel_inner) / 2.;
    let mixed = gear(spec);
    assert!(mixed.has_core(End::Heel) && !mixed.has_core(End::Toe));
    spec.bore_radius_mm = heel_inner + 1.;
    let gear = gear(spec);
    assert!(!gear.has_core(End::Heel) && !gear.has_core(End::Toe));
    for gear in [mixed, gear] {
        let mesh = gear.mesh(options(0.005)).unwrap();
        let proof = manufacturing::certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 - gear.volume_mm3()).abs() / gear.volume_mm3() < 5e-4);
        let topology = gear.topology();
        let euler = topology.vertices_mm.len() as i64 - topology.edges.len() as i64
            + topology.faces.len() as i64;
        assert_eq!(euler, 0);
        let exact: f64 = topology.faces.iter().map(|f| gear.face_metrics(*f).1).sum();
        let measured = mesh_area(&mesh, None);
        assert!(
            (measured - exact).abs() / exact < 5e-4,
            "{measured} {exact}"
        );
    }
    spec.bore_radius_mm = toe_inner;
    assert_eq!(Bevel::new(spec, 1e-3, 1e7), Err(Error::InvalidProfile));
}

#[test]
fn invalid_dimensions_profiles_limits_and_budgets_are_refused() {
    let good = spec(1., 24, 32, 2., 0., 0.2);
    for broken in [
        Spec { teeth: 3, ..good },
        Spec {
            root_cone_angle_rad: good.pitch_cone_angle_rad,
            ..good
        },
        Spec {
            tip_cone_angle_rad: FRAC_PI_2,
            ..good
        },
        Spec {
            face_width_mm: good.outer_cone_distance_mm,
            ..good
        },
        Spec {
            bore_radius_mm: -1.,
            ..good
        },
        Spec {
            root_fillet_radius_mm: 0.,
            ..good
        },
        Spec {
            half_base_tooth_angle_rad: f64::NAN,
            ..good
        },
    ] {
        assert_eq!(Bevel::new(broken, 1e-3, 1e7), Err(Error::InvalidDimensions));
    }
    assert_eq!(Bevel::new(good, 0., 1e7), Err(Error::InvalidDimensions));
    assert_eq!(Bevel::new(good, 1e-3, 10.), Err(Error::Limit));
    for broken in [
        Spec {
            bore_radius_mm: 20.,
            ..good
        },
        Spec {
            half_base_tooth_angle_rad: 0.01,
            ..good
        },
        Spec {
            half_base_tooth_angle_rad: 0.3,
            ..good
        },
        Spec {
            root_fillet_radius_mm: 0.9,
            ..good
        },
    ] {
        assert_eq!(
            Bevel::new(broken, 1e-3, 1e7),
            Err(Error::InvalidProfile),
            "{broken:?}"
        );
    }
    let gear = gear(good);
    let budget = Tessellation {
        triangle_limit: 100,
        ..options(0.01)
    };
    assert_eq!(
        gear.mesh(budget),
        Err(crate::display::MeshError::BudgetExceeded)
    );
    let tiny = Tessellation {
        triangle_limit: 3,
        ..options(1e-9)
    };
    assert_eq!(
        gear.piece_segments(Piece::InvoluteLeft, tiny),
        Err(crate::display::MeshError::BudgetExceeded)
    );
    assert_eq!(
        gear.row_segments(gear.root_gamma(), 1., tiny),
        Err(crate::display::MeshError::BudgetExceeded)
    );
    assert_eq!(
        gear.mesh(Tessellation {
            deflection: 0.,
            ..budget
        }),
        Err(crate::display::MeshError::InvalidOptions)
    );
    assert_eq!(End::Toe.name(), "toe");
    assert_eq!(gear.spec(), good);
    assert_eq!(gear.teeth(), 24);
}

#[test]
fn sliver_meridians_are_absorbed_by_an_exactly_tangent_fillet() {
    let minimum = 1e-3;
    let base = spec(1., 24, 64, 2., 0., 0.2);
    let distance = base.outer_cone_distance_mm;
    let root = base.root_cone_angle_rad;
    let beta = base.base_cone_angle_rad;
    // Meridian length on the unit sphere for a fillet radius in mm.
    let length = |fillet: f64| {
        let rho = (fillet / distance).asin();
        beta - ((root + rho).cos() / rho.cos()).acos()
    };
    let (mut lo, mut hi) = (base.root_fillet_radius_mm, distance * 0.5);
    let target = 0.5 * minimum / distance;
    for _ in 0..200 {
        let middle = (lo + hi) / 2.;
        if length(middle) > target {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    let sliver = Spec {
        root_fillet_radius_mm: lo,
        ..base
    };
    assert!(length(lo) > 0. && length(lo) < minimum / distance);
    let gear = Bevel::new(sliver, minimum, 1e7).unwrap();
    assert!(!gear.has_meridian());
    assert!(
        gear.pieces()
            .iter()
            .all(|p| !matches!(p, Piece::MeridianLeft | Piece::MeridianRight))
    );
    // The fillet ends exactly where the involute leaves the base cone.
    for (fillet, involute) in [
        (Piece::FilletLeft, Piece::InvoluteLeft),
        (Piece::FilletRight, Piece::InvoluteRight),
    ] {
        let (a, b) = if fillet == Piece::FilletLeft {
            (gear.sample(fillet, 1.), gear.sample(involute, 0.))
        } else {
            (gear.sample(fillet, 0.), gear.sample(involute, 1.))
        };
        assert!(norm(sub(a.direction, b.direction)) < 1e-12);
        assert!((a.gamma - beta).abs() < 1e-12);
        // Both leave the joint in the meridian direction.
        let tangent = |d: Point, u: Point| {
            let radial = sub(d, scale(u, dot(d, u)));
            scale(radial, 1. / norm(radial))
        };
        let along = |s: &Sample| {
            let polar = [
                s.gamma.cos() * s.phi.cos(),
                s.gamma.cos() * s.phi.sin(),
                -s.gamma.sin(),
            ];
            dot(tangent(s.derivative, s.direction), polar).abs()
        };
        assert!((along(&a) - 1.).abs() < 1e-9);
    }
    // The absorbed fillet stays within the derived radius bound and the
    // solid remains watertight with its exact volume.
    let effective = gear.rounding.sin() * distance;
    assert!(effective > lo && effective - lo <= minimum * beta.sin() / root.sin());
    let mesh = gear.mesh(options(0.002)).unwrap();
    let proof = manufacturing::certify(&mesh).unwrap();
    assert!((proof.signed_volume_mm3 - gear.volume_mm3()).abs() / gear.volume_mm3() < 2e-4);
    let roles: std::collections::BTreeSet<_> = gear
        .topology()
        .faces
        .iter()
        .map(|face| face.role())
        .collect();
    assert!(roles.iter().all(|role| !role.contains("meridian")));
    let text = crate::step::write_bevel(&gear, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-6).unwrap();
    let decoded =
        crate::step_read::verify_bevel(&text, &gear, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-6)
            .unwrap();
    assert!((decoded - gear.volume_mm3()).abs() / decoded < 1e-8);
}
