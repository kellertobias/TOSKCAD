//! CAD-193: exact, associative regions formed by strokes on a finite face.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityId},
    model::{
        self, BodyInput, Extent, ExtrudeBooleanRequest, ExtrudeParameters, ExtrudeRequest,
        Operation, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, Termination, length,
    },
    project_file::{self, FileLimits, ProjectFile},
    references::TopologyKind,
    sketch::{self, CreationPlane, PlaneOptions},
    sketch_regions::{Extraction, RegionIntent},
};
use multicad_kernel::NativeRegionQueries;
use multicad_model::{Compiler, GeometryBackend, Model};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    compile_on(s, GeometryBackend::default())
}
fn compile_on(s: &Session, backend: GeometryBackend) -> Model {
    Compiler::default()
        .compile_with_backend(s.document(), s.revision(), backend, None)
        .unwrap()
}
fn save_review(session: &Session, name: &str) {
    if let Some(directory) = std::env::var_os("MULTICAD_FACE_CELLS_REVIEW") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut std::fs::File::create(directory.join(name)).unwrap(),
            FileLimits::default(),
        )
        .unwrap();
    }
}
struct Fixture {
    session: Session,
    stock: PrimitiveRequest,
    sketch: EntityId,
}
impl Fixture {
    fn new() -> Self {
        Self::with_hole(None)
    }
    fn with_hole(center: Option<[f64; 2]>) -> Self {
        Self::on(GeometryBackend::default(), center)
    }
    fn on(backend: GeometryBackend, center: Option<[f64; 2]>) -> Self {
        let mut session = Session::new(Document::new("Diagonal face regions").unwrap()).unwrap();
        let mut stock = PrimitiveRequest::new(
            "20 × 20 × 10 box",
            PrimitiveParameters::Box {
                size: [length(20.), length(20.), length(10.)],
                frame: PrimitiveFrame::default(),
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
        session.commit(prepared.prepared).unwrap();
        if let Some(center) = center {
            let tool = PrimitiveRequest::new(
                "Through hole",
                PrimitiveParameters::Cylinder {
                    radius: length(2.),
                    height: length(10.),
                    frame: PrimitiveFrame {
                        origin: [length(center[0]), length(center[1]), length(0.)],
                        ..Default::default()
                    },
                },
            );
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor(), &tool).unwrap();
            session.commit(prepared.prepared).unwrap();
            let request = model::BooleanRequest::new(
                Operation::Cut,
                BodyInput {
                    body: stock.body,
                    producer: stock.feature,
                },
                vec![BodyInput {
                    body: tool.body,
                    producer: tool.feature,
                }],
            );
            let prepared =
                model::prepare_boolean(&session, session.revision(), actor(), &request).unwrap();
            session.commit(prepared.prepared).unwrap();
            stock.feature = request.feature;
        }
        if center.is_none() {
            save_review(&session, "box-only.tcd");
        }
        let compiled = compile_on(&session, backend);
        let snapshot = &compiled.snapshot().shapes[&stock.feature].topology;
        // Find the top face independently of evaluation-local face order and provenance roles.
        let face = compiled
            .solid(stock.feature)
            .unwrap()
            .faces()
            .unwrap()
            .into_iter()
            .find(|f| {
                f.plane
                    .is_some_and(|(p, n)| (p[2] - 10.).abs() < 1e-7 && n[2] > 0.9)
            })
            .unwrap();
        let element = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == face.index)
            .unwrap();
        let anchor = element.frames.first().unwrap().origin.clone();
        let created = sketch::prepare_new(
            &session,
            session.revision(),
            actor(),
            "One diagonal",
            CreationPlane::Face {
                snapshot,
                key: element.key,
                anchor: Some(&anchor),
            },
            PlaneOptions::default(),
        )
        .unwrap();
        let sketch = created.sketch;
        let frame = created.frame;
        session.commit(created.prepared).unwrap();
        let mut source = Source::read_for_evaluation(
            session.document(),
            sketch,
            session.revision(),
            Some(&compile_on(&session, backend).snapshot().references),
        )
        .unwrap()
        .0;
        let a = source.add_point(frame.coordinates([0., 0., 10.]));
        let b = source.add_point(frame.coordinates([20., 20., 10.]));
        source.add_line(a, b);
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Draw one ordinary diagonal".into(),
                edits: source.replace(session.document(), sketch).unwrap(),
            })
            .unwrap();
        Self {
            session,
            stock,
            sketch,
        }
    }
    fn regions(&self) -> Extraction {
        self.regions_on(GeometryBackend::default())
    }
    fn regions_on(&self, backend: GeometryBackend) -> Extraction {
        let compiled = compile_on(&self.session, backend);
        let report = &compiled.snapshot().references;
        let queries = if backend == GeometryBackend::Rust {
            NativeRegionQueries::analytic(Default::default()).unwrap()
        } else {
            NativeRegionQueries::default()
        };
        assert!(
            report.sketch_supports[&self.sketch].is_ok(),
            "{:?}",
            report.sketch_supports
        );
        multicad_solver::evaluate_document_sketch(
            self.session.document(),
            self.session.revision(),
            self.sketch,
            Some(report),
        )
        .unwrap()
        .regions_with_queries(&queries)
        .unwrap()
    }
    fn edit_strokes(&mut self, edit: impl FnOnce(&mut Source, sketch::Frame)) {
        let native = compile(&self.session);
        let (mut source, frame, _) = Source::read_for_evaluation(
            self.session.document(),
            self.sketch,
            self.session.revision(),
            Some(&native.snapshot().references),
        )
        .unwrap();
        edit(&mut source, frame);
        self.session
            .apply(Request {
                expected_revision: self.session.revision(),
                actor: actor(),
                label: "Edit face sketch strokes".into(),
                edits: source
                    .replace(self.session.document(), self.sketch)
                    .unwrap(),
            })
            .unwrap();
    }
    fn extrusion_request(&self, regions: Vec<RegionIntent>, distance: f64) -> ExtrudeRequest {
        ExtrudeRequest::new(
            self.sketch,
            regions,
            ExtrudeParameters {
                extent: Extent::OneSided {
                    end: Termination::Distance {
                        distance: length(distance),
                    },
                },
                ..Default::default()
            },
        )
    }
    fn prepare_extrusion(
        &self,
        regions: Vec<RegionIntent>,
        operation: Operation,
        distance: f64,
    ) -> (model::ModelPrepared, Vec<EntityId>) {
        let request = self.extrusion_request(regions, distance);
        let ids = request.outputs.iter().map(|o| o.feature).collect();
        let prepared = if operation == Operation::NewBody {
            model::prepare_extrude(&self.session, self.session.revision(), actor(), &request)
                .unwrap()
        } else {
            let boolean = ExtrudeBooleanRequest::new(
                request,
                operation,
                vec![BodyInput {
                    body: self.stock.body,
                    producer: self.stock.feature,
                }],
                false,
            );
            model::prepare_extrude_boolean(
                &self.session,
                self.session.revision(),
                actor(),
                &boolean,
            )
            .unwrap()
        };
        (prepared, ids)
    }
    fn extrude(
        &mut self,
        regions: Vec<RegionIntent>,
        operation: Operation,
        distance: f64,
    ) -> Vec<EntityId> {
        let (prepared, ids) = self.prepare_extrusion(regions, operation, distance);
        // Preview compiles a candidate without committing source or history.
        let before = self.session.document().clone();
        Compiler::default()
            .compile(prepared.prepared.document(), self.session.revision() + 1)
            .unwrap();
        assert_eq!(&before, self.session.document());
        self.session.commit(prepared.prepared).unwrap();
        ids
    }
}

#[test]
fn one_diagonal_has_two_selectable_triangles_and_independent_exact_prisms() {
    for count in [1, 2] {
        let mut f = Fixture::new();
        let regions = f.regions();
        save_review(&f.session, "diagonal.tcd");
        assert_eq!(regions.regions.len(), 2);
        for region in &regions.regions {
            near(region.area_mm2, 200.);
        }
        let ids = f.extrude(
            regions
                .regions
                .iter()
                .take(count)
                .map(|r| r.intent())
                .collect(),
            Operation::NewBody,
            5.,
        );
        let native = compile(&f.session);
        save_review(&f.session, &format!("prisms-{count}.tcd"));
        near(native.snapshot().shapes[&f.stock.feature].volume_mm3, 4000.);
        for id in ids {
            near(native.snapshot().shapes[&id].volume_mm3, 1000.);
            assert_eq!(native.solid(id).unwrap().solid_count().unwrap(), 1);
        }
    }
}

#[test]
fn either_or_both_triangles_join_and_cut_without_double_counting_shared_edges() {
    for (operation, distance, delta) in
        [(Operation::Join, 5., 1000.), (Operation::Cut, -5., -1000.)]
    {
        for selection in [vec![0], vec![1], vec![0, 1]] {
            let mut f = Fixture::new();
            let regions = f.regions();
            f.extrude(
                selection
                    .iter()
                    .map(|i| regions.regions[*i].intent())
                    .collect(),
                operation,
                distance,
            );
            let native = compile(&f.session);
            let head = native.snapshot().bodies[&f.stock.body];
            near(
                native.snapshot().shapes[&head].volume_mm3,
                4000. + delta * selection.len() as f64,
            );
        }
    }
}

#[test]
fn rust_cuts_attached_triangles_through_both_model_commands() {
    for (composite, keep_tools, count) in [(true, false, 2), (false, true, 1), (false, false, 1)] {
        let mut f = Fixture::on(GeometryBackend::Rust, None);
        let regions = f.regions_on(GeometryBackend::Rust);
        let intents: Vec<_> = regions
            .regions
            .iter()
            .take(count)
            .map(|r| r.intent())
            .collect();
        assert_eq!(intents.len(), count);
        let extrusion = f.extrusion_request(intents.clone(), -5.);
        let target = BodyInput {
            body: f.stock.body,
            producer: f.stock.feature,
        };
        let tools =
            model::prepare_extrude(&f.session, f.session.revision(), actor(), &extrusion).unwrap();
        let document = tools.prepared.document();
        let revision = f.session.revision() + 1;
        let query = model::CutTargetQuery {
            tools: extrusion
                .outputs
                .iter()
                .map(|output| output.feature)
                .collect(),
            component: None,
            include: Vec::new(),
            exclude: Vec::new(),
        };
        let discover = |backend| {
            Compiler::default()
                .compile_with_backend(document, revision, backend, None)
                .unwrap()
                .cut_targets(document, revision, &query, None)
                .unwrap()
        };
        let selection = discover(GeometryBackend::Rust);
        assert_eq!(selection.targets, vec![target]);
        #[cfg(feature = "occt")]
        assert_eq!(selection, discover(GeometryBackend::OpenCascade));

        let request =
            ExtrudeBooleanRequest::new(extrusion, Operation::Cut, selection.targets, keep_tools);
        let cut = &request.booleans[0];
        let prepared = if composite {
            model::prepare_extrude_boolean(&f.session, f.session.revision(), actor(), &request)
        } else {
            f.session.commit(tools.prepared).unwrap();
            model::prepare_boolean(&f.session, f.session.revision(), actor(), cut)
        }
        .unwrap();
        let preview = Compiler::default()
            .compile_with_backend(
                prepared.prepared.document(),
                f.session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        near(
            preview.snapshot().shapes[&cut.feature].volume_mm3,
            4000. - 1000. * count as f64,
        );
        f.session.commit(prepared.prepared).unwrap();

        let mut bytes = Vec::new();
        project_file::write(
            &ProjectFile::new(f.session.document().clone()).unwrap(),
            &mut bytes,
            FileLimits::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut std::io::Cursor::new(bytes),
            FileLimits::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        f.session = Session::new(reopened.project.document).unwrap();

        for height in [10., 15.] {
            if height == 15. {
                let edit = model::prepare_primitive_edit(
                    &f.session,
                    f.session.revision(),
                    actor(),
                    f.stock.feature,
                    &PrimitiveParameters::Box {
                        size: [length(20.), length(20.), length(height)],
                        frame: PrimitiveFrame::default(),
                    },
                )
                .unwrap();
                f.session.commit(edit.prepared).unwrap();
            }
            let saved_regions = f.regions_on(GeometryBackend::Rust);
            for intent in &intents {
                assert!(saved_regions.resolve(intent).is_some());
            }
            let result = compile_on(&f.session, GeometryBackend::Rust);
            let snapshot = result.snapshot();
            let shape = &snapshot.shapes[&cut.feature];
            assert_eq!(snapshot.bodies[&f.stock.body], cut.feature);
            assert_eq!(shape.solid_count, 1);
            for tool in &cut.tools {
                assert_eq!(snapshot.bodies.contains_key(&tool.body), keep_tools);
                assert_eq!(snapshot.consumed_bodies.contains(&tool.body), !keep_tools);
                assert!(shape.topology.elements.iter().any(|face| {
                    face.key.kind == TopologyKind::Face
                        && face
                            .origins
                            .iter()
                            .any(|origin| origin.source == tool.producer)
                }));
            }
            assert!(shape.topology.elements.iter().any(|face| {
                face.key.kind == TopologyKind::Face
                    && face
                        .origins
                        .iter()
                        .any(|origin| origin.source == f.stock.feature)
            }));
            near(shape.volume_mm3, 400. * height - 1000. * count as f64);
            near(shape.bounds.min[2], 0.);
            near(
                shape.bounds.max[2],
                height - if count == 2 { 5. } else { 0. },
            );
            let area = |m: &Model| {
                m.solid(cut.feature)
                    .unwrap()
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|face| face.area_mm2)
                    .sum::<f64>()
            };
            let expected_area = if count == 2 {
                800. + 80. * (height - 5.)
            } else {
                600. + 80. * height + 100. * 2.0_f64.sqrt()
            };
            near(area(&result), expected_area);
            #[cfg(feature = "occt")]
            {
                let native = compile_on(&f.session, GeometryBackend::OpenCascade);
                let native_shape = &native.snapshot().shapes[&cut.feature];
                near(shape.volume_mm3, native_shape.volume_mm3);
                near(area(&result), area(&native));
                for axis in 0..3 {
                    near(shape.bounds.min[axis], native_shape.bounds.min[axis]);
                    near(shape.bounds.max[axis], native_shape.bounds.max[axis]);
                }
            }
        }
    }
}

#[test]
fn rust_cut_of_both_full_depth_triangles_rejects_empty_output_before_commit() {
    let f = Fixture::on(GeometryBackend::Rust, None);
    let regions = f.regions_on(GeometryBackend::Rust);
    let (prepared, _) = f.prepare_extrusion(
        regions
            .regions
            .iter()
            .map(|region| region.intent())
            .collect(),
        Operation::Cut,
        -10.,
    );
    let before = f.session.document().clone();
    let result = Compiler::default().compile_with_backend(
        prepared.prepared.document(),
        f.session.revision() + 1,
        GeometryBackend::Rust,
        None,
    );
    let failure = result.err().expect("an empty cut must refuse the preview");
    assert!(
        failure.to_string().contains("removed all material"),
        "{failure}"
    );
    assert_eq!(f.session.document(), &before);
}

#[test]
fn regions_and_settings_survive_undo_redo_and_project_roundtrip() {
    let mut f = Fixture::new();
    let region = f.regions().regions[0].intent();
    let before = f.session.document().clone();
    let ids = f.extrude(vec![region.clone()], Operation::NewBody, 5.);
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(f.session.revision(), actor()).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let project = project_file::read(
        &mut std::io::Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let mut old_reader = project_file::supported_capabilities();
    old_reader.remove(multicad_core::sketch_regions::FACE_SKETCH_REGIONS_CAPABILITY);
    let mut encoded = vec![];
    project_file::write(
        &ProjectFile::new(f.session.document().clone()).unwrap(),
        &mut encoded,
        FileLimits::default(),
    )
    .unwrap();
    assert!(
        project_file::read(
            &mut std::io::Cursor::new(encoded),
            FileLimits::default(),
            &old_reader
        )
        .is_err()
    );
    f.session = Session::new(project.project.document).unwrap();
    assert!(f.regions().resolve(&region).is_some());
    near(
        compile(&f.session).snapshot().shapes[&ids[0]].volume_mm3,
        1000.,
    );
}

#[test]
fn circular_face_holes_are_excluded_even_when_the_diagonal_crosses_them() {
    for center in [[5., 15.], [10., 10.]] {
        let mut f = Fixture::with_hole(Some(center));
        let regions = f.regions();
        assert_eq!(regions.regions.len(), 2);
        near(
            regions.regions.iter().map(|r| r.area_mm2).sum(),
            400. - 4. * std::f64::consts::PI,
        );
        let report = compile(&f.session).snapshot().references.clone();
        let semantic = multicad_solver::evaluate_document_sketch(
            f.session.document(),
            f.session.revision(),
            f.sketch,
            Some(&report),
        )
        .unwrap();
        let local = semantic.frame.coordinates([center[0], center[1], 10.]);
        assert!(regions.regions.iter().all(|r| {
            !r.contains_with_queries(local, &NativeRegionQueries::default())
                .unwrap()
        }));
        let ids = f.extrude(
            regions.regions.iter().map(|r| r.intent()).collect(),
            Operation::NewBody,
            5.,
        );
        let native = compile(&f.session);
        near(
            ids.iter()
                .map(|id| native.snapshot().shapes[id].volume_mm3)
                .sum(),
            (400. - 4. * std::f64::consts::PI) * 5.,
        );
    }
}

#[test]
fn supporting_height_rebuild_preserves_selection_and_moves_the_prism_associatively() {
    let mut f = Fixture::new();
    let region = f.regions().regions[0].intent();
    let ids = f.extrude(vec![region.clone()], Operation::NewBody, 5.);
    let prepared = model::prepare_primitive_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.stock.feature,
        &PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(15.)],
            frame: PrimitiveFrame::default(),
        },
    )
    .unwrap();
    f.session.commit(prepared.prepared).unwrap();
    assert!(f.regions().resolve(&region).is_some());
    let native = compile(&f.session);
    near(native.snapshot().shapes[&ids[0]].volume_mm3, 1000.);
    near(native.snapshot().shapes[&ids[0]].bounds.min[2], 15.);
    near(native.snapshot().shapes[&ids[0]].bounds.max[2], 20.);
}

#[test]
fn construction_and_dangling_strokes_do_not_partition_but_crossing_lines_do() {
    let mut f = Fixture::new();
    let original = f.regions();
    f.edit_strokes(|source, frame| {
        for (a, b, construction) in [
            ([0., 20., 10.], [20., 0., 10.], true),
            ([0., 5., 10.], [3., 5., 10.], false),
        ] {
            let a = source.add_point(frame.coordinates(a));
            let b = source.add_point(frame.coordinates(b));
            source.add_line(a, b);
            source.sketch.curves.last_mut().unwrap().construction = construction;
        }
    });
    let unchanged = f.regions();
    assert_eq!(unchanged.regions.len(), 2);
    for r in &original.regions {
        assert!(unchanged.resolve(&r.intent()).is_some());
    }
    f.edit_strokes(|source, frame| {
        let a = source.add_point(frame.coordinates([0., 10., 10.]));
        let b = source.add_point(frame.coordinates([20., 10., 10.]));
        source.add_line(a, b);
    });
    let split = f.regions();
    assert_eq!(split.regions.len(), 4);
    near(split.regions.iter().map(|r| r.area_mm2).sum(), 400.);
    let ids = f.extrude(
        split.regions.iter().map(|r| r.intent()).collect(),
        Operation::NewBody,
        5.,
    );
    let native = compile(&f.session);
    near(
        ids.iter()
            .map(|id| native.snapshot().shapes[id].volume_mm3)
            .sum(),
        2000.,
    );
}

#[test]
fn changed_or_missing_diagonal_recomputes_and_never_retargets_a_missing_region() {
    let mut f = Fixture::new();
    let before = f.regions();
    f.edit_strokes(|source, frame| {
        source.sketch.points[1].position_mm = frame.coordinates([20., 10., 10.]);
    });
    let changed = f.regions();
    let mut areas: Vec<_> = changed.regions.iter().map(|r| r.area_mm2).collect();
    areas.sort_by(f64::total_cmp);
    near(areas[0], 100.);
    near(areas[1], 300.);
    assert_eq!(
        before
            .regions
            .iter()
            .filter(|r| changed.resolve(&r.intent()).is_some())
            .count(),
        1
    );
    let selected = changed.regions[0].intent();
    f.edit_strokes(|source, _| {
        source.sketch.curves.clear();
        source.sketch.points.clear();
    });
    assert!(f.regions().resolve(&selected).is_none());
    let source = f.session.document().clone();
    let request = ExtrudeRequest::new(f.sketch, vec![selected], ExtrudeParameters::default());
    let prepared =
        model::prepare_extrude(&f.session, f.session.revision(), actor(), &request).unwrap();
    assert!(
        Compiler::default()
            .compile(prepared.prepared.document(), f.session.revision() + 1)
            .is_err()
    );
    let report = Compiler::default()
        .sketch_reference_report(
            prepared.prepared.document(),
            f.session.revision() + 1,
            f.sketch,
            None,
        )
        .unwrap();
    let repair = multicad_solver::evaluate_document_sketch(
        prepared.prepared.document(),
        f.session.revision() + 1,
        f.sketch,
        Some(&report),
    )
    .unwrap();
    let remaining = repair
        .regions_with_queries(&NativeRegionQueries::default())
        .unwrap();
    assert_eq!(remaining.regions.len(), 1);
    near(remaining.regions[0].area_mm2, 400.);
    drop(prepared);
    assert_eq!(f.session.document(), &source);
}

#[test]
fn missing_or_foreign_native_support_receipts_never_become_selectable_profiles() {
    let f = Fixture::new();
    let native = compile(&f.session);
    let mut report = native.snapshot().references.clone();
    report.sketch_supports.clear();
    let evaluated = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        Some(&report),
    )
    .unwrap();
    assert!(evaluated.geometry().is_ok());
    assert!(
        evaluated
            .regions_with_queries(&NativeRegionQueries::default())
            .is_err()
    );
    let mut report = native.snapshot().references.clone();
    report
        .sketch_supports
        .get_mut(&f.sketch)
        .unwrap()
        .as_mut()
        .unwrap()
        .source_hash = multicad_core::project_file::ContentHash::of(b"foreign source");
    let evaluated = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        Some(&report),
    )
    .unwrap();
    assert!(
        evaluated
            .regions_with_queries(&NativeRegionQueries::default())
            .is_err()
    );
}

#[test]
fn an_existing_authored_face_perimeter_keeps_its_closed_profile_identity() {
    let mut f = Fixture::new();
    let mut intent = None;
    f.edit_strokes(|source, frame| {
        source.sketch.curves.clear();
        source.sketch.points.clear();
        source
            .add_rectangle(
                frame.coordinates([0., 0., 10.]),
                frame.coordinates([20., 20., 10.]),
            )
            .unwrap();
        intent = Some(multicad_core::sketch_regions::extract(source).regions[0].intent());
    });
    let intent = intent.unwrap();
    let regions = f.regions();
    assert_eq!(regions.regions.len(), 1);
    near(regions.resolve(&intent).unwrap().area_mm2, 400.);
    let ids = f.extrude(vec![intent], Operation::NewBody, 5.);
    let native = compile(&f.session);
    near(native.snapshot().shapes[&ids[0]].volume_mm3, 2000.);
    near(native.snapshot().shapes[&f.stock.feature].volume_mm3, 4000.);
}

#[test]
fn nested_rectangle_and_surrounding_face_are_independent_exact_extrusion_inputs() {
    let mut fixture = Fixture::new();
    fixture.edit_strokes(|source, frame| {
        source.sketch.curves.clear();
        source.sketch.points.clear();
        source
            .add_rectangle(
                frame.coordinates([5., 5., 10.]),
                frame.coordinates([15., 15., 10.]),
            )
            .unwrap();
    });
    save_review(&fixture.session, "nested-rectangle.tcd");
    let extraction = fixture.regions();
    assert_eq!(extraction.regions.len(), 2);
    let report = compile(&fixture.session).snapshot().references.clone();
    let sketch = multicad_solver::evaluate_document_sketch(
        fixture.session.document(),
        fixture.session.revision(),
        fixture.sketch,
        Some(&report),
    )
    .unwrap();
    let queries = NativeRegionQueries::default();
    let pick = |world| {
        let local = sketch.frame.coordinates(world);
        let mut hits = extraction
            .regions
            .iter()
            .filter(|region| region.contains_with_queries(local, &queries).unwrap());
        let region = hits.next().expect("one exact region contains this point");
        assert!(hits.next().is_none(), "region interiors must not overlap");
        region
    };
    let inner = pick([10., 10., 10.]);
    let surrounding = pick([2., 3., 10.]);
    near(inner.area_mm2, 100.);
    near(surrounding.area_mm2, 300.);
    assert!(inner.holes.is_empty());
    assert_eq!(surrounding.holes.len(), 1);
    assert_eq!(surrounding.holes[0].intent, inner.outer.intent);
    let intents = [inner.intent(), surrounding.intent()];

    // A validated preview of both areas may still be discarded without
    // changing the sketch, host solid, or transaction history.
    let before = fixture.session.document().clone();
    let revision = fixture.session.revision();
    let (preview, outputs) = fixture.prepare_extrusion(intents.to_vec(), Operation::NewBody, 5.);
    let candidate = Compiler::default()
        .compile(preview.prepared.document(), revision + 1)
        .unwrap();
    for (output, expected) in outputs.into_iter().zip([500., 1500.]) {
        near(candidate.snapshot().shapes[&output].volume_mm3, expected);
    }
    drop(preview);
    assert_eq!(fixture.session.document(), &before);
    assert_eq!(fixture.session.revision(), revision);

    for (operation, distance) in [
        (Operation::NewBody, 5.),
        (Operation::Join, 5.),
        (Operation::Cut, -5.),
    ] {
        for selection in [&[0][..], &[1], &[0, 1]] {
            let mut sample = Fixture {
                session: Session::new(before.clone()).unwrap(),
                stock: fixture.stock.clone(),
                sketch: fixture.sketch,
            };
            let selected: Vec<_> = selection.iter().map(|&i| intents[i].clone()).collect();
            let output = sample.extrude(selected.clone(), operation, distance);
            let compiled = compile(&sample.session);
            if operation == Operation::NewBody {
                near(
                    compiled.snapshot().shapes[&sample.stock.feature].volume_mm3,
                    4000.,
                );
                for ((output, index), intent) in output.iter().zip(selection).zip(&selected) {
                    near(
                        compiled.snapshot().shapes[output].volume_mm3,
                        [500., 1500.][*index],
                    );
                    assert_eq!(compiled.solid(*output).unwrap().solid_count().unwrap(), 1);
                    let source = model::read_extrude(sample.session.document(), *output).unwrap();
                    assert_eq!(&source.profile.sketch().unwrap().region, intent);
                }
                if selection.len() == 2 {
                    let mut bytes = vec![];
                    project_file::write(
                        &ProjectFile::new(sample.session.document().clone()).unwrap(),
                        &mut bytes,
                        FileLimits::default(),
                    )
                    .unwrap();
                    let saved = project_file::read(
                        &mut bytes.as_slice(),
                        FileLimits::default(),
                        &project_file::supported_capabilities(),
                    )
                    .unwrap();
                    sample.session = Session::new(saved.project.document).unwrap();
                    let reopened = sample.regions();
                    let compiled = compile(&sample.session);
                    for ((output, intent), expected) in
                        output.iter().zip(&intents).zip([500., 1500.])
                    {
                        assert!(reopened.resolve(intent).is_some());
                        let source =
                            model::read_extrude(sample.session.document(), *output).unwrap();
                        assert_eq!(&source.profile.sketch().unwrap().region, intent);
                        near(compiled.snapshot().shapes[output].volume_mm3, expected);
                    }
                }
            } else {
                let current = compiled.snapshot().bodies[&sample.stock.body];
                let area: f64 = selection.iter().map(|&i| [100., 300.][i]).sum();
                near(
                    compiled.snapshot().shapes[&current].volume_mm3,
                    4000. + area * distance,
                );
                assert_eq!(compiled.solid(current).unwrap().solid_count().unwrap(), 1);
            }
        }
    }
}
