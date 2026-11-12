use super::*;
use crate::{
    display::Tessellation,
    manufacturing::certify,
    revolution::{cross, moments},
};

const MINIMUM: f64 = 1e-5;
const TOLERANCE: f64 = 1e-7;

/// The native internal-gear test specimen: module 1, 20 degrees, a 0.2 mm
/// fillet and a 4 mm rim beyond the pitch circle.
fn spec(teeth: u32, thinning: f64) -> RingSpec {
    let a = 20_f64.to_radians();
    let pitch = f64::from(teeth) / 2.;
    RingSpec {
        teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * a.cos(),
        tip_radius_mm: pitch - 1.,
        root_radius_mm: pitch + 1.25,
        root_fillet_radius_mm: 0.2,
        half_base_angle_rad: (PI / 2. - thinning) / (2. * pitch) - a.tan() + a,
        face_width_mm: 2.,
        outer_radius_mm: pitch + 4.,
    }
}

fn ring(spec: RingSpec) -> Result<Ring, Error> {
    Ring::new(spec, MINIMUM, MAX_COORDINATE_MM, TOLERANCE)
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

fn angle_offset([x, y]: Point, phase: f64) -> f64 {
    let angle = y.atan2(x) - phase;
    angle.sin().atan2(angle.cos())
}

#[test]
fn cavity_matches_native_roles_joins_and_independent_area() {
    // Independent radial integration of the material-width equation, as
    // recorded by the native internal-gear test (64/128-point quadrature).
    for (teeth, band_area) in [(48, 181.940372898279), (60, 225.536388250187)] {
        let spec = spec(teeth, 0.05);
        let ring = ring(spec).unwrap();
        let cavity = ring.cavity();
        assert_eq!(cavity.len(), teeth as usize * 6);
        assert_eq!(ring.teeth(), teeth);
        assert_eq!(ring.tip_radius_mm(), spec.tip_radius_mm);
        assert_eq!(ring.root_radius_mm(), spec.root_radius_mm);
        assert_eq!(ring.outer_radius_mm(), spec.outer_radius_mm);
        assert_eq!(ring.height_mm(), spec.face_width_mm);
        let expected =
            band_area + PI * (spec.outer_radius_mm.powi(2) - spec.root_radius_mm.powi(2));
        close(ring.area_mm2(), expected, 1e-8);
        close(ring.volume_mm3(), expected * spec.face_width_mm, 2e-8);
        // Exact joins, all shared bit-for-bit, and a clockwise cavity.
        for (i, entry) in cavity.iter().enumerate() {
            let next = cavity[(i + 1) % cavity.len()].edge;
            assert_eq!(entry.edge.point(1.), next.point(0.), "join {i}");
        }
        let mut sampled = 0.;
        for entry in cavity {
            for i in 0..512 {
                let [a, b] = [i, i + 1].map(|j| entry.edge.point(f64::from(j) / 512.));
                sampled += cross(a, b) / 2.;
            }
        }
        assert!(sampled < 0.);
        close(PI * spec.outer_radius_mm.powi(2) + sampled, expected, 1e-4);
        // Tangent continuity at both fillet ends, as the native fillet.
        for (i, entry) in cavity.iter().enumerate() {
            if matches!(entry.role, ToothRole::LeftFillet | ToothRole::RightFillet) {
                let before = cavity[(i + cavity.len() - 1) % cavity.len()].edge;
                let after = cavity[(i + 1) % cavity.len()].edge;
                for [a, b] in [
                    [before.tangent(1.), entry.edge.tangent(0.)],
                    [entry.edge.tangent(1.), after.tangent(0.)],
                ] {
                    close(cross(a, b), 0., 1e-9);
                }
            }
        }
        // Native tooth roles: tips centered on material teeth, left/right
        // flanks and fillets on either side, roots centered in spaces.
        let pitch = TAU / f64::from(teeth);
        for entry in cavity {
            let phase = pitch * f64::from(entry.tooth);
            let offset = angle_offset(entry.edge.centroid(), phase);
            match entry.role {
                ToothRole::Tip => close(offset, 0., 1e-12),
                ToothRole::LeftFlank | ToothRole::LeftFillet => {
                    assert!(offset < 0. && offset > -pitch / 2.)
                }
                ToothRole::RightFlank | ToothRole::RightFillet => {
                    assert!(offset > 0. && offset < pitch / 2.)
                }
                ToothRole::Root => close(offset, pitch / 2., 1e-12),
                ToothRole::LeftRadial | ToothRole::RightRadial => unreachable!(),
            }
        }
        let radius = |role| {
            cavity
                .iter()
                .find(|entry| entry.role == role)
                .map(|entry| entry.edge.point(0.5))
                .map(|p| p[0].hypot(p[1]))
                .unwrap()
        };
        close(radius(ToothRole::Tip), spec.tip_radius_mm, 1e-12);
        close(radius(ToothRole::Root), spec.root_radius_mm, 1e-12);
        // Surface area, centroid and support from the analytic boundary.
        let perimeter = cavity
            .iter()
            .map(|entry| entry.edge.length_mm())
            .sum::<f64>();
        close(
            ring.surface_area_mm2(),
            2. * ring.area_mm2() + (perimeter + TAU * spec.outer_radius_mm) * 2.,
            1e-9,
        );
        let [x, y, z] = ring.centroid();
        assert!(x.abs() < 1e-10 && y.abs() < 1e-10);
        assert_eq!(z, 1.);
        assert_eq!(
            ring.rim(),
            normal_sweep::Edge::Circle {
                center: [0.; 2],
                radius_mm: spec.outer_radius_mm,
                clockwise: false
            }
        );
        let r = spec.outer_radius_mm;
        assert_eq!(ring.bounds(), [[-r, -r, 0.], [r, r, 2.]]);
        close(ring.support([0.6, 0.8, 1.]), r + 2., 1e-12);
        assert_eq!(ring.support([0., 0., -1.]), 0.);
        assert!(ring.support([f64::NAN, 0., 0.]).is_nan());
    }
}

#[test]
fn involute_first_moments_match_sampled_green_integrals() {
    let curve = Involute::new(12., [0.15, 0.9], -0.2).unwrap();
    for curve in [
        curve,
        curve.reversed(),
        curve.reflected().rotated(1.1).unwrap(),
        curve.reflected().reversed().rotated(-2.).unwrap(),
    ] {
        let mut points: Vec<_> = (0..=20_000)
            .map(|i| curve.point(f64::from(i) / 20_000.))
            .collect();
        // Close through the base center, which adds no first moment.
        points.push([0.; 2]);
        let [area, x, y] = moments(&points, [0.; 2]);
        close(curve.signed_area_mm2(), area, 1e-4);
        let [mx, my] = curve.first_moment_mm3();
        close(mx, x, 1e-3);
        close(my, y, 1e-3);
    }
}

#[test]
fn exact_classification_matches_the_native_probes() {
    let spec = spec(48, 0.05);
    let ring = ring(spec).unwrap();
    let pitch = TAU / 48.;
    let at = |r: f64, angle: f64, z: f64| {
        ring.classify([r * angle.cos(), r * angle.sin(), z], TOLERANCE)
    };
    for tooth in [0., 5., 47.] {
        let a = tooth * pitch;
        for z in [0.4, 1., 1.6] {
            assert_eq!(at(spec.pitch_radius_mm, a, z), Containment::Inside);
            assert_eq!(
                at(spec.pitch_radius_mm, a + pitch / 2., z),
                Containment::Outside
            );
            assert_eq!(
                at(
                    (spec.outer_radius_mm + spec.root_radius_mm) / 2.,
                    a + pitch / 2.,
                    z
                ),
                Containment::Inside
            );
            assert_eq!(at(spec.tip_radius_mm * 0.99, a, z), Containment::Outside);
        }
    }
    // Independently reconstructed material width from the pitch thickness.
    let reference = 20_f64.to_radians();
    for r in [
        spec.tip_radius_mm + 0.1,
        spec.pitch_radius_mm,
        spec.root_radius_mm - 0.5,
    ] {
        let alpha = (spec.base_radius_mm / r).acos();
        let half =
            (PI / 2. - 0.05) / (2. * spec.pitch_radius_mm) + alpha.tan() - alpha - reference.tan()
                + reference;
        for sign in [-1., 1.] {
            assert_eq!(at(r, sign * (half - 1e-6), 1.), Containment::Inside);
            assert_eq!(at(r, sign * (half + 1e-6), 1.), Containment::Outside);
            assert_eq!(at(r, sign * half, 1.), Containment::Boundary);
        }
    }
    // Inside the circular fillet between the foot and the outward root.
    let fillet = ring
        .cavity()
        .iter()
        .find(|entry| entry.role == ToothRole::RightFillet)
        .unwrap()
        .edge
        .point(0.5);
    let [x, y] = fillet;
    assert_eq!(ring.classify([x, y, 1.], TOLERANCE), Containment::Boundary);
    let outward = scale(fillet, 1. + 1e-5);
    let inward = scale(fillet, 1. - 1e-5);
    assert_ne!(
        ring.classify([outward[0], outward[1], 1.], TOLERANCE),
        ring.classify([inward[0], inward[1], 1.], TOLERANCE)
    );
    // Root, tip, rim and both caps are boundaries; points beyond are outside.
    for (r, angle, z, expected) in [
        (spec.root_radius_mm, pitch / 2., 1., Containment::Boundary),
        (
            spec.root_radius_mm + 0.1,
            pitch / 2.,
            1.,
            Containment::Inside,
        ),
        (spec.root_radius_mm + 0.1, 0., 1., Containment::Inside),
        (spec.tip_radius_mm, 0., 1., Containment::Boundary),
        (
            spec.tip_radius_mm - 0.1,
            pitch / 2.,
            1.,
            Containment::Outside,
        ),
        (spec.outer_radius_mm, 0.3, 1., Containment::Boundary),
        (spec.outer_radius_mm + 1., 0.3, 1., Containment::Outside),
        (26., 0.3, 0., Containment::Boundary),
        (26., 0.3, 2., Containment::Boundary),
        (26., 0.3, 2. + 1e-8, Containment::Boundary),
        (26., 0.3, 3., Containment::Outside),
        (spec.pitch_radius_mm, pitch / 2., -1., Containment::Outside),
        (spec.pitch_radius_mm, 0., 0.5, Containment::Inside),
    ] {
        assert_eq!(at(r, angle, z), expected, "{r} {angle} {z}");
    }
    assert_eq!(ring.classify([f64::NAN, 0., 1.], 0.), Containment::Outside);
    assert_eq!(ring.classify([26., 0., 1.], -1.), Containment::Outside);
}

#[test]
fn mesh_is_a_certified_closed_annulus_with_native_face_order() {
    let options = Tessellation {
        deflection: 0.002,
        angle: 0.15,
        triangle_limit: 400_000,
    };
    for teeth in [48, 60] {
        let ring = ring(spec(teeth, 0.05)).unwrap();
        let mesh = ring.mesh(options).unwrap();
        let proof = certify(&mesh).unwrap();
        let lateral = ring.surface_area_mm2() - 2. * ring.area_mm2();
        close(
            proof.signed_volume_mm3,
            ring.volume_mm3(),
            lateral * options.deflection,
        );
        let faces = ring.cavity().len() + 3;
        let mut counts = vec![0; faces];
        for &face in &mesh.face_indices {
            counts[face as usize] += 1;
        }
        assert!(counts.iter().all(|&n| n > 0));
        // Every rim vertex lies on the exact outside cylinder.
        for (triangle, &face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            if face as usize == faces - 1 {
                for &v in triangle {
                    let p = &mesh.vertices[v as usize * 3..v as usize * 3 + 2];
                    close(p[0].hypot(p[1]), ring.outer_radius_mm(), 1e-12);
                }
            }
        }
    }
    let ring = ring(spec(48, 0.05)).unwrap();
    for limited in [
        Tessellation {
            triangle_limit: 1100,
            ..options
        },
        Tessellation {
            triangle_limit: 100,
            ..options
        },
    ] {
        assert_eq!(ring.mesh(limited), Err(MeshError::BudgetExceeded));
    }
    assert_eq!(
        ring.mesh(Tessellation {
            deflection: -1.,
            ..options
        }),
        Err(MeshError::InvalidOptions)
    );
}

#[test]
fn refuses_the_native_internal_ring_envelope() {
    let nominal = spec(48, 0.05);
    let limits = |minimum, maximum, tolerance| Ring::new(nominal, minimum, maximum, tolerance);
    for (minimum, maximum, tolerance) in [
        (0., 1e7, 1e-7),
        (1e-5, 1e-5, 1e-7),
        (1e-5, 2e7, 1e-7),
        (1e-5, 1e7, 0.),
        (1e-5, 1e7, 1e-4),
        (1e-5, 27., 1e-7),
    ] {
        assert_eq!(
            limits(minimum, maximum, tolerance),
            Err(Error::InvalidDimensions)
        );
    }
    let start = {
        let t = ((nominal.tip_radius_mm / nominal.base_radius_mm).powi(2) - 1.).sqrt();
        t - t.atan()
    };
    for invalid in [
        RingSpec {
            teeth: 3,
            ..nominal
        },
        RingSpec {
            teeth: 129,
            ..nominal
        },
        RingSpec {
            base_radius_mm: f64::NAN,
            ..nominal
        },
        RingSpec {
            root_fillet_radius_mm: 0.,
            ..nominal
        },
        RingSpec {
            half_base_angle_rad: f64::INFINITY,
            ..nominal
        },
        RingSpec {
            base_radius_mm: 23.,
            ..nominal
        },
        RingSpec {
            tip_radius_mm: 24.,
            ..nominal
        },
        RingSpec {
            root_radius_mm: 24.,
            ..nominal
        },
        RingSpec {
            outer_radius_mm: 25.25 + 5e-6,
            ..nominal
        },
        RingSpec {
            root_fillet_radius_mm: 3.,
            ..nominal
        },
        // Pointed inward tip, then a tooth that fills the whole pitch.
        RingSpec {
            half_base_angle_rad: -start,
            ..nominal
        },
        RingSpec {
            half_base_angle_rad: PI / 48.,
            ..nominal
        },
    ] {
        assert_eq!(ring(invalid), Err(Error::InvalidDimensions), "{invalid:?}");
    }
    for invalid in [
        // The fillet foot lies inside the tip circle.
        RingSpec {
            teeth: 24,
            pitch_radius_mm: 12.5,
            base_radius_mm: 10.,
            tip_radius_mm: 12.,
            root_radius_mm: 13.1,
            root_fillet_radius_mm: 3.,
            half_base_angle_rad: 0.02,
            face_width_mm: 2.,
            outer_radius_mm: 15.,
        },
        // Flanks cross before the outward root: no separated root space.
        RingSpec {
            half_base_angle_rad: PI / 48. - start - 1e-3,
            ..nominal
        },
    ] {
        assert_eq!(ring(invalid), Err(Error::InvalidProfile), "{invalid:?}");
    }
    // An involute beyond one half turn exceeds the exact flank domain.
    let deep = RingSpec {
        teeth: 4,
        pitch_radius_mm: 1.1,
        base_radius_mm: 1.,
        tip_radius_mm: 1.05,
        root_radius_mm: 4.5,
        root_fillet_radius_mm: 0.1,
        half_base_angle_rad: 0.3,
        face_width_mm: 1.,
        outer_radius_mm: 5.,
    };
    assert_eq!(ring(deep), Err(Error::Limit));
}
