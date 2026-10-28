use ::dxf::{Drawing, entities::EntityType, enums::Units};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation, Placement},
    exchange::*,
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::export::{Exporter, dxf};
use std::io::Cursor;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn developed_provider_contract_accepts_actual_native_contours_and_rejects_foreign_receipts() {
    struct NativeFixture(u8);
    impl multicad_model::export::DevelopedContourProvider for NativeFixture {
        fn contours(
            &self,
            document: &Document,
            model: &multicad_model::Model,
            feature: EntityId,
            _interrupt: Option<&dyn Fn() -> bool>,
        ) -> Result<StampedPlanarContours, ExportError> {
            let solid = model.solid(feature).unwrap();
            let face = solid
                .faces()
                .unwrap()
                .into_iter()
                .find(|f| {
                    f.plane
                        .is_some_and(|(o, n)| (o[2] - 18.).abs() < 1e-7 && n[2] > 0.9)
                })
                .unwrap();
            let contours = solid
                .planar_face_contours(
                    face.index,
                    multicad_kernel::PlaneFrame {
                        origin: [0., 0., 18.],
                        ..Default::default()
                    },
                )
                .unwrap();
            let mut stamp = ExportStamp::capture(document, model.snapshot().stamp.revision)?;
            if self.0 == 1 {
                stamp.source_hash = multicad_core::project_file::ContentHash::of(b"stale");
            }
            Ok(StampedPlanarContours {
                stamp,
                source: if self.0 == 2 {
                    EntityId::new()
                } else {
                    feature
                },
                contours,
            })
        }
    }
    let (s, p) = panel();
    let exporter = Exporter::default();
    let result = exporter
        .developed_contours(
            s.document(),
            s.revision(),
            p.feature,
            &NativeFixture(0),
            None,
        )
        .unwrap();
    assert!((result.contours.area_mm2() - 9600.).abs() < 1e-6);
    assert_eq!(
        exporter
            .developed_contours(
                s.document(),
                s.revision(),
                p.feature,
                &NativeFixture(1),
                None
            )
            .unwrap_err()
            .kind,
        ErrorKind::Stale
    );
    assert_eq!(
        exporter
            .developed_contours(
                s.document(),
                s.revision(),
                p.feature,
                &NativeFixture(2),
                None
            )
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
    assert_eq!(
        exporter
            .developed_contours(
                s.document(),
                s.revision(),
                p.feature,
                &NativeFixture(0),
                Some(&|| true)
            )
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
}
fn primitive(s: &mut Session, name: &str, params: PrimitiveParameters) -> PrimitiveRequest {
    let r = PrimitiveRequest::new(name, params);
    let p = model::prepare_primitive(s, s.revision(), actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    r
}
fn panel() -> (Session, PrimitiveRequest) {
    let mut s = Session::new(Document::new("Native panel DXF").unwrap()).unwrap();
    let p = primitive(
        &mut s,
        "Cabinet side",
        PrimitiveParameters::Box {
            size: [length(120.), length(80.), length(18.)],
            frame: Default::default(),
        },
    );
    let mut tools = vec![];
    for (x, y, r) in [(30., 20., 5.), (80., 50., 3.)] {
        let t = primitive(
            &mut s,
            "Bore",
            PrimitiveParameters::Cylinder {
                radius: length(r),
                height: length(20.),
                frame: PrimitiveFrame {
                    origin: [length(x), length(y), length(-1.)],
                    ..Default::default()
                },
            },
        );
        tools.push(BodyInput {
            body: t.body,
            producer: t.feature,
        });
    }
    let request = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: p.body,
            producer: p.feature,
        },
        tools,
    );
    let q = model::prepare_boolean(&s, s.revision(), actor(), &request).unwrap();
    s.commit(q.prepared).unwrap();
    (s, p)
}
fn choose(s: &mut Session, scope: Scope, units: LengthUnit) -> (EntityId, SetDefinition) {
    let exporter = Exporter::default();
    let catalog = exporter
        .planar_catalog(s.document(), s.revision(), &scope, None)
        .unwrap();
    assert_eq!(catalog.parts.len(), 1);
    let part = &catalog.parts[0];
    let choice = part
        .faces
        .iter()
        .find(|f| f.plane.normal()[2] > 0.999999)
        .unwrap();
    let recipe = Recipe {
        id: EntityId::new(),
        name: "Panel contour".into(),
        output: RecipeOutput::PlanarDxf {
            units,
            layers: Default::default(),
            aids: AidPolicy::IncludeAll,
        },
    };
    let definition = SetDefinition {
        scope,
        recipes: vec![recipe.clone()],
        planar_faces: vec![PlanarFace {
            recipe: recipe.id,
            subject: part.subject.clone(),
            source: choice.source,
            reference: choice.intent.id,
            horizontal: PlanarAxis::WorldX,
        }],
        ..Default::default()
    };
    let set = EntityId::new();
    let p = prepare_set_with_geometry(
        s,
        s.revision(),
        actor(),
        set,
        "Panel cutting",
        &definition,
        &[Link {
            target: choice.source,
            role: PLANAR_FACE_ROLE.into(),
            geometry: Some(choice.intent.clone()),
        }],
    )
    .unwrap();
    s.commit(p).unwrap();
    (set, definition)
}
fn scope(id: EntityId) -> Scope {
    Scope {
        root: ScopeRoot::Explicit { entities: vec![id] },
        tags: None,
    }
}
fn measured_area(poly: &::dxf::entities::LwPolyline) -> f64 {
    poly.vertices
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = &poly.vertices[(i + 1) % poly.vertices.len()];
            let cross = (a.x * b.y - a.y * b.x) * 0.5;
            if a.bulge.abs() < 1e-12 {
                cross
            } else {
                let theta = 4. * a.bulge.atan();
                let chord = (a.x - b.x).hypot(a.y - b.y);
                let radius = chord / (2. * (theta.abs() * 0.5).sin());
                cross + radius * radius * (theta - theta.sin()) * 0.5
            }
        })
        .sum()
}
#[test]
fn actual_panel_holes_import_as_exact_closed_dxf_loops_centers_and_labels() {
    let (mut s, p) = panel();
    let (set, _) = choose(&mut s, scope(p.body), LengthUnit::Millimeter);
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let evidence = preview.artifacts[0].planar.as_ref().unwrap();
    assert_eq!((evidence.outer_loops, evidence.inner_loops), (1, 2));
    assert!((evidence.area_mm2 - (9600. - std::f64::consts::PI * 34.)).abs() < 1e-6);
    assert_eq!(evidence.bounds_mm, [[0., 0.], [120., 80.]]);
    let directory = tempfile::tempdir().unwrap();
    let receipt = exporter
        .run(
            s.document(),
            s.revision(),
            &preview.key,
            directory.path(),
            &actor(),
            None,
        )
        .unwrap();
    let path =
        std::path::Path::new(&receipt.directory).join(&receipt.files[0].artifact.output.filename);
    let imported = Drawing::load_file(&path).unwrap();
    assert_eq!(imported.header.default_drawing_units, Units::Millimeters);
    let mut outer = 0;
    let mut inner = 0;
    let mut centers = vec![];
    let mut labels = 0;
    for e in imported.entities() {
        match &e.specific {
            EntityType::LwPolyline(poly) => {
                assert!(poly.is_closed());
                assert!(poly.vertices.len() >= 2);
                for (i, a) in poly.vertices.iter().enumerate() {
                    let b = &poly.vertices[(i + 1) % poly.vertices.len()];
                    assert!((a.x - b.x).hypot(a.y - b.y) > 1e-7);
                    assert!(a.bulge.is_finite());
                }
                let area = measured_area(poly);
                match e.common.layer.as_str() {
                    "CUT" => {
                        outer += 1;
                        assert!((area - 9600.).abs() < 1e-6);
                    }
                    "CUT_INNER" => {
                        inner += 1;
                        assert!(area < 0.);
                        assert_eq!(poly.vertices.len(), 2);
                        assert!(poly.vertices.iter().all(|v| (v.bulge + 1.).abs() < 1e-12));
                    }
                    other => panic!("Unexpected cut layer {other}"),
                }
            }
            EntityType::ModelPoint(point) => {
                assert_eq!(e.common.layer, "CONSTRUCTION");
                centers.push([point.location.x, point.location.y]);
            }
            EntityType::Text(text) => {
                assert_eq!(e.common.layer, "ANNOTATION");
                assert_eq!(text.value, "Cabinet side");
                labels += 1;
            }
            other => panic!("Unexpected DXF entity {other:?}"),
        }
    }
    assert_eq!((outer, inner, labels), (1, 2, 1));
    centers.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(centers, vec![[30., 20.], [80., 50.]]);
    let reused = exporter
        .run(
            s.document(),
            s.revision(),
            &preview.key,
            directory.path(),
            &actor(),
            None,
        )
        .unwrap();
    assert!(reused.reused);
}
#[test]
fn explicit_units_world_occurrence_and_source_changes_survive_save_reopen() {
    let (mut s, p) = panel();
    let instance = Entity::new(
        "Placed panel",
        EntityData::Instance {
            component: p.body,
            placement: Placement {
                translation_mm: [200., 100., 10.],
                rotation_xyzw: [
                    0.,
                    0.,
                    std::f64::consts::FRAC_PI_4.sin(),
                    std::f64::consts::FRAC_PI_4.cos(),
                ],
            },
            placement_source: None,
        },
    );
    let id = instance.id;
    s.apply(multicad_core::commands::Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Place panel".into(),
        edits: vec![Mutation::Insert(instance)],
    })
    .unwrap();
    let (set, _) = choose(&mut s, scope(id), LengthUnit::Inch);
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let evidence = preview.artifacts[0].planar.as_ref().unwrap();
    for (a, b) in evidence
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([120., 100., 200., 220.])
    {
        assert!((a - b).abs() < 1e-6);
    }
    let root = tempfile::tempdir().unwrap();
    let output = exporter
        .run(
            s.document(),
            s.revision(),
            &preview.key,
            root.path(),
            &actor(),
            None,
        )
        .unwrap();
    let imported = Drawing::load_file(
        std::path::Path::new(&output.directory).join(&output.files[0].artifact.output.filename),
    )
    .unwrap();
    assert_eq!(imported.header.default_drawing_units, Units::Inches);
    let outer = imported
        .entities()
        .find_map(|e| match &e.specific {
            EntityType::LwPolyline(p) if e.common.layer == "CUT" => Some(p),
            _ => None,
        })
        .unwrap();
    assert!((measured_area(outer) - 9600. / 25.4_f64.powi(2)).abs() < 1e-7);
    let mut bytes = vec![];
    let file = ProjectFile::new(s.document().clone()).unwrap();
    assert!(file.capabilities().contains(PLANAR_CAPABILITY));
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let again = exporter
        .preview(&reopened.project.document, 0, set, &actor(), None)
        .unwrap();
    assert!(again.can_run());
    assert_eq!(again.artifacts[0].planar, preview.artifacts[0].planar);
    assert!(
        exporter
            .run(
                s.document(),
                s.revision() + 1,
                &preview.key,
                root.path(),
                &actor(),
                None
            )
            .is_err()
    );
    let edit = model::prepare_primitive_edit(
        &s,
        s.revision(),
        actor(),
        p.feature,
        &PrimitiveParameters::Box {
            size: [length(140.), length(80.), length(18.)],
            frame: Default::default(),
        },
    )
    .unwrap();
    s.commit(edit.prepared).unwrap();
    assert!(
        exporter
            .run(
                s.document(),
                s.revision(),
                &preview.key,
                root.path(),
                &actor(),
                None
            )
            .is_err()
    );
    let updated = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(updated.can_run(), "{:?}", updated.recipes);
    let current = updated.artifacts[0].planar.as_ref().unwrap();
    for (a, b) in current
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([120., 100., 200., 240.])
    {
        assert!((a - b).abs() < 1e-6);
    }
    assert!((current.area_mm2 - (11200. - std::f64::consts::PI * 34.)).abs() < 1e-6);
    assert_ne!(current.geometry_hash, evidence.geometry_hash);
}
#[test]
fn absent_axis_mismatched_or_removed_face_never_falls_back_to_an_arbitrary_projection() {
    let (mut s, p) = panel();
    let (set, mut definition) = choose(&mut s, scope(p.body), LengthUnit::Millimeter);
    let links = geometry_links(s.document(), set);
    definition.planar_faces[0].horizontal = PlanarAxis::WorldZ;
    let q = prepare_set_with_geometry(
        &s,
        s.revision(),
        actor(),
        set,
        "Panel cutting",
        &definition,
        &links,
    )
    .unwrap();
    s.commit(q).unwrap();
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(
        matches!(&preview.recipes[0].status,RecipeStatus::Unsupported(reason)if reason.contains("normal"))
    );
    let mut bad = links.clone();
    let mut intent = bad[0].geometry.clone().unwrap();
    intent.origins = std::collections::BTreeSet::from([multicad_core::references::Origin {
        source: p.feature,
        role: "explicitly-missing-face".into(),
    }]);
    bad[0].geometry = Some(intent);
    definition.planar_faces[0].horizontal = PlanarAxis::WorldX;
    let q = prepare_set_with_geometry(
        &s,
        s.revision(),
        actor(),
        set,
        "Panel cutting",
        &definition,
        &bad,
    )
    .unwrap();
    s.commit(q).unwrap();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(
        matches!(&preview.recipes[0].status,RecipeStatus::Unsupported(reason)if reason.contains("repair"))
    );
    let model = multicad_model::Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let diagnostic = model
        .snapshot()
        .references
        .entries
        .iter()
        .find(|d| d.address.owner == set)
        .unwrap();
    assert!(is_non_driving_planar_reference(s.document(), diagnostic));
    let mut foreign = diagnostic.clone();
    foreign.role = "attachment".into();
    assert!(!is_non_driving_planar_reference(s.document(), &foreign));
    definition.planar_faces.clear();
    let q = prepare_set(&s, s.revision(), actor(), set, "Panel cutting", &definition).unwrap();
    s.commit(q).unwrap();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(
        matches!(&preview.recipes[0].status,RecipeStatus::Unsupported(reason)if reason.contains("explicit"))
    );
}
#[test]
fn exact_arc_writer_roundtrips_units_and_rejects_invalid_layers_or_tampering() {
    let solid =
        multicad_kernel::Solid::cylinder(Default::default(), 5., 3., Default::default()).unwrap();
    let top = solid
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane
                .is_some_and(|(o, n)| (o[2] - 3.).abs() < 1e-8 && n[2] > 0.9)
        })
        .unwrap();
    let contours = solid
        .planar_face_contours(
            top.index,
            multicad_kernel::PlaneFrame {
                origin: [0., 0., 3.],
                ..Default::default()
            },
        )
        .unwrap();
    for (units, expected) in [
        (LengthUnit::Millimeter, Units::Millimeters),
        (LengthUnit::Centimeter, Units::Centimeters),
        (LengthUnit::Meter, Units::Meters),
        (LengthUnit::Inch, Units::Inches),
    ] {
        let bytes = dxf::encode(
            &contours,
            units,
            &Default::default(),
            "Circular profile",
            1e-7,
        )
        .unwrap();
        dxf::verify(&bytes, &bytes).unwrap();
        let imported = Drawing::load(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(imported.header.default_drawing_units, expected);
        let poly = imported
            .entities()
            .find_map(|e| match &e.specific {
                EntityType::LwPolyline(p) => Some(p),
                _ => None,
            })
            .unwrap();
        assert_eq!(poly.vertices.len(), 2);
        assert!(poly.vertices.iter().all(|v| (v.bulge - 1.).abs() < 1e-12));
        let mut bad = bytes.clone();
        bad[0] = b'9';
        assert!(dxf::verify(&bad, &bytes).is_err());
    }
    for construction in ["CUT", "cut", "0", "   "] {
        let layers = Layers {
            construction: construction.into(),
            ..Default::default()
        };
        assert!(dxf::encode(&contours, LengthUnit::Millimeter, &layers, "Bad", 1e-7).is_err());
    }
}
