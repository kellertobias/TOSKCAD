mod common;
use common::*;
use multicad_core::{
    document::{EntityData, Mutation},
    model::{
        self, BodyInput, Direction, Extent, ExtrudeParameters, Operation, Termination, angle,
        length,
    },
    project_file::ContentHash,
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, Model, ShapeSnapshot};

fn shape(model: &Model, body: multicad_core::document::EntityId) -> &ShapeSnapshot {
    let producer = model.snapshot().bodies.get(&body).expect("Part producer");
    model
        .snapshot()
        .shapes
        .get(producer)
        .expect("Exact shape snapshot")
}
#[test]
fn one_shared_sketch_compiles_two_real_solids_without_mutating_canonical_source() {
    let fixture = SharedProfiles::new(Default::default());
    let before = fixture.session.document().clone();
    let hash = ContentHash::document(&before).unwrap();
    let model = Compiler::default()
        .compile(&before, fixture.session.revision())
        .unwrap();
    let snapshot = model.snapshot();
    assert_eq!(snapshot.stamp.document, before.id());
    assert_eq!(snapshot.stamp.revision, fixture.session.revision());
    assert_eq!(snapshot.stamp.source_hash, hash);
    assert_eq!(snapshot.bodies.len(), 2);
    assert_eq!(snapshot.shapes.len(), 2);
    let outer = shape(&model, fixture.outer().body);
    let inner = shape(&model, fixture.inner().body);
    near(outer.volume_mm3, (WIDTH * HEIGHT - INNER_AREA) * 10.);
    near(inner.volume_mm3, INNER_AREA * 10.);
    near(outer.volume_mm3 + inner.volume_mm3, WIDTH * HEIGHT * 10.);
    point_near(outer.bounds.min, [0., 0., 0.]);
    point_near(outer.bounds.max, [WIDTH, HEIGHT, 10.]);
    point_near(inner.bounds.min, [14., 14., 0.]);
    point_near(inner.bounds.max, [26., 26., 10.]);
    for output in &fixture.request.outputs {
        let snapshot = shape(&model, output.body);
        assert_eq!(snapshot.producer, output.feature);
        assert_eq!(snapshot.solid_count, 1);
        assert!(!snapshot.mesh.vertices.is_empty());
        assert!(!snapshot.mesh.indices.is_empty());
        assert!(!snapshot.topology.elements.is_empty());
        let exact = model.solid(output.feature).expect("Owned native B-rep");
        assert_eq!(exact.solid_count().unwrap(), 1);
        near(exact.volume_mm3().unwrap(), snapshot.volume_mm3);
    }
    assert_eq!(fixture.session.document(), &before);
    assert_eq!(
        ContentHash::document(fixture.session.document()).unwrap(),
        hash
    );
}
#[test]
fn sketch_edit_rebuilds_profiles_with_the_same_feature_and_body_ids() {
    let mut fixture = SharedProfiles::new(Default::default());
    let compiler = Compiler::default();
    let first = compiler
        .compile(fixture.session.document(), fixture.session.revision())
        .unwrap();
    let identities = first.snapshot().bodies.clone();
    let authored_ids: Vec<_> = fixture
        .source()
        .sketch
        .curves
        .iter()
        .map(|c| c.id)
        .collect();
    fixture.set_width(100.);
    let changed = fixture.session.document().clone();
    let second = compiler
        .compile(&changed, fixture.session.revision())
        .unwrap();
    assert_eq!(second.snapshot().bodies, identities);
    assert_eq!(
        fixture
            .source()
            .sketch
            .curves
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        authored_ids
    );
    near(
        shape(&second, fixture.outer().body).volume_mm3,
        (100. * HEIGHT - INNER_AREA) * 10.,
    );
    near(
        shape(&second, fixture.inner().body).volume_mm3,
        INNER_AREA * 10.,
    );
    point_near(
        shape(&second, fixture.outer().body).bounds.max,
        [100., HEIGHT, 10.],
    );
    assert_ne!(first.snapshot().stamp, second.snapshot().stamp);
    assert_eq!(fixture.session.document(), &changed);
}
#[test]
fn changed_holes_and_invalid_boundary_intent_fail_without_adopting_a_different_region() {
    let mut fixture = SharedProfiles::new(Default::default());
    let compiler = Compiler::default();
    let previous = compiler
        .compile(fixture.session.document(), fixture.session.revision())
        .unwrap();
    let outer = fixture.outer().feature;
    fixture.add_unexpected_hole();
    let changed = fixture.session.document().clone();
    let error = compiler
        .compile(&changed, fixture.session.revision())
        .err()
        .expect("Stale selected holes must fail");
    assert_eq!(error.entity, Some(outer));
    assert!(error.message.contains("Selected profile changed"));
    assert_eq!(fixture.session.document(), &changed);
    // A caller may retain this old display result, but the failed compile has
    // not relabeled it as current or silently generated a newly holed part.
    assert_ne!(
        previous.snapshot().stamp.revision,
        fixture.session.revision()
    );

    let mut fixture = SharedProfiles::new(Default::default());
    let feature = fixture.outer().feature;
    let mut source = model::read_extrude(fixture.session.document(), feature).unwrap();
    source.profile.region.outer.curves = source
        .profile
        .region
        .outer
        .curves
        .iter()
        .copied()
        .take(1)
        .collect();
    source.profile.region.holes.clear();
    let mut entity = fixture.session.document().get(feature).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut entity.data else {
        panic!("Feature")
    };
    *definition = source.definition().unwrap();
    fixture.apply(vec![Mutation::Replace(entity)]);
    let error = compiler
        .compile(fixture.session.document(), fixture.session.revision())
        .err()
        .expect("One open line is not the selected area");
    assert_eq!(error.entity, Some(feature));
    assert!(error.message.contains("Selected profile changed"));
}
#[test]
fn offsets_tilt_and_symmetric_total_have_real_sweep_bounds_and_volumes() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let cases = [
        (
            90.,
            false,
            Extent::OneSided {
                end: Termination::Distance {
                    distance: length(10.),
                },
            },
            [0., 0., 7.],
            [WIDTH, HEIGHT, 17.],
            10.,
        ),
        (
            45.,
            false,
            Extent::OneSided {
                end: Termination::Distance {
                    distance: length(10.),
                },
            },
            [0., 0., 7.],
            [WIDTH + 10. * s, HEIGHT, 7. + 10. * s],
            10. * s,
        ),
        (
            135.,
            false,
            Extent::OneSided {
                end: Termination::Distance {
                    distance: length(10.),
                },
            },
            [-10. * s, 0., 7.],
            [WIDTH, HEIGHT, 7. + 10. * s],
            10. * s,
        ),
        (
            45.,
            true,
            Extent::OneSided {
                end: Termination::Distance {
                    distance: length(10.),
                },
            },
            [-10. * s, 0., 7. - 10. * s],
            [WIDTH, HEIGHT, 7.],
            10. * s,
        ),
        (
            45.,
            false,
            Extent::Symmetric { total: length(10.) },
            [-5. * s, 0., 7. - 5. * s],
            [WIDTH + 5. * s, HEIGHT, 7. + 5. * s],
            10. * s,
        ),
        (
            90.,
            false,
            Extent::TwoSided {
                forward: Termination::Distance {
                    distance: length(10.),
                },
                backward: Termination::Distance {
                    distance: length(4.),
                },
            },
            [0., 0., 3.],
            [WIDTH, HEIGHT, 17.],
            14.,
        ),
    ];
    for (elevation, reverse, extent, min, max, normal_height) in cases {
        let parameters = ExtrudeParameters {
            direction: Direction {
                elevation: angle(elevation),
                azimuth: angle(0.),
                reverse,
            },
            start_offset: length(7.),
            extent,
            ..Default::default()
        };
        let fixture = SharedProfiles::new(parameters);
        let before = fixture.session.document().clone();
        let model = Compiler::default()
            .compile(&before, fixture.session.revision())
            .unwrap();
        let outer = shape(&model, fixture.outer().body);
        point_near(outer.bounds.min, min);
        point_near(outer.bounds.max, max);
        near(
            outer.volume_mm3,
            (WIDTH * HEIGHT - INNER_AREA) * normal_height,
        );
        near(
            shape(&model, fixture.inner().body).volume_mm3,
            INNER_AREA * normal_height,
        );
        assert_eq!(fixture.session.document(), &before);
    }
}
#[test]
fn unsupported_native_options_are_errors_instead_of_default_geometry() {
    for case in 0..3 {
        let mut fixture = SharedProfiles::new(Default::default());
        let feature = fixture.outer().feature;
        let mut source = model::read_extrude(fixture.session.document(), feature).unwrap();
        match case {
            0 => {
                source.operation = Operation::Cut;
                source.targets = vec![BodyInput {
                    body: fixture.inner().body,
                    producer: fixture.inner().feature,
                }];
            }
            1 => source.parameters.taper = angle(2.),
            _ => {
                let plane = sketch_plane(&fixture);
                source.parameters.extent = Extent::OneSided {
                    end: Termination::ToPlane {
                        plane,
                        offset: length(0.),
                    },
                };
            }
        }
        let mut entity = fixture.session.document().get(feature).unwrap().clone();
        let EntityData::Feature {
            definition, inputs, ..
        } = &mut entity.data
        else {
            panic!()
        };
        *definition = source.definition().unwrap();
        *inputs = source.inputs(&[]).unwrap();
        fixture.apply(vec![Mutation::Replace(entity)]);
        let before = fixture.session.document().clone();
        let error = Compiler::default()
            .compile(&before, fixture.session.revision())
            .err()
            .expect("Unsupported mode must not produce default extrusion");
        assert_eq!(error.entity, Some(feature));
        assert_eq!(fixture.session.document(), &before);
    }
}
fn sketch_plane(fixture: &SharedProfiles) -> multicad_core::document::EntityId {
    let EntityData::Sketch { plane, .. } =
        &fixture.session.document().get(fixture.sketch).unwrap().data
    else {
        panic!()
    };
    *plane
}
#[test]
fn exact_compiled_body_writes_and_reads_step_without_using_the_display_mesh() {
    let fixture = SharedProfiles::new(Default::default());
    let model = Compiler::default()
        .compile(fixture.session.document(), fixture.session.revision())
        .unwrap();
    let exact = model.solid(fixture.inner().feature).unwrap();
    let path = std::env::temp_dir().join(format!(
        "multicad-model-{}.step",
        multicad_core::document::EntityId::new()
    ));
    exact.write_step(&path).unwrap();
    let imported = Solid::read_step(&path, Default::default()).unwrap();
    std::fs::remove_file(&path).unwrap();
    near(imported.volume_mm3().unwrap(), INNER_AREA * 10.);
    assert_eq!(imported.solid_count().unwrap(), 1);
    let bounds = imported.bounds().unwrap();
    point_near(bounds.min, [14., 14., 0.]);
    point_near(bounds.max, [26., 26., 10.]);
}
