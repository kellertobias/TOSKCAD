use multicad_core::document::EntityId;
use multicad_kernel::{
    AxialFlatHelicalThread, BooleanOperation, HelicalThread, RigidTransform, Solid,
};

fn material(solid: &Solid, point: [f64; 3]) -> f64 {
    let edge = 0.0002;
    let cube = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|x| x - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / edge.powi(3)
}
#[test]
fn axial_npt_flanks_and_stock_clipped_crests_preserve_gauge_cones_and_native_roles() {
    for (tpi, e1, l1, l2, od, truncation) in [
        (27, 0.37360, 0.1615, 0.2639, 0.405, 0.0645),
        (18, 0.49163, 0.2278, 0.4018, 0.540, 0.0605),
    ] {
        let pitch = 25.4 / f64::from(tpi);
        let m = 3_f64.sqrt();
        let fundamental = pitch * (m * m - (1_f64 / 32.).powi(2)) / (2. * m);
        let depth = fundamental - 2. * truncation * pitch;
        let width = truncation * pitch * (1. / (m - 1. / 32.) + 1. / (m + 1. / 32.));
        for internal in [false, true] {
            let k = if internal { -1. / 32. } else { 1. / 32. };
            let base_pitch = e1 * 25.4 - if internal { 0. } else { l1 * 25.4 / 16. };
            let length = if internal {
                l1 * 25.4 + 3. * pitch
            } else {
                l2 * 25.4
            };
            let spec = AxialFlatHelicalThread {
                profile: HelicalThread {
                    major_radius_mm: (base_pitch + depth) / 2.,
                    pitch_mm: pitch,
                    length_mm: length,
                    depth_mm: depth,
                    crest_width_mm: width,
                    lower_flank_deg: 30.,
                    upper_flank_deg: 30.,
                    radial_taper: k,
                    starts: 1,
                    left_hand: false,
                },
                crest_radius_limit_mm: (!internal).then_some(od * 25.4 / 2.),
            };
            let feature = EntityId::new();
            let solid = Solid::axial_flat_helical_thread_named(spec, feature, Default::default())
                .unwrap_or_else(|error| panic!("{tpi}TPI internal={internal} {spec:?}: {error}"));
            assert_eq!(solid.solid_count().unwrap(), 1);
            // Integrate squared radial sections over phase and then along the
            // axis. This oracle does not use the native sweep or its mesh.
            let slopes = 1. / (m - k) + 1. / (m + k);
            let root_width = pitch - width - depth * slopes;
            let area = |z: f64| {
                let major = spec.profile.major_radius_mm + k * z;
                let root = major - depth;
                let ceiling = spec
                    .crest_radius_limit_mm
                    .map_or(major, |limit| major.min(limit));
                let flanks =
                    (ceiling.powi(3) - root.powi(3)) / 3. + (major - ceiling) * ceiling.powi(2);
                std::f64::consts::PI
                    * (width * ceiling.powi(2) + root_width * root.powi(2) + slopes * flanks)
                    / pitch
            };
            let mut cuts = vec![0., length];
            if let Some(limit) = spec.crest_radius_limit_mm {
                let z = (limit - spec.profile.major_radius_mm) / k;
                if z > 0. && z < length {
                    cuts.insert(1, z);
                }
            }
            let volume = cuts
                .windows(2)
                .map(|interval| {
                    let mid = (interval[0] + interval[1]) / 2.;
                    let half = (interval[1] - interval[0]) / 2.;
                    half * (area(mid - half / 3_f64.sqrt()) + area(mid + half / 3_f64.sqrt()))
                })
                .sum::<f64>();
            let measured = solid.volume_mm3().unwrap();
            let surface: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            let budget = multicad_kernel::GeometryPolicy::default().linear_tolerance_mm * surface
                + 1e-7 * volume;
            assert!(
                (measured - volume).abs() < budget,
                "{tpi}/{internal}: native {measured}, independent {volume}, budget {budget}"
            );
            let bounds = solid.bounds().unwrap();
            assert!(bounds.min[2].abs() < 16e-7);
            assert!((bounds.max[2] - length).abs() < 16e-7);
            // Independent axial 60-degree lines at two distances from each
            // cone-parallel crest flat. A sheared 30-degree profile fails.
            let center = 2. * pitch;
            for sign in [-1., 1.] {
                let start = center + sign * width / 2.;
                let radius = spec.profile.major_radius_mm + k * start;
                for distance in [0.12 * pitch, 0.28 * pitch] {
                    let z = start + sign * distance;
                    let boundary = radius - m * distance;
                    assert!(material(&solid, [boundary - 0.001, 0., z]) > 0.999);
                    assert!(material(&solid, [boundary + 0.001, 0., z]) < 1e-5);
                }
            }
            if let Some(limit) = spec.crest_radius_limit_mm {
                assert!(bounds.max[0] <= limit + 16e-7);
                let transition = (limit - spec.profile.major_radius_mm) / k;
                assert!(transition > 0. && transition < length);
                for z in [transition - pitch / 2., (transition + length) / 2.] {
                    let angle = std::f64::consts::TAU * z / pitch;
                    let boundary = (spec.profile.major_radius_mm + k * z).min(limit);
                    for (offset, inside) in [(-0.001, true), (0.001, false)] {
                        let p = [
                            (boundary + offset) * angle.cos(),
                            (boundary + offset) * angle.sin(),
                            z,
                        ];
                        let fraction = material(&solid, p);
                        assert!(
                            if inside {
                                fraction > 0.999
                            } else {
                                fraction < 1e-5
                            },
                            "stock transition {tpi}/{z}/{offset}: {fraction}"
                        );
                    }
                }
            }
            let topology = solid.topology_snapshot(feature, 0).unwrap();
            assert!(topology.elements.iter().all(|e| !e.origins.is_empty()));
            assert!(topology.elements.iter().any(|e| {
                e.origins
                    .iter()
                    .any(|origin| origin.source == feature && origin.role == "thread/start/0/crest")
            }));
        }
    }
}
