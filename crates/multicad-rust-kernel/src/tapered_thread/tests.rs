use super::*;
pub(super) fn spec(starts: u32, left_hand: bool) -> Spec {
    Spec {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: 8.,
        depth_mm: 0.5,
        crest_width_mm: 0.25,
        lower_flank_deg: 15.,
        upper_flank_deg: 15.,
        starts,
        left_hand,
        root_radius_mm: None,
    }
}
#[test]
fn matches_fresh_native_boundary_witnesses_in_mm() {
    for row in include_str!("tests/fixtures/native_transport.csv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let v: Vec<f64> = row.split(',').map(|s| s.parse().unwrap()).collect();
        let t = Thread::new(spec(v[1] as u32, false), v[0], 1e-7, 1e-6).unwrap();
        let [lo, hi] = t.radial_bounds(v[3], v[2], 1e-10).unwrap();
        assert!(hi - lo <= 1e-10);
        assert!(v[4] >= lo - 1e-7 && v[4] <= hi + 1e-7, "{row}: [{lo},{hi}]");
    }
}
#[test]
fn preserves_straight_limit_hand_starts_and_cancellation() {
    for starts in [1, 3, 8] {
        for left in [false, true] {
            let t = Thread::new(spec(starts, left), 0., 1e-7, 1e-6).unwrap();
            let straight = StraightThread::new(spec(starts, left)).unwrap();
            for i in 0..100 {
                let phase = 1.25 * f64::from(i) / 100.;
                let [a, b] = t.radial_bounds(phase, 3., 1e-10).unwrap();
                let expected = straight.radius_at_phase(phase);
                assert!(a <= expected + 1e-12 && b >= expected - 1e-12);
            }
            assert_eq!(
                t.radial_bounds_with_interrupt(0.2, 2., 1e-7, Some(&|| true)),
                Err(Error::Cancelled)
            );
        }
    }
    assert_eq!(
        Thread::new_with_interrupt(spec(1, false), 0.1, 1e-7, 1e-6, Some(&|| true)),
        Err(Error::Cancelled)
    );
}
#[test]
fn invalid_source_queries_and_unproved_profile_refuse() {
    let s = spec(1, false);
    assert_eq!(Thread::new(s, f64::NAN, 1e-7, 1e-6), Err(Error::Invalid));
    assert_eq!(Thread::new(s, 0.11, 1e-7, 1e-6), Err(Error::Invalid));
    assert_eq!(
        Thread::new(
            Spec {
                root_radius_mm: Some(0.1),
                ..s
            },
            0.1,
            1e-7,
            1e-6
        ),
        Err(Error::Unsupported)
    );
    assert_eq!(
        Thread::new(
            Spec {
                length_mm: 1000.,
                ..s
            },
            0.1,
            1e-7,
            1e-6
        ),
        Err(Error::Limit)
    );
    let t = Thread::new(s, 0.1, 1e-7, 1e-6).unwrap();
    assert_eq!(t.radial_bounds(1.25, 2., 1e-7), Err(Error::Invalid));
    assert_eq!(t.radial_bounds(0., 9., 1e-7), Err(Error::Invalid));
    assert_eq!(t.radial_bounds(0., 2., 0.), Err(Error::Invalid));
}
#[test]
fn mesh_has_one_shared_closed_oriented_boundary() {
    use crate::display::Tessellation;
    use std::collections::BTreeMap;
    for left in [false, true] {
        let thread = Thread::new(spec(1, left), -0.1, 1e-7, 1e-6).unwrap();
        let mesh = tessellate(
            &thread,
            Tessellation {
                deflection: 0.05,
                angle: 0.35,
                triangle_limit: 2_000_000,
            },
        )
        .unwrap();
        let mut edges = BTreeMap::new();
        let mut volume = 0.;
        for tri in mesh.indices.as_chunks::<3>().0 {
            for i in 0..3 {
                let a = tri[i];
                let b = tri[(i + 1) % 3];
                let key = (a.min(b), a.max(b));
                let entry = edges.entry(key).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
            let a = &mesh.vertices[3 * tri[0] as usize..][..3];
            let b = &mesh.vertices[3 * tri[1] as usize..][..3];
            let c = &mesh.vertices[3 * tri[2] as usize..][..3];
            volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.;
        }
        assert!(
            edges
                .values()
                .all(|&(count, direction)| count == 2 && direction == 0)
        );
        assert!(volume > 200. && volume < 350., "{volume}");
        assert_eq!(mesh.face_indices.len() * 3, mesh.indices.len());
    }
}
#[test]
fn volume_is_validated_from_material_not_mesh() {
    let source = spec(1, false);
    let t = Thread::new(source, 0., 1e-7, 1e-6).unwrap();
    let [lo, hi] = t.volume_bounds(0.02).unwrap();
    let v = StraightThread::new(source).unwrap().volume_mm3();
    assert!(lo <= v && hi >= v && hi - lo <= 0.02, "{lo} {v} {hi}");
    assert_eq!(
        t.volume_bounds_with_interrupt(0.01, Some(&|| true)),
        Err(Error::Cancelled)
    );
}
#[test]
fn interval_indeterminate_products_fail_closed() {
    let unbounded = I::new(f64::NEG_INFINITY, f64::INFINITY);
    let zero = I::point(0.) * unbounded;
    assert!(!zero.valid() && zero.lo == f64::NEG_INFINITY && zero.hi == f64::INFINITY);
    let zero_div = I::point(0.) / (I::new(-1., 1.));
    assert!(!(zero_div * I::point(0.)).valid());
    let x = J::d(I::new(-1., 1.));
    let y = x / J::constant(0.);
    assert!(!y.v.valid() && !y.d.valid());
}
#[test]
fn tapered_volume_matches_independent_native_solid_integration() {
    let t = Thread::new(spec(1, false), -0.1, 1e-7, 1e-6).unwrap();
    let [lo, hi] = t.volume_bounds(0.002).unwrap();
    let native = 268.50468219835;
    assert!(
        lo <= native && native <= hi && hi - lo <= 0.002,
        "[{lo},{hi}], Native{native}"
    );
}
#[test]
fn support_bounds_enclose_every_retained_wall_and_both_caps() {
    let t = Thread::new(spec(3, true), 0.1, 1e-7, 1e-6).unwrap();
    for d in [
        [0., 0., 0.],
        [0., 0., 1.],
        [0., 0., -1.],
        [1., 0.3, 0.2],
        [-0.2, -1., -0.4],
    ] {
        let [lo, hi] = t.support_bounds(d, 1e-4).unwrap();
        assert!(hi - lo <= 1e-4);
        for band in 0..4 {
            for iz in 0..=8 {
                for iu in 0..=8 {
                    let p = t
                        .surface_point_bounds(band, f64::from(iu) / 8., f64::from(iz))
                        .unwrap()
                        .map(|p| (p[0] + p[1]) * 0.5);
                    let dot = p.into_iter().zip(d).map(|(p, d)| p * d).sum::<f64>();
                    assert!(dot <= hi + 1e-9, "{d:?}, {dot}, {hi}");
                }
            }
        }
    }
    assert_eq!(
        t.support_bounds_with_interrupt([1., 0., 0.], 1e-4, Some(&|| true)),
        Err(Error::Cancelled)
    );
}
#[test]
fn multistart_asymmetric_patches_obey_chord_normals_and_real_volume() {
    use crate::display::Tessellation;
    use std::f64::consts::TAU;
    for (starts, left, k) in [(1, false, 0.1), (3, true, -0.03125), (8, false, 0.)] {
        let source = Spec {
            lower_flank_deg: 10.,
            upper_flank_deg: 20.,
            ..spec(starts, left)
        };
        let t = Thread::new(source, k, 1e-7, 1e-6).unwrap();
        let options = Tessellation {
            deflection: 0.1,
            angle: 0.5,
            triangle_limit: 4_000_000,
        };
        let mesh = tessellate(&t, options).unwrap();
        let proof = crate::manufacturing::certify(&mesh).unwrap();
        assert!(proof.signed_volume_mm3 > 0.);
        let vertices = mesh.vertices.as_chunks::<3>().0;
        let ring = vertices.iter().take_while(|p| p[2] == 0.).count();
        let nz = (vertices.len() - 2) / ring - 1;
        let nu = std::array::from_fn::<_, 4, _>(|band| {
            mesh.face_indices
                .iter()
                .filter(|&&face| face == 2 + band as u32)
                .count()
                / (2 * nz)
        });
        let per: usize = nu.into_iter().sum();
        let offsets = [0, nu[0], nu[0] + nu[1], nu[0] + nu[1] + nu[2]];
        for (&face, tri) in mesh
            .face_indices
            .iter()
            .zip(mesh.indices.as_chunks::<3>().0)
            .step_by(131)
        {
            if face < 2 {
                continue;
            }
            let start = (face - 2) / 4;
            let band = ((face - 2) % 4) as usize;
            let offset = start as usize * per + offsets[band];
            let points = tri.map(|i| vertices[i as usize]);
            let u = tri
                .map(|i| ((i as usize % ring + ring - offset) % ring) as f64 / nu[band] as f64)
                .into_iter()
                .sum::<f64>()
                / 3.;
            let z = points.into_iter().map(|p| p[2]).sum::<f64>() / 3.;
            let p = t.patch(band, I::point(u), I::point(z)).unwrap();
            let rotation =
                -TAU * f64::from(start) / f64::from(starts) * if left { -1. } else { 1. };
            let (s, c) = rotation.sin_cos();
            let rotate = |v: [f64; 3]| [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]];
            let exact = rotate(p.map(|p| p.v.midpoint()));
            let bary: [f64; 3] =
                std::array::from_fn(|i| points.into_iter().map(|p| p[i]).sum::<f64>() / 3.);
            let error = exact
                .into_iter()
                .zip(bary)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(
                error <= options.deflection,
                "{starts} {left} band{band} error{error}"
            );
            let a = std::array::from_fn::<_, 3, _>(|i| points[1][i] - points[0][i]);
            let b = std::array::from_fn::<_, 3, _>(|i| points[2][i] - points[0][i]);
            let cross = |a: [f64; 3], b: [f64; 3]| {
                [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ]
            };
            let n = cross(a, b);
            let mut expected = rotate(cross(p.map(|p| p.d.midpoint()), p.map(|p| p.z.midpoint())));
            if !left {
                expected = expected.map(|v| -v)
            }
            let dot = n.into_iter().zip(expected).map(|(a, b)| a * b).sum::<f64>();
            let norm = |v: [f64; 3]| v.into_iter().map(|x| x * x).sum::<f64>().sqrt();
            assert!(dot / (norm(n) * norm(expected)) >= options.angle.cos() - 1e-10);
        }
    }
}
#[test]
fn display_failure_and_cancellation_are_bounded_and_retryable() {
    use crate::display::{MeshError, Tessellation};
    let t = Thread::new(spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
    let options = Tessellation {
        deflection: 0.1,
        angle: 0.5,
        triangle_limit: 4_000_000,
    };
    assert_eq!(
        tessellate(
            &t,
            Tessellation {
                deflection: 0.,
                ..options
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        tessellate(
            &t,
            Tessellation {
                triangle_limit: 1,
                ..options
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        tessellate(
            &t,
            Tessellation {
                deflection: 1e-18,
                ..options
            }
        ),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        tessellate_with_interrupt(&t, options, Some(&|| true)),
        Err(Error::Cancelled)
    );
    assert_eq!(
        tessellate(
            &t,
            Tessellation {
                angle: 1e-100,
                triangle_limit: usize::MAX,
                ..options
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert!(tessellate(&t, options).is_ok());
}
#[test]
fn retained_source_metadata_and_seam_receipts_are_explicit() {
    let t = Thread::new(spec(3, true), -0.03125, 1e-7, 1e-6).unwrap();
    assert_eq!(t.spec(), spec(3, true));
    assert_eq!(t.radial_taper(), -0.03125);
    assert_eq!(t.tolerance_mm(), 1e-7);
    assert_eq!(t.minimum_feature_mm(), 1e-6);
    let stations = t.profile_stations();
    assert!(stations.windows(2).all(|w| w[0] < w[1]));
    for positive in [false, true] {
        for z in [0., 3., 8.] {
            let p = t.seam_point_bounds(positive, z).unwrap();
            let radius = t.root_radius_at(z).unwrap();
            let r = ((p[0][0] + p[0][1]) * 0.5).hypot((p[1][0] + p[1][1]) * 0.5);
            assert!(r >= radius[0] - 1e-10 && r <= radius[1] + 1e-10);
        }
    }
    assert_eq!(t.seam_point_bounds(false, f64::NAN), Err(Error::Invalid));
    assert_eq!(t.surface_point_bounds(4, 0., 0.), Err(Error::Invalid));
    assert_eq!(t.surface_point_bounds(0, -0.1, 0.), Err(Error::Invalid));
    assert_eq!(t.radial_bounds(0.1, 2., 1e-18), Err(Error::Unresolved));
    assert_eq!(t.volume_bounds(0.), Err(Error::Invalid));
    assert_eq!(
        t.support_bounds([f64::NAN, 0., 0.], 0.1),
        Err(Error::Invalid)
    );
    assert_eq!(
        t.support_bounds([0., 0., 1.], 1e-20),
        Err(Error::Unresolved)
    );
    let mut work = Work {
        remaining: 0,
        interrupt: None,
    };
    assert_eq!(work.step(), Err(Error::Limit));
    let mut work = Work {
        remaining: 100,
        interrupt: None,
    };
    assert_eq!(
        t.prove(0, I::new(-10., 10.), I::new(0., 8.), 24, &mut work),
        Err(Error::Unresolved)
    );
    assert_eq!(
        t.prove_station(0., false, I::new(0., 8.), 24, &mut work),
        Err(Error::Unresolved)
    );
}
#[test]
fn extended_cone_source_policy_and_strong_transport_are_bounded() {
    let source = spec(1, false);
    for (s, k, tol, min) in [
        (
            Spec {
                major_radius_mm: 1.,
                depth_mm: 0.9,
                ..source
            },
            0.1,
            1e-7,
            1e-6,
        ),
        (source, 0., 0.1, 1e-6),
        (source, 0., 1e-7, 0.5),
        (
            Spec {
                major_radius_mm: 1e7,
                length_mm: 10.,
                depth_mm: 0.5,
                ..source
            },
            0.1,
            1e-7,
            1e-6,
        ),
        (
            Spec {
                depth_mm: 3.,
                lower_flank_deg: 80.,
                ..source
            },
            0.,
            1e-7,
            1e-6,
        ),
        (
            Spec {
                pitch_mm: 0.0001,
                ..source
            },
            0.,
            1e-7,
            1e-6,
        ),
    ] {
        assert!(Thread::new(s, k, tol, min).is_err());
    }
    for source in [
        Spec {
            major_radius_mm: 2.,
            pitch_mm: 1.,
            length_mm: 10.,
            depth_mm: 1.5,
            crest_width_mm: 0.05,
            lower_flank_deg: 5.,
            upper_flank_deg: 5.,
            ..source
        },
        Spec {
            pitch_mm: 1.,
            length_mm: 100.,
            depth_mm: 1.5,
            crest_width_mm: 0.05,
            lower_flank_deg: 5.,
            upper_flank_deg: 5.,
            starts: 8,
            ..source
        },
    ] {
        let t = Thread::new(source, 0.1, 1e-7, 1e-6).unwrap();
        for phase in [0., 0.1, 0.5, 0.9] {
            let [lo, hi] = t
                .radial_bounds(phase, source.length_mm * 0.9, 1e-8)
                .unwrap();
            assert!(lo > 0. && hi - lo <= 1e-8);
        }
    }
}
#[test]
fn numeric_receipts_cover_arithmetic_failures_and_precision_exhaustion() {
    let whole = I::new(f64::NEG_INFINITY, f64::INFINITY);
    assert_eq!(whole.checked().unwrap_err(), Error::Unresolved);
    assert!(
        J {
            v: whole,
            ..J::constant(0.)
        }
        .checked()
        .is_err()
    );
    assert_eq!(I::new(-1., 1.).positive().unwrap_err(), Error::Unresolved);
    assert!(I::point(1.).positive().is_ok());
    assert_eq!(I::point(1.).bounded_by(0.5), Err(Error::Unresolved));
    assert_eq!(
        I::new(1., 1f64.next_up()).split().unwrap_err(),
        Error::Unresolved
    );
    let [left, right] = I::new(-1., 1.).split().unwrap();
    assert_eq!(left.hi, right.lo);
    assert_eq!(I::new(-4., 4.).sin().lo, -1.);
    assert_eq!(I::new(-4., 4.).cos().hi, 1.);
    let j = J::d(I::point(0.3)).atan();
    assert!(j.dd.lo <= -0.6 / 1.09f64.powi(2) && j.dd.hi >= -0.6 / 1.09f64.powi(2));
    let t = Thread::new(spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
    assert!(t.root_station(1, I::point(0.)).is_err());
}
#[test]
fn complete_profile_proofs_refuse_unburied_or_uncrossed_continuations() {
    let t = Thread::new(spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
    let mut work = Work {
        remaining: 10000,
        interrupt: None,
    };
    assert_eq!(
        t.prove(2, I::new(0.125, 10.), I::point(0.), 0, &mut work),
        Err(Error::Unresolved)
    );
    let root = t.root_station(2, I::point(4.)).unwrap().v.midpoint();
    let inside = t
        .transport(2, I::point(root), I::point(0.))
        .excess
        .v
        .midpoint()
        > 0.;
    let mut work = Work {
        remaining: 10000,
        interrupt: None,
    };
    assert!(
        t.prove_station(root, inside, I::new(0., 8.), 0, &mut work)
            .is_err()
    );
    let straight = Thread::new(spec(1, false), 0., 1e-7, 1e-6).unwrap();
    assert!(straight.root_station(1, I::point(0.)).is_err());
    assert_eq!(
        Thread::new(
            Spec {
                major_radius_mm: 0.76,
                ..spec(1, false)
            },
            0.1,
            1e-7,
            1e-6
        ),
        Err(Error::Invalid)
    );
    assert_eq!(
        Thread::new(
            Spec {
                depth_mm: 1.85,
                ..spec(1, false)
            },
            0.,
            1e-7,
            1e-6
        ),
        Err(Error::Invalid)
    );
    assert!(
        [1e-14, 5e-15, 3e-15, 2e-15]
            .into_iter()
            .any(|accuracy| t.radial_bounds(0.21, 2., accuracy) == Err(Error::Limit))
    );
    assert!(t.support_bounds([1., 0., 0.], 1e-8).is_ok());
}
#[test]
fn retained_stock_source_contours_keep_exact_dimensions_and_feature() {
    use crate::{
        Rational,
        prism::{BoundaryCurve, Prism},
    };
    let n = |v| Rational::new(v, 1).unwrap();
    let stock = Prism::rectangular(42, [n(12), n(10), n(8)]).unwrap();
    assert_eq!(stock.feature(), 42);
    assert_eq!(
        stock.outer_loop(),
        [
            BoundaryCurve::Segment {
                from: [n(0), n(0)],
                to: [n(12), n(0)]
            },
            BoundaryCurve::Segment {
                from: [n(12), n(0)],
                to: [n(12), n(10)]
            },
            BoundaryCurve::Segment {
                from: [n(12), n(10)],
                to: [n(0), n(10)]
            },
            BoundaryCurve::Segment {
                from: [n(0), n(10)],
                to: [n(0), n(0)]
            },
        ]
    );
    let rounded = Prism::extrude(43, n(12), n(5), n(8)).unwrap();
    assert_eq!(rounded.feature(), 43);
    assert!(matches!(
        rounded.outer_loop()[1],
        BoundaryCurve::RightSemicircle(_)
    ));
}
