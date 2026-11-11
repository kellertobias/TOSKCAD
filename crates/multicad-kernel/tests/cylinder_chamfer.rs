use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
fn capture(key: TopologyKey, feature: EntityId) -> TreatmentEdge {
    TreatmentEdge {
        key,
        origin: Origin {
            source: feature,
            role: format!("rim/{}", key.ordinal),
        },
    }
}
fn source(owner: EntityId, reverse: Option<bool>, frame: PlaneFrame) -> Solid {
    let geometry = AxialCylinder::new(
        owner.uuid_bits(),
        Circle::new([n(0); 2], n(10)).unwrap(),
        n(0),
        n(20),
    )
    .unwrap();
    if let Some(reverse) = reverse {
        Solid::from_axial_cylinder_sweep(
            geometry,
            frame,
            [3., -2.],
            reverse,
            Origin {
                source: owner,
                role: "circle".into(),
            },
            GeometryPolicy::default(),
        )
        .unwrap()
    } else {
        Solid::from_axial_cylinder(geometry, PlaneFrame::default(), GeometryPolicy::default())
            .unwrap()
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            })
            .unwrap()
    }
}

#[test]
fn rust_cylinder_chamfers_preserve_cap_frames_and_untouched_rim_identity() {
    let policy = GeometryPolicy::default();
    for reverse in [None, Some(false), Some(true)] {
        for frame in [
            PlaneFrame::default(),
            PlaneFrame {
                origin: [17., -8., 13.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            },
        ] {
            let owner = EntityId::new();
            let stock = source(owner, reverse, frame);
            let before = stock.topology_snapshot(owner, 0).unwrap();
            let references: Vec<_> = before
                .elements
                .iter()
                .map(|element| GeometryIntent::capture(&before, element.key).unwrap())
                .collect();
            for rims in [vec![0], vec![1], vec![0, 1]] {
                for mode in 0..4 {
                    let feature = EntityId::new();
                    let edges: Vec<_> = rims
                        .iter()
                        .map(|&ordinal| {
                            capture(
                                TopologyKey {
                                    kind: TopologyKind::Edge,
                                    ordinal,
                                },
                                feature,
                            )
                        })
                        .collect();
                    let (radial, axial) = match mode {
                        0 => (2., 2.),
                        1 => (2., 3.),
                        2 => (3., 2.),
                        _ => (2., 1.),
                    };
                    let result = if mode == 0 {
                        stock.treat_edges_named(
                            &edges,
                            EdgeTreatment::Chamfer { distance_mm: 2. },
                            feature,
                            policy,
                        )
                    } else {
                        let selections: Vec<_> = edges
                            .iter()
                            .map(|edge| SidedTreatmentEdge {
                                edge: edge.clone(),
                                reference_face: TopologyKey {
                                    kind: TopologyKind::Face,
                                    ordinal: if mode == 2 { 2 } else { edge.key.ordinal },
                                },
                            })
                            .collect();
                        let treatment = if mode == 3 {
                            SidedChamfer::DistanceAngle {
                                distance_mm: 2.,
                                angle_rad: 0.5_f64.atan(),
                            }
                        } else {
                            SidedChamfer::Distances {
                                first_mm: 2.,
                                second_mm: 3.,
                            }
                        };
                        stock.chamfer_edges_sided_named(&selections, treatment, feature, policy)
                    }
                    .unwrap();
                    let expected = std::f64::consts::PI
                        * (2000.
                            - rims.len() as f64 * axial * (10. * radial - radial * radial / 3.));
                    near(result.volume_mm3().unwrap(), expected);
                    assert_eq!(result.solid_count().unwrap(), 1);
                    let output = result.topology_snapshot(feature, 1).unwrap();
                    output.validate().unwrap();
                    for (reference, old) in references.iter().zip(&before.elements) {
                        let resolution = resolve(reference, &output, 1);
                        if old.key.kind == TopologyKind::Edge && rims.contains(&old.key.ordinal) {
                            assert_eq!(resolution, Resolution::Missing);
                        } else {
                            let Resolution::Resolved { key, .. } = resolution else {
                                panic!("lost {:?}: {resolution:?}", old.key)
                            };
                            let new = output.elements.iter().find(|e| e.key == key).unwrap();
                            assert_eq!(new.origins, old.origins);
                            if old.key.kind == TopologyKind::Edge {
                                assert_eq!(new.boundary, old.boundary);
                            }
                            for anchor in &old.frames {
                                let current = new
                                    .frames
                                    .iter()
                                    .find(|f| f.origin == anchor.origin)
                                    .unwrap();
                                for (a, b) in current
                                    .frame
                                    .origin_mm
                                    .into_iter()
                                    .chain(current.frame.u)
                                    .chain(current.frame.v)
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
                            if old.key.kind == TopologyKind::Edge {
                                assert_eq!(new.geometry, old.geometry);
                            }
                        }
                    }
                    for edge in &edges {
                        let cones: Vec<_> = output
                            .elements
                            .iter()
                            .filter(|e| {
                                e.key.kind == TopologyKind::Face && e.origins.contains(&edge.origin)
                            })
                            .collect();
                        assert_eq!(cones.len(), 1);
                        assert_eq!(
                            cones[0].origins,
                            std::collections::BTreeSet::from([edge.origin.clone()])
                        );
                    }
                    result
                        .manufacturing_mesh_with_receipt(Default::default())
                        .unwrap();
                    let temp = tempfile::tempdir().unwrap();
                    let path = temp.path().join("chamfer.step");
                    result.write_step(&path).unwrap();
                    let text = std::fs::read_to_string(&path).unwrap();
                    let imported = Solid::import_rust_step(&text, feature, policy).unwrap();
                    near(imported.volume_mm3().unwrap(), expected);
                    if OCCT_LINKED {
                        let native = Shape::read_step(&path).unwrap();
                        assert!(native.valid().unwrap());
                        near(native.volume().unwrap(), expected);
                    }
                    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
                }
            }
        }
    }
}

#[test]
fn rust_cylinder_chamfers_refuse_axis_collapse_overlapping_ends_and_unrelated_side() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let stock = source(owner, None, PlaneFrame::default());
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let feature = EntityId::new();
    let edges = [0, 1].map(|ordinal| {
        capture(
            TopologyKey {
                kind: TopologyKind::Edge,
                ordinal,
            },
            feature,
        )
    });
    for (radial, axial) in [(10., 2.), (11., 2.), (2., 10.), (2., 11.)] {
        let sided: Vec<_> = edges
            .iter()
            .map(|edge| SidedTreatmentEdge {
                edge: edge.clone(),
                reference_face: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: edge.key.ordinal,
                },
            })
            .collect();
        let error = stock
            .chamfer_edges_sided_named(
                &sided,
                SidedChamfer::Distances {
                    first_mm: radial,
                    second_mm: axial,
                },
                feature,
                policy,
            )
            .err()
            .unwrap();
        assert_eq!(error.kind, GeometryErrorKind::InvalidResult);
    }
    let unrelated = SidedTreatmentEdge {
        edge: edges[0].clone(),
        reference_face: TopologyKey {
            kind: TopologyKind::Face,
            ordinal: 1,
        },
    };
    assert_eq!(
        stock
            .chamfer_edges_sided_named(
                &[unrelated],
                SidedChamfer::Distances {
                    first_mm: 2.,
                    second_mm: 3.
                },
                feature,
                policy
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    near(stock.volume_mm3().unwrap(), 2000. * std::f64::consts::PI);
}
