use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, AssemblyRequest, ComponentPlacementRequest, NumericTransform, OccurrencePath,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, angle, length,
    },
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::{collections::BTreeSet, f64::consts::TAU, io::Cursor};

type Point = [f64; 3];
const BOX_SIZE: Point = [3., 5., 7.];
const CYLINDER_BASE: Point = [2.5, -1., 0.75];
const RADIUS: f64 = 1.25;
const HEIGHT: f64 = 4.5;
const INITIAL: Point = [101., -33., 17.];
const EDITED: Point = [103.25, -37.5, 21.125];
const PIVOT: Point = [2., -3., 1.];
const ANGLE: f64 = 37.;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn near_point(actual: Point, expected: Point) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        near(actual, expected);
    }
}

fn pose(translation: Point, axis: Point, degrees: f64) -> NumericTransform {
    NumericTransform {
        translation: translation.map(length),
        axis,
        angle: angle(degrees),
        ..Default::default()
    }
}

fn root_pose(translation: Point) -> NumericTransform {
    NumericTransform {
        pivot: PIVOT.map(length),
        ..pose(translation, [1., 0., 1.], ANGLE)
    }
}

fn placed(
    session: &mut Session,
    component: EntityId,
    parameters: NumericTransform,
) -> ComponentPlacementRequest {
    let request = ComponentPlacementRequest::new(component, parameters);
    let prepared =
        model::prepare_place_component(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    request
}

fn assembled(
    session: &mut Session,
    name: &str,
    members: Vec<EntityId>,
    parts: Vec<EntityId>,
) -> AssemblyRequest {
    let request = AssemblyRequest::new(name, members, parts);
    let prepared = model::prepare_assembly(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    request
}

struct Fixture {
    session: Session,
    parts: [PrimitiveRequest; 2],
    inner: AssemblyRequest,
    members: [EntityId; 2],
    wrapper: AssemblyRequest,
    copies: [EntityId; 2],
    root: ComponentPlacementRequest,
}

fn fixture() -> Fixture {
    let mut session = Session::new(Document::new("Repeated angled assemblies").unwrap()).unwrap();
    let parts = [
        PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: BOX_SIZE.map(length),
                frame: Default::default(),
            },
        ),
        PrimitiveRequest::new(
            "Pin",
            PrimitiveParameters::Cylinder {
                radius: length(RADIUS),
                height: length(HEIGHT),
                frame: PrimitiveFrame {
                    origin: CYLINDER_BASE.map(length),
                    ..Default::default()
                },
            },
        ),
    ];
    for part in &parts {
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), part).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let members = [
        placed(
            &mut session,
            parts[0].body,
            pose([7., 2., 1.], [0., 0., 1.], 90.),
        )
        .instance,
        placed(
            &mut session,
            parts[1].body,
            pose([20., -4., 5.], [1., 0., 0.], 90.),
        )
        .instance,
    ];
    let inner = assembled(
        &mut session,
        "Stock and pin",
        members.to_vec(),
        parts.iter().map(|part| part.body).collect(),
    );
    let copies = [
        placed(
            &mut session,
            inner.assembly,
            pose([90., 17., 4.], [0., 1., 0.], 90.),
        )
        .instance,
        placed(
            &mut session,
            inner.assembly,
            pose([-70., 11., -5.], [0., 0., 1.], 180.),
        )
        .instance,
    ];
    let wrapper = assembled(&mut session, "Double fixture", copies.to_vec(), vec![]);
    let root = placed(&mut session, wrapper.assembly, root_pose(INITIAL));
    Fixture {
        session,
        parts,
        inner,
        members,
        wrapper,
        copies,
        root,
    }
}

fn world([x, y, z]: Point, member: usize, copy: usize, translation: Point) -> Point {
    // Signed permutations give the two cardinal placement levels exactly.
    let [x, y, z] = if member == 0 {
        [7. - y, 2. + x, 1. + z]
    } else {
        [20. + x, -4. - z, 5. + y]
    };
    let placed = if copy == 0 {
        [90. + z, 17. + y, 4. - x]
    } else {
        [-70. - x, 11. - y, -5. + z]
    };
    // Independent Rodrigues rotation about the tilted root's pivot.
    let [x, y, z] = std::array::from_fn(|i| placed[i] - PIVOT[i]);
    let d = std::f64::consts::FRAC_1_SQRT_2;
    let projected = (x + z) * 0.5;
    let cross = [-d * y, d * (x - z), d * y];
    let (sin, cos) = ANGLE.to_radians().sin_cos();
    let along_axis = [projected, 0., projected];
    std::array::from_fn(|i| {
        translation[i] + PIVOT[i] + cos * [x, y, z][i] + sin * cross[i] + (1. - cos) * along_axis[i]
    })
}

fn bounds(points: impl IntoIterator<Item = Point>) -> [Point; 2] {
    points.into_iter().fold(
        [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
        |[min, max], point| {
            [
                std::array::from_fn(|i| min[i].min(point[i])),
                std::array::from_fn(|i| max[i].max(point[i])),
            ]
        },
    )
}

fn expected_box(copy: usize, translation: Point) -> [Point; 2] {
    bounds((0..8).map(|bits| {
        let corner = std::array::from_fn(|i| {
            if bits & (1 << i) == 0 {
                0.
            } else {
                BOX_SIZE[i]
            }
        });
        world(corner, 0, copy, translation)
    }))
}

fn expected_cylinder(copy: usize, translation: Point) -> ([Point; 2], [Point; 2], Point) {
    let [x, y, z] = CYLINDER_BASE;
    let rims = [[x, y, z], [x, y, z + HEIGHT]].map(|point| world(point, 1, copy, translation));
    let axis: Point = std::array::from_fn(|i| (rims[1][i] - rims[0][i]) / HEIGHT);
    let radius_extent: Point =
        std::array::from_fn(|i| RADIUS * (1. - axis[i].powi(2)).max(0.).sqrt());
    let [min, max] = bounds(rims);
    (
        [
            std::array::from_fn(|i| min[i] - radius_extent[i]),
            std::array::from_fn(|i| max[i] + radius_extent[i]),
        ],
        rims,
        axis,
    )
}

fn compile(session: &Session, backend: GeometryBackend) -> Model {
    Compiler::default()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}

fn reopen(session: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let loaded = project_file::read(
        &mut Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&loaded.project.document, session.document());
    Session::new(loaded.project.document).unwrap()
}

fn member_origin(assembly: EntityId, member: EntityId) -> Origin {
    Origin {
        source: assembly,
        role: format!("assembly/member/{member}"),
    }
}

fn verify(fixture: &Fixture, session: &Session, translation: Point) -> Model {
    let native = cfg!(feature = "occt").then(|| compile(session, GeometryBackend::OpenCascade));
    let rust = compile(session, GeometryBackend::Rust);
    let root = fixture.root.instance;
    let snapshot = rust.snapshot();
    assert!(
        snapshot.bodies.is_empty(),
        "definition Parts must not be displayed twice"
    );
    assert_eq!(
        snapshot.occurrences.keys().copied().collect::<Vec<_>>(),
        [root]
    );
    assert_eq!(snapshot.leaf_occurrences.len(), 4);
    let group = rust.solid(root).unwrap();
    assert_eq!(group.solid_count().unwrap(), 4);
    let volumes = [
        BOX_SIZE.into_iter().product::<f64>(),
        std::f64::consts::PI * RADIUS.powi(2) * HEIGHT,
    ];
    near(
        group.volume_mm3().unwrap(),
        2. * volumes.iter().sum::<f64>(),
    );
    if let Some(native) = &native {
        near(
            group.volume_mm3().unwrap(),
            native.solid(root).unwrap().volume_mm3().unwrap(),
        );
    }
    let mut extrema = Vec::new();
    for (copy, &copy_instance) in fixture.copies.iter().enumerate() {
        let (cylinder_bounds, rims, axis) = expected_cylinder(copy, translation);
        for (member, expected) in [expected_box(copy, translation), cylinder_bounds]
            .into_iter()
            .enumerate()
        {
            extrema.extend(expected);
            let path = OccurrencePath {
                root,
                members: vec![copy_instance, fixture.members[member]],
            };
            let leaf = &snapshot.leaf_occurrences[&path];
            assert_eq!(leaf.component, fixture.parts[member].body);
            assert_eq!(leaf.producer, fixture.parts[member].feature);
            assert_eq!(leaf.topology.owner, root);
            assert_eq!(leaf.topology.revision, session.revision());
            let solid = rust.leaf_solid(&path).unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            near(solid.volume_mm3().unwrap(), volumes[member]);
            let native_solid = native
                .as_ref()
                .map(|model| model.leaf_solid(&path).unwrap());
            if let Some(native_solid) = native_solid {
                near(
                    solid.volume_mm3().unwrap(),
                    native_solid.volume_mm3().unwrap(),
                );
            }
            near_point(leaf.bounds.min, expected[0]);
            near_point(leaf.bounds.max, expected[1]);
            let actual = solid.bounds().unwrap();
            near_point(actual.min, expected[0]);
            near_point(actual.max, expected[1]);
            near_point(
                leaf.pose.point(CYLINDER_BASE),
                world(CYLINDER_BASE, member, copy, translation),
            );
            if member == 0 {
                if let Some(native_solid) = native_solid {
                    let reference = native_solid.bounds().unwrap();
                    near_point(reference.min, expected[0]);
                    near_point(reference.max, expected[1]);
                }
            } else {
                let circular_edges = leaf
                    .topology
                    .elements
                    .iter()
                    .filter_map(|element| {
                        if let Some(AnalyticGeometry::CircularArc {
                            frame,
                            radius_mm,
                            sweep_rad,
                            ..
                        }) = element.geometry
                        {
                            near(radius_mm, RADIUS);
                            near(sweep_rad.abs(), TAU);
                            near_point(frame.normal(), axis);
                            Some(frame.origin_mm)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                assert_eq!(circular_edges.len(), rims.len());
                for rim in rims {
                    assert!(
                        circular_edges.iter().any(|center| center
                            .iter()
                            .zip(rim)
                            .all(|(a, b)| (a - b).abs() < 1e-6)),
                        "missing circular rim at {rim:?}: {circular_edges:?}"
                    );
                }
            }
        }
    }
    let [min, max] = bounds(extrema);
    near_point(group.bounds().unwrap().min, min);
    near_point(group.bounds().unwrap().max, max);
    let shape = &snapshot.shapes[&fixture.wrapper.assembly];
    assert_eq!(shape.solid_count, 4);
    assert_eq!(shape.mesh.indices.len() / 3, shape.mesh.face_indices.len());
    let face_ordinals: BTreeSet<_> = shape
        .topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .map(|element| element.key.ordinal)
        .collect();
    assert_eq!(face_ordinals.len(), 18);
    assert_eq!(
        shape
            .mesh
            .face_indices
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        face_ordinals
    );
    rust
}

#[test]
fn nested_component_reuses_placed_leaf_geometry_and_adds_only_its_parent_origin() {
    let fixture = fixture();
    let model = compile(&fixture.session, GeometryBackend::Rust);
    for copy in fixture.copies {
        let added = Origin {
            source: fixture.wrapper.assembly,
            role: format!("assembly/member/{copy}"),
        };
        for member in fixture.members {
            let placed = model
                .leaf_snapshot(&OccurrencePath {
                    root: copy,
                    members: vec![member],
                })
                .unwrap();
            let nested_path = OccurrencePath {
                root: fixture.wrapper.assembly,
                members: vec![copy, member],
            };
            let nested = model.leaf_snapshot(&nested_path).unwrap();
            assert_eq!(nested.pose, placed.pose);
            assert_eq!(nested.bounds, placed.bounds);
            assert_eq!(nested.topology.owner, fixture.wrapper.assembly);
            assert_eq!(
                nested.topology.elements.len(),
                placed.topology.elements.len()
            );
            for (old, new) in placed
                .topology
                .elements
                .iter()
                .zip(&nested.topology.elements)
            {
                assert_eq!(new.key, old.key);
                assert_eq!(new.geometry, old.geometry);
                assert!(!old.origins.contains(&added));
                let mut expected = old.origins.clone();
                expected.insert(added.clone());
                assert_eq!(new.origins, expected);
            }
            assert_eq!(
                model
                    .leaf_solid(&nested_path)
                    .unwrap()
                    .topology_snapshot(fixture.wrapper.assembly, fixture.session.revision())
                    .unwrap(),
                nested.topology,
            );
        }
    }
}

#[test]
fn rust_nested_occurrences_rebuild_and_reopen_with_exact_world_geometry() {
    let mut fixture = fixture();
    let model = verify(&fixture, &fixture.session, INITIAL);
    let root = fixture.root.instance;
    let topology = &model.snapshot().occurrences[&root].topology;
    let cap = topology
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element.origins.contains(&Origin {
                    source: fixture.parts[0].feature,
                    role: "cap.end".into(),
                })
                && element
                    .origins
                    .contains(&member_origin(fixture.inner.assembly, fixture.members[0]))
                && element
                    .origins
                    .contains(&member_origin(fixture.wrapper.assembly, fixture.copies[0]))
        })
        .expect("repeated part cap with both nested member identities");
    let intent = GeometryIntent::capture(topology, cap.key).unwrap();

    let edited = model::prepare_instance_placement_edit(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        fixture.root.driver,
        &root_pose(EDITED),
    )
    .unwrap();
    fixture.session.commit(edited.prepared).unwrap();
    let mut reordered =
        AssemblyRequest::existing(fixture.session.document(), fixture.wrapper.assembly).unwrap();
    reordered.members.reverse();
    let edited = model::prepare_assembly_edit(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        &reordered,
    )
    .unwrap();
    fixture.session.commit(edited.prepared).unwrap();

    let reopened = reopen(&fixture.session);
    let actual = verify(&fixture, &reopened, EDITED);
    let topology = &actual.snapshot().occurrences[&root].topology;
    let Resolution::Resolved { geometry, .. } =
        references::resolve(&intent, topology, reopened.revision())
    else {
        panic!("member reorder or pose edit lost the original face reference");
    };
    let Some(AnalyticGeometry::Plane { frame }) = geometry else {
        panic!("resolved member cap must remain planar");
    };
    let on_cap = world(BOX_SIZE, 0, 0, EDITED);
    let offset: Point = std::array::from_fn(|i| on_cap[i] - frame.origin_mm[i]);
    near(
        offset
            .into_iter()
            .zip(frame.normal())
            .map(|(a, b)| a * b)
            .sum(),
        0.,
    );
}

#[test]
fn saved_toolcase_drawer_preserves_exact_ellipses_in_a_rotated_occurrence() {
    let document = project_file::read(
        &mut include_bytes!("../../../examples/toolcase.tcd").as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let drawer = EntityId::parse("bca11863-a6dd-48f1-9f34-63cc66eb7474").unwrap();
    let producer = EntityId::parse("d12c787f-4e61-4797-854e-64278f2b81f9").unwrap();
    let mut session = Session::new(document).unwrap();
    let translation = [37., -81., 19.];
    let copy = placed(&mut session, drawer, pose(translation, [1., 0., 1.], ANGLE)).instance;
    let session = reopen(&session);
    let model = Compiler::default()
        .compile_sources_with_backend(
            session.document(),
            session.revision(),
            &[copy],
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let occurrence = &model.snapshot().occurrences[&copy];
    assert_eq!(occurrence.producer, producer);
    assert_eq!(occurrence.topology.owner, copy);
    assert_eq!(occurrence.topology.revision, session.revision());
    near_point(occurrence.pose.translation_mm, translation);
    assert_ne!(occurrence.pose.vector([0., 1., 0.]), [0., 1., 0.]);
    let source = &model.snapshot().shapes[&producer].topology;
    assert_eq!(source.elements.len(), occurrence.topology.elements.len());

    let mut ellipses = 0;
    let mut anchors = 0;
    for (local, world) in source.elements.iter().zip(&occurrence.topology.elements) {
        assert_eq!(local.key, world.key);
        assert_eq!(local.origins, world.origins);
        assert_eq!(local.boundary, world.boundary);
        assert_eq!(local.frames.len(), world.frames.len());
        for (a, b) in local.frames.iter().zip(&world.frames) {
            assert_eq!(a.origin, b.origin);
            let placed = occurrence.pose.frame(a.frame);
            near_point(placed.origin_mm, b.frame.origin_mm);
            near_point(placed.u, b.frame.u);
            near_point(placed.v, b.frame.v);
            anchors += 1;
        }
        if let Some(AnalyticGeometry::EllipticArc {
            frame,
            semiaxes_mm,
            start_rad,
            sweep_rad,
        }) = local.geometry
        {
            let Some(AnalyticGeometry::EllipticArc {
                frame: placed,
                semiaxes_mm: actual_axes,
                start_rad: actual_start,
                sweep_rad: actual_sweep,
            }) = world.geometry
            else {
                panic!("ellipse descriptor was lost during placement");
            };
            let expected = occurrence.pose.frame(frame);
            near_point(placed.origin_mm, expected.origin_mm);
            near_point(placed.u, expected.u);
            near_point(placed.v, expected.v);
            assert_eq!(
                (actual_axes, actual_start, actual_sweep),
                (semiaxes_mm, start_rad, sweep_rad)
            );
            ellipses += 1;
        }
    }
    assert!(ellipses > 0);
    assert!(anchors > 0);
    let [local, world] = [producer, copy].map(|id| model.solid(id).unwrap());
    near(world.volume_mm3().unwrap(), local.volume_mm3().unwrap());
    assert_eq!(occurrence.bounds, world.bounds().unwrap());
}
