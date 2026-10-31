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
use multicad_model::{Compiler, Model};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
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
        let native = compile(&session);
        let snapshot = &native.snapshot().shapes[&stock.feature].topology;
        // Select the genuinely native planar face at z=10, independent of
        // evaluation-local face ordering and current provenance role spelling.
        let face = native
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
            Some(&compile(&session).snapshot().references),
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
        let native = compile(&self.session);
        let report = &native.snapshot().references;
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
        .regions_with_queries(&NativeRegionQueries::default())
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
    fn extrude(
        &mut self,
        regions: Vec<RegionIntent>,
        operation: Operation,
        distance: f64,
    ) -> Vec<EntityId> {
        let request = ExtrudeRequest::new(
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
        );
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
