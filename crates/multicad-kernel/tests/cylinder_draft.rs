use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};
use std::f64::consts::PI;

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}
fn pose() -> PlaneFrame {
    PlaneFrame {
        origin: [17., -8., 13.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    }
}
fn source(owner: EntityId, frame: PlaneFrame, reverse: Option<bool>) -> Solid {
    let cylinder = AxialCylinder::new(
        owner.uuid_bits(),
        Circle::new([n(0); 2], n(10)).unwrap(),
        n(0),
        n(20),
    )
    .unwrap();
    if let Some(reverse) = reverse {
        Solid::from_axial_cylinder_sweep(
            cylinder,
            frame,
            [3., -2.],
            reverse,
            Origin {
                source: owner,
                role: "circle".into(),
            },
            Default::default(),
        )
        .unwrap()
    } else {
        Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), Default::default())
            .unwrap()
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            })
            .unwrap()
    }
}
fn wall(stock: &Solid, feature: EntityId) -> TreatmentFace {
    let face = stock
        .faces()
        .unwrap()
        .into_iter()
        .find(|face| face.plane.is_none())
        .unwrap();
    TreatmentFace {
        key: TopologyKey {
            kind: TopologyKind::Face,
            ordinal: face.index,
        },
        origin: Origin {
            source: feature,
            role: "draft.wall".into(),
        },
    }
}
fn check(solid: &Solid, frame: PlaneFrame, radii: [f64; 2]) -> f64 {
    let [a, b] = radii;
    let expected = PI * 20. / 3. * (a * a + a * b + b * b);
    near(solid.volume_mm3().unwrap(), expected);
    near(
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum(),
        PI * ((a + b) * (400. + (a - b) * (a - b)).sqrt() + a * a + b * b),
    );
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    for i in 0..3 {
        let radial = frame.u[i].hypot(frame.v[i]);
        near(
            bounds.min[i],
            (frame.origin[i] - a * radial)
                .min(frame.origin[i] + 20. * frame.normal()[i] - b * radial),
        );
        near(
            bounds.max[i],
            (frame.origin[i] + a * radial)
                .max(frame.origin[i] + 20. * frame.normal()[i] + b * radial),
        );
    }
    expected
}

#[test]
fn cylinder_draft_exact_taper_matches_native_for_signed_angles_pulls_and_neutral_offsets() {
    let policy = GeometryPolicy::default();
    for frame in [PlaneFrame::default(), pose()] {
        #[cfg(feature = "occt")]
        let native = Solid::cylinder(frame, 10., 20., policy).unwrap();
        for reverse in [None, Some(false), Some(true)] {
            let owner = EntityId::new();
            let stock = source(owner, frame, reverse);
            let before = stock.topology_snapshot(owner, 0).unwrap();
            for sense in [-1., 1.] {
                for degrees in [-5_f64, 5.] {
                    for level in [0., 7.] {
                        let feature = EntityId::new();
                        let selection = wall(&stock, feature);
                        let angle = degrees.to_radians();
                        let neutral = PlaneFrame {
                            origin: std::array::from_fn(|i| {
                                frame.origin[i] + level * frame.normal()[i]
                            }),
                            ..frame
                        };
                        let pull = frame.normal().map(|v| v * sense);
                        let result = stock
                            .draft_faces_named(
                                std::slice::from_ref(&selection),
                                pull,
                                neutral,
                                angle,
                                feature,
                                policy,
                            )
                            .unwrap();
                        let radii = [
                            10. + sense * angle.tan() * level,
                            10. - sense * angle.tan() * (20. - level),
                        ];
                        check(&result, frame, radii);
                        let output = result.topology_snapshot(feature, 1).unwrap();
                        output.validate().unwrap();
                        for old in &before.elements {
                            let mut intent = GeometryIntent::capture(&before, old.key).unwrap();
                            if old.key.kind == TopologyKind::Edge {
                                intent.boundary = Some(old.boundary.clone());
                            }
                            let Resolution::Resolved { key, .. } = resolve(&intent, &output, 1)
                            else {
                                panic!("lost cylinder reference {intent:?}")
                            };
                            let new = output
                                .elements
                                .iter()
                                .find(|element| element.key == key)
                                .unwrap();
                            assert!(old.origins.is_subset(&new.origins));
                            if old.key.kind == TopologyKind::Edge {
                                assert_eq!(old.boundary, new.boundary);
                                let mut attributed = intent.clone();
                                attributed.origins =
                                    std::collections::BTreeSet::from([selection.origin.clone()]);
                                assert!(matches!(
                                    resolve(&attributed, &output, 1),
                                    Resolution::Resolved { key: found, .. } if found == key
                                ));
                                let Some(AnalyticGeometry::CircularArc {
                                    frame: old_frame, ..
                                }) = old.geometry
                                else {
                                    panic!("source rim")
                                };
                                let Some(AnalyticGeometry::CircularArc {
                                    frame: new_frame,
                                    radius_mm,
                                    ..
                                }) = new.geometry
                                else {
                                    panic!("draft rim")
                                };
                                assert_eq!(old_frame, new_frame);
                                near(radius_mm, radii[old.key.ordinal as usize]);
                            }
                            for anchor in &old.frames {
                                let new = new
                                    .frames
                                    .iter()
                                    .find(|current| current.origin == anchor.origin)
                                    .unwrap();
                                for (a, b) in new
                                    .frame
                                    .origin_mm
                                    .into_iter()
                                    .chain(new.frame.u)
                                    .chain(new.frame.v)
                                    .zip(
                                        anchor
                                            .frame
                                            .origin_mm
                                            .into_iter()
                                            .chain(anchor.frame.u)
                                            .chain(anchor.frame.v),
                                    )
                                {
                                    near(a, b);
                                }
                            }
                        }
                        assert_eq!(
                            output
                                .elements
                                .iter()
                                .filter(|element| element.key.kind == TopologyKind::Face
                                    && element.origins.contains(&selection.origin))
                                .count(),
                            1
                        );
                        result
                            .manufacturing_mesh_with_receipt(Default::default())
                            .unwrap();
                        let folder = tempfile::tempdir().unwrap();
                        let path = folder.path().join("cylinder-draft.step");
                        result.write_step(&path).unwrap();
                        let imported = Solid::import_rust_step(
                            &std::fs::read_to_string(&path).unwrap(),
                            feature,
                            policy,
                        )
                        .unwrap();
                        check(&imported, frame, radii);
                        #[cfg(feature = "occt")]
                        {
                            let native_result = native
                                .draft_faces_named(
                                    &[wall(&native, feature)],
                                    pull,
                                    neutral,
                                    angle,
                                    feature,
                                    policy,
                                )
                                .unwrap();
                            check(&native_result, frame, radii);
                            check(&Solid::read_step(&path, policy).unwrap(), frame, radii);
                        }
                        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
                    }
                }
            }
        }
    }
}

#[test]
fn cylinder_draft_refuses_nonaxial_wrong_face_collapse_and_uncertain_planes_atomically() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let frame = pose();
    let stock = source(owner, frame, None);
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let feature = EntityId::new();
    let selection = wall(&stock, feature);
    for (pull, neutral, angle, kind) in [
        (frame.u, frame, 0.1, GeometryErrorKind::Unsupported),
        (
            frame.normal(),
            PlaneFrame {
                u: frame.v,
                v: frame.normal(),
                ..frame
            },
            0.1,
            GeometryErrorKind::Unsupported,
        ),
        (
            frame.normal(),
            frame,
            45_f64.to_radians(),
            GeometryErrorKind::InvalidResult,
        ),
        (
            frame.normal(),
            PlaneFrame {
                origin: std::array::from_fn(|i| frame.origin[i] + 1e6 * frame.u[i]),
                u: std::array::from_fn(|i| frame.u[i] + 1e-14 * frame.normal()[i]),
                v: frame.v,
            },
            -85_f64.to_radians(),
            GeometryErrorKind::Unsupported,
        ),
    ] {
        assert_eq!(
            stock
                .draft_faces_named(
                    std::slice::from_ref(&selection),
                    pull,
                    neutral,
                    angle,
                    feature,
                    policy
                )
                .err()
                .unwrap()
                .kind,
            kind
        );
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    }
    for ordinals in [vec![0], vec![0, 2]] {
        let faces: Vec<_> = ordinals
            .into_iter()
            .map(|ordinal| TreatmentFace {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal,
                },
                origin: Origin {
                    source: feature,
                    role: format!("face.{ordinal}"),
                },
            })
            .collect();
        assert_eq!(
            stock
                .draft_faces_named(&faces, frame.normal(), frame, 0.1, feature, policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Unsupported
        );
    }
    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
}
