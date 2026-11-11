use multicad_core::document::EntityId;
use multicad_kernel::{
    BlindThreadCut, GeometryError, GeometryErrorKind, GeometryPolicy, HelicalThread, MeshOptions,
    PlaneFrame, RigidTransform, Solid,
    rust_kernel::{
        Circle, Rational,
        cylinder::AxialCylinder,
        headed_screw::{ScrewHead, ScrewSpec},
    },
};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Shaft,
    Hex,
    Socket,
    Nut,
    Receiver,
    Blind,
    Shank,
}
const KINDS: [Kind; 7] = [
    Kind::Shaft,
    Kind::Hex,
    Kind::Socket,
    Kind::Nut,
    Kind::Receiver,
    Kind::Blind,
    Kind::Shank,
];
fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}
fn profile() -> HelicalThread {
    HelicalThread {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 3.7,
        depth_mm: 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    }
}
fn noisy(error: f64) -> PlaneFrame {
    PlaneFrame {
        u: [1. + error, 0., 0.],
        ..Default::default()
    }
}
fn pose(frame: PlaneFrame) -> RigidTransform {
    let normal = frame.normal();
    RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], normal[i]]),
        translation_mm: frame.origin,
    }
}
fn make(kind: Kind, extent: i64, frame: PlaneFrame) -> Result<Solid, GeometryError> {
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let mut thread = profile();
    match kind {
        Kind::Shaft => {
            thread.major_radius_mm = extent as f64;
            Solid::rust_helical_thread(thread, None, id, frame, policy)
        }
        Kind::Hex | Kind::Socket | Kind::Shank => Solid::rust_machine_screw(
            thread,
            None,
            ScrewSpec {
                shank_length_mm: if matches!(kind, Kind::Shank) {
                    extent as f64
                } else {
                    0.
                },
                shank_radius_mm: 3.,
                head: if matches!(kind, Kind::Socket) {
                    ScrewHead::SocketCap {
                        radius_mm: extent as f64,
                        height_mm: 4.,
                        socket_across_flats_mm: 4.,
                        socket_depth_mm: 2.,
                    }
                } else {
                    ScrewHead::ExternalHex {
                        across_flats_mm: if matches!(kind, Kind::Shank) {
                            12.
                        } else {
                            extent as f64
                        },
                        height_mm: 4.,
                    }
                },
            },
            id,
            frame,
            policy,
        ),
        Kind::Nut => Solid::rust_hex_nut(thread, extent as f64, 1., id, frame, policy),
        Kind::Receiver => {
            let cylinder = AxialCylinder::new(
                EntityId::new().uuid_bits(),
                Circle::new([n(0); 2], n(extent)).unwrap(),
                n(0),
                Rational::new(37, 10).unwrap(),
            )
            .unwrap();
            Solid::from_axial_cylinder(cylinder, frame, policy)?
                .rust_cut_internal_thread(thread, 1., id, frame, policy)
        }
        Kind::Blind => {
            // The complete retained stock extends far from this small tap.
            let stock = Solid::rust_box(
                EntityId::new().uuid_bits(),
                [n(extent), n(20), n(20)],
                frame,
                policy,
            )?;
            let point = |p: [f64; 3]| {
                std::array::from_fn(|i| {
                    frame.origin[i]
                        + frame.u[i] * p[0]
                        + frame.v[i] * p[1]
                        + frame.normal()[i] * p[2]
                })
            };
            stock.rust_cut_blind_internal_thread(
                BlindThreadCut {
                    profile: thread,
                    entry: PlaneFrame {
                        origin: point([10., 10., 20.]),
                        u: frame.u,
                        v: frame.v.map(|v| -v),
                    },
                    receiving_radius_mm: 3.,
                    minimum_wall_mm: 1.,
                    minimum_floor_mm: 1.,
                },
                id,
                policy,
            )
        }
    }
}

#[test]
fn helical_metric_guards_cover_heads_shanks_receivers_and_full_blind_stock() {
    let transform = pose(noisy(4e-11));
    let proper = pose(PlaneFrame {
        origin: [7., -3., 2.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    });
    for kind in KINDS {
        let source = make(kind, 1_000_000, PlaneFrame::default())
            .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        let owner = EntityId::new();
        let before = source.topology_snapshot(owner, 0).unwrap();
        let bounds = source.bounds().unwrap();
        let volume = source.volume_mm3().unwrap();
        assert_eq!(
            source.transformed(transform).err().unwrap().kind,
            GeometryErrorKind::Unsupported,
            "{kind:?}"
        );
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
        assert_eq!(source.bounds().unwrap(), bounds);
        assert_eq!(source.volume_mm3().unwrap(), volume);
        assert_eq!(
            source.transformed(proper).unwrap().volume_mm3().unwrap(),
            volume
        );
        if !matches!(kind, Kind::Receiver | Kind::Blind) {
            // Receiving stock constructors already prove their own frame;
            // the transform above independently checks the final thread body.
            assert_eq!(
                make(kind, 1_000_000, noisy(4e-11)).err().unwrap().kind,
                GeometryErrorKind::Unsupported,
                "{kind:?}"
            );
        }
    }
    let source = make(Kind::Shaft, 800, PlaneFrame::default()).unwrap();
    let first = source.transformed(pose(noisy(2e-11))).unwrap();
    let bounds = first.bounds().unwrap();
    assert_eq!(
        first.transformed(pose(noisy(2e-11))).err().unwrap().kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(first.bounds().unwrap(), bounds);
}

#[test]
fn small_helical_frames_keep_real_mesh_and_supported_step_closures() {
    let scratch = tempfile::tempdir().unwrap();
    let options = MeshOptions {
        linear_deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
        maximum_triangles: 100_000,
    };
    for kind in KINDS {
        let small = if matches!(kind, Kind::Receiver | Kind::Blind) {
            // These features derive their cap frame from receiving stock;
            // test the retained body chart rather than that separate remap.
            make(kind, 20, PlaneFrame::default())
                .unwrap()
                .transformed(pose(noisy(4e-11)))
                .unwrap()
        } else {
            make(kind, 20, noisy(4e-11)).unwrap_or_else(|e| panic!("{kind:?}: {e}"))
        };
        let before = small.bounds().unwrap();
        let mesh = small
            .manufacturing_mesh_with_receipt(options)
            .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
        let (points, remainder) = mesh.mesh.vertices.as_chunks::<3>();
        assert!(remainder.is_empty());
        for p in points {
            for (i, coordinate) in p.iter().enumerate() {
                assert!(
                    *coordinate >= before.min[i] - 1e-7 && *coordinate <= before.max[i] + 1e-7,
                    "{kind:?}"
                );
            }
        }
        assert_eq!(small.bounds().unwrap(), before);
        let proper = make(kind, 20, PlaneFrame::default())
            .unwrap()
            .transformed(pose(PlaneFrame {
                origin: [7., -3., 2.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            }))
            .unwrap();
        if proper.supports_step_export() {
            let path = scratch.path().join(format!("{kind:?}.step"));
            proper.write_step(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let verified = proper.verify_helical_step(&text).unwrap().unwrap();
            assert!((verified / proper.volume_mm3().unwrap() - 1.).abs() < 1e-7);
            // Partial-shank independent inference is a separate provider stage;
            // its known-source complete STEP shell is nevertheless proved.
            if !matches!(kind, Kind::Shank) {
                let imported =
                    Solid::import_rust_step(&text, EntityId::new(), GeometryPolicy::default())
                        .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
                assert!(
                    (imported.volume_mm3().unwrap() / proper.volume_mm3().unwrap() - 1.).abs()
                        < 1e-7
                );
            }
            #[cfg(feature = "occt")]
            {
                let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                assert!(
                    (native.volume_mm3().unwrap() / proper.volume_mm3().unwrap() - 1.).abs() < 1e-7
                );
                let expected = proper.bounds().unwrap();
                let actual = native.bounds().unwrap();
                for (a, b) in expected
                    .min
                    .into_iter()
                    .chain(expected.max)
                    .zip(actual.min.into_iter().chain(actual.max))
                {
                    assert!((a - b).abs() < 5e-6, "{kind:?}: {a} vs {b}");
                }
            }
        }
    }
}
