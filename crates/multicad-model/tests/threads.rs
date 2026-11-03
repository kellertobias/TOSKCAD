use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::*,
    model::{
        self, BodyInput, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ThreadMode,
        ThreadParameters, ThreadRepresentation, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, Solid};
use multicad_model::{Compiler, export::Exporter};
use std::{f64::consts::PI, path::Path};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Native threads").unwrap()).unwrap()
}
fn params() -> ThreadParameters {
    ThreadParameters {
        length: length(8.),
        ..Default::default()
    }
}
fn compile(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(s.document(), s.revision())
    .unwrap()
}
fn primitive(s: &mut Session, request: &PrimitiveRequest) {
    let p = model::prepare_primitive(s, s.revision(), actor(), request).unwrap();
    s.commit(p.prepared).unwrap();
}
fn thread(s: &mut Session, request: &ThreadRequest) {
    let p = model::prepare_thread(s, s.revision(), actor(), request).unwrap();
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(p.prepared.document(), s.revision() + 1)
    .unwrap();
    s.commit(p.prepared).unwrap();
}
fn shaft(s: &mut Session, parameters: ThreadParameters) -> ThreadRequest {
    let request = ThreadRequest::new("Threaded shaft", parameters);
    thread(s, &request);
    request
}
fn frame(x: f64, y: f64) -> PrimitiveFrame {
    PrimitiveFrame {
        origin: [length(x), length(y), length(0.)],
        ..Default::default()
    }
}

#[test]
fn native_thread_edits_keep_identity_and_roundtrip_geometry_in_an_independent_session() {
    let mut s = session();
    let request = shaft(&mut s, params());
    let model = compile(&s);
    let volume = model.solid(request.feature).unwrap().volume_mm3().unwrap();
    assert!(volume > 330.8 && volume < 331.);
    let before = s.document().clone();
    let edited = ThreadParameters {
        length: length(10.),
        ..params()
    };
    let prepared =
        model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &edited).unwrap();
    assert_eq!(s.document(), &before);
    let candidate = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(prepared.prepared.document(), s.revision() + 1)
    .unwrap();
    assert!(
        (candidate
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume * 1.25)
            .abs()
            < 1e-3
    );
    s.commit(prepared.prepared).unwrap();
    let saved = s.document().clone();
    s.undo(2, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(3, actor()).unwrap();
    assert_eq!(s.document(), &saved);
    let file = ProjectFile::new(saved.clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let file = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let reopened = Session::new(file.document).unwrap();
    let readback = compile(&reopened);
    assert_eq!(readback.snapshot().bodies[&request.body], request.feature);
    assert!(
        (readback
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume * 1.25)
            .abs()
            < 1e-3
    );
}
#[test]
fn external_cut_retains_the_unthreaded_shaft_and_internal_cut_mates_with_the_separate_male() {
    let mut external = session();
    let stock = PrimitiveRequest::new(
        "Shaft",
        PrimitiveParameters::Cylinder {
            radius: length(4.),
            height: length(20.),
            frame: Default::default(),
        },
    );
    primitive(&mut external, &stock);
    let cut = ThreadRequest::receiving(
        "External thread",
        ThreadMode::CutExternal,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        params(),
    )
    .unwrap();
    thread(&mut external, &cut);
    let built = compile(&external);
    let result = built.solid(cut.feature).unwrap();
    assert_eq!(built.snapshot().bodies[&stock.body], cut.feature);
    assert!((result.volume_mm3().unwrap() - (PI * 16. * 12. + 330.877)).abs() < 0.01);
    let bounds = result.bounds().unwrap();
    assert!((bounds.max[2] - 20.).abs() < 1e-6);
    let mut internal = session();
    let blank = PrimitiveRequest::new(
        "Receiver",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(20.)],
            frame: Default::default(),
        },
    );
    primitive(&mut internal, &blank);
    let hole = ThreadRequest::receiving(
        "Internal thread",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        ThreadParameters {
            frame: frame(10., 10.),
            radial_allowance: length(0.1),
            ..params()
        },
    )
    .unwrap();
    thread(&mut internal, &hole);
    let male = shaft(
        &mut internal,
        ThreadParameters {
            frame: frame(10., 10.),
            ..params()
        },
    );
    let model = compile(&internal);
    let receiver = model.solid(hole.feature).unwrap();
    assert_eq!(receiver.solid_count().unwrap(), 1);
    assert!(receiver.volume_mm3().unwrap() < 7700.);
    assert!(
        receiver
            .boolean(
                model.solid(male.feature).unwrap(),
                BooleanOperation::Intersect,
                GeometryPolicy::default()
            )
            .unwrap()
            .is_none()
    );
}
#[test]
fn incomplete_or_wrong_receiving_geometry_fails_without_changing_the_session() {
    for (mode, radius) in [
        (ThreadMode::CutExternal, 3.9),
        (ThreadMode::CutExternal, 4.5),
        (ThreadMode::CutInternal, 4.5),
    ] {
        let mut s = session();
        let stock = PrimitiveRequest::new(
            "Invalid receiver",
            PrimitiveParameters::Cylinder {
                radius: length(radius),
                height: length(20.),
                frame: Default::default(),
            },
        );
        primitive(&mut s, &stock);
        let before = s.document().clone();
        let request = ThreadRequest::receiving(
            "Thread",
            mode,
            BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
            params(),
        )
        .unwrap();
        let prepared = model::prepare_thread(&s, s.revision(), actor(), &request).unwrap();
        let result = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(prepared.prepared.document(), s.revision() + 1);
        assert!(result.is_err());
        assert_eq!(s.document(), &before);
    }
}
fn define_export(s: &mut Session) -> EntityId {
    let id = EntityId::new();
    let definition = SetDefinition {
        recipes: vec![
            Recipe::step("Physical STEP"),
            Recipe {
                id: EntityId::new(),
                name: "Physical STL".into(),
                output: RecipeOutput::Mesh {
                    format: MeshFormat::Stl,
                    units: LengthUnit::Millimeter,
                    hierarchy: HierarchyMode::SeparateParts,
                    names: NameHandling::FileNamesOnly,
                    aids: AidPolicy::IncludeAll,
                    quality: MeshQuality::default(),
                },
            },
        ],
        ..Default::default()
    };
    let prepared = prepare_set(s, s.revision(), actor(), id, "Manufacturing", &definition).unwrap();
    s.commit(prepared).unwrap();
    id
}
#[test]
fn cosmetic_envelopes_are_explicit_and_cannot_publish_as_modeled_step_mesh_or_print_geometry() {
    let mut s = session();
    let request = shaft(
        &mut s,
        ThreadParameters {
            representation: ThreadRepresentation::CosmeticEnvelope,
            ..params()
        },
    );
    let model = compile(&s);
    assert!(
        (model.solid(request.feature).unwrap().volume_mm3().unwrap() - PI * 16. * 8.).abs() < 1e-6
    );
    let set = define_export(&mut s);
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(preview.artifacts.is_empty());
    assert!(preview.recipes.iter().all(|r|matches!(&r.status,RecipeStatus::Unsupported(reason) if reason.contains("cosmetic")&&reason.contains("Modeled"))));
    let root = tempfile::tempdir().unwrap();
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
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    let print = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![request.body],
        },
        aids: AidPolicy::IncludeAll,
        quality: MeshQuality::default(),
        stl_per_part: true,
    };
    let failure = exporter
        .print_preview(s.document(), s.revision(), &print, &actor(), None)
        .err()
        .unwrap();
    assert!(failure.message.contains("cosmetic") && failure.message.contains("Modeled"));
    let prepared =
        model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &params()).unwrap();
    s.commit(prepared.prepared).unwrap();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let receipt = exporter
        .run(
            s.document(),
            s.revision(),
            &preview.key,
            root.path(),
            &actor(),
            None,
        )
        .unwrap();
    assert_eq!(receipt.files.len(), 2);
    for file in receipt.files {
        let path = Path::new(&receipt.directory).join(&file.artifact.output.filename);
        assert!(file.bytes > 1000);
        if path.extension().unwrap() == "step" {
            let solid = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert!((solid.volume_mm3().unwrap() - 330.877).abs() < 0.01);
        }
    }
    assert!(
        exporter
            .print_preview(s.document(), s.revision(), &print, &actor(), None)
            .unwrap()
            .can_write()
    );
}

fn unified_parameters(family: model::ThreadFamily, internal: bool) -> ThreadParameters {
    let pitch = 25.4
        / if family == model::ThreadFamily::Unc {
            20.
        } else {
            28.
        };
    ThreadParameters {
        family,
        fit: if internal {
            model::ThreadFit::Unified2B
        } else {
            model::ThreadFit::Unified2A
        },
        major_diameter: length(6.35),
        pitch: length(pitch),
        length: length(4. * pitch),
        ..Default::default()
    }
}

fn material_fraction(solid: &Solid, center: [f64; 3]) -> f64 {
    let side = 0.005;
    let probe = Solid::box_solid([side; 3], Default::default())
        .unwrap()
        .transformed(multicad_kernel::RigidTransform {
            translation_mm: center.map(|v| v - side / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&probe, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / side.powi(3)
}

#[test]
fn unified_unc_and_unf_have_native_pitch_sections_mating_clearance_and_manufacturing_exports() {
    for family in [model::ThreadFamily::Unc, model::ThreadFamily::Unf] {
        let mut s = session();
        let male_parameters = unified_parameters(family, false);
        let evaluated = multicad_core::parameters::evaluate(s.document()).unwrap();
        let resolved = male_parameters
            .resolve(&evaluated, ThreadMode::NewExternal)
            .unwrap();
        let male = shaft(&mut s, male_parameters);
        let blank = PrimitiveRequest::new(
            "Unified nut stock",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(resolved.length_mm),
                frame: Default::default(),
            },
        );
        primitive(&mut s, &blank);
        let hole = ThreadRequest::receiving(
            "Unified 2B nut",
            ThreadMode::CutInternal,
            BodyInput {
                body: blank.body,
                producer: blank.feature,
            },
            unified_parameters(family, true),
        )
        .unwrap();
        thread(&mut s, &hole);
        let native = compile(&s);
        let screw = native.solid(male.feature).unwrap();
        let nut = native.solid(hole.feature).unwrap();
        assert!(
            screw
                .boolean(nut, BooleanOperation::Intersect, Default::default())
                .unwrap()
                .is_none()
        );
        let radius = resolved.actual_pitch_diameter_mm.unwrap() / 2.;
        for turn in [1., 2.] {
            assert!(
                material_fraction(screw, [radius, 0., (turn + 0.20) * resolved.pitch_mm]) > 0.999
            );
            assert!(
                material_fraction(screw, [radius, 0., (turn + 0.30) * resolved.pitch_mm]) < 1e-5
            );
        }
        assert!(material_fraction(nut, [0., 0., 2. * resolved.pitch_mm]) < 1e-5);
        assert!(material_fraction(nut, [4.5, 0., 2. * resolved.pitch_mm]) > 0.999);
        let screw_volume = screw.volume_mm3().unwrap();
        let envelope_volume = PI * resolved.major_radius_mm.powi(2) * resolved.length_mm;
        assert!(screw_volume < envelope_volume * 0.98);
        let set = define_export(&mut s);
        let exporter = Exporter::default();
        let preview = exporter
            .preview(s.document(), s.revision(), set, &actor(), None)
            .unwrap();
        assert!(preview.can_run(), "{:?}", preview.recipes);
        let output = tempfile::tempdir().unwrap();
        let receipt = exporter
            .run(
                s.document(),
                s.revision(),
                &preview.key,
                output.path(),
                &actor(),
                None,
            )
            .unwrap();
        let expected_volume = screw_volume + nut.volume_mm3().unwrap();
        let mut step_files = 0;
        let mut step_volume = 0.;
        for file in &receipt.files {
            let path = Path::new(&receipt.directory).join(&file.artifact.output.filename);
            if path.extension().is_some_and(|ext| ext == "step") {
                let readback = Solid::read_step(&path, Default::default()).unwrap();
                step_volume += readback.volume_mm3().unwrap();
                step_files += 1;
            }
        }
        assert!(step_files > 0);
        assert!((step_volume - expected_volume).abs() < 1e-3);
        let print = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![male.body],
            },
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality::default(),
            stl_per_part: true,
        };
        let preview = exporter
            .print_preview(s.document(), s.revision(), &print, &actor(), None)
            .unwrap();
        assert!(preview.can_write());
        let print_receipt = exporter
            .write_print(
                s.document(),
                s.revision(),
                &print,
                &preview.key,
                &output.path().join("unified-screw.3mf"),
                &actor(),
                None,
            )
            .unwrap();
        assert!(!print_receipt.files.is_empty());
        for file in print_receipt.files {
            assert!(file.triangles > 100);
            assert!((file.mesh_volume_mm3 - screw_volume).abs() < screw_volume * 0.02);
            assert!(file.mesh_volume_mm3 < envelope_volume * 0.98);
        }
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(s.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut &bytes[..],
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let reopened = Session::new(reopened.document).unwrap();
        let source = model::read_thread(reopened.document(), male.feature).unwrap();
        let readback = source
            .parameters
            .resolve(
                &multicad_core::parameters::evaluate(reopened.document()).unwrap(),
                source.mode,
            )
            .unwrap();
        assert_eq!(resolved.fit, readback.fit);
        assert_eq!(
            resolved.actual_pitch_diameter_mm,
            readback.actual_pitch_diameter_mm
        );
        assert!(
            (compile(&reopened)
                .solid(male.feature)
                .unwrap()
                .volume_mm3()
                .unwrap()
                - screw_volume)
                .abs()
                < 1e-3
        );
    }
}

#[test]
fn unified_internal_wall_uses_fitted_cutter_radius_and_failure_preserves_the_document() {
    for extra_wall in [0., 0.1] {
        let mut s = session();
        let parameters = unified_parameters(model::ThreadFamily::Unc, true);
        let stock = PrimitiveRequest::new(
            "Wall-limit stock",
            PrimitiveParameters::Cylinder {
                radius: length(6.35 / 2. + 1. + extra_wall),
                height: length(5.08),
                frame: Default::default(),
            },
        );
        primitive(&mut s, &stock);
        let before = s.document().clone();
        let request = ThreadRequest::receiving(
            "Fitted 2B wall",
            ThreadMode::CutInternal,
            BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
            parameters,
        )
        .unwrap();
        let prepared = model::prepare_thread(&s, s.revision(), actor(), &request).unwrap();
        let built = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(prepared.prepared.document(), s.revision() + 1);
        assert_eq!(built.is_ok(), extra_wall > 0.);
        assert_eq!(s.document(), &before);
    }
}

#[test]
fn unified_cosmetic_fit_cannot_be_exported_as_a_manufacturing_thread() {
    let mut s = session();
    shaft(
        &mut s,
        ThreadParameters {
            representation: ThreadRepresentation::CosmeticEnvelope,
            ..unified_parameters(model::ThreadFamily::Unf, false)
        },
    );
    let set = define_export(&mut s);
    let preview = Exporter::default()
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(preview.artifacts.is_empty());
    assert!(preview.recipes.iter().all(
        |r| matches!(&r.status, RecipeStatus::Unsupported(reason) if reason.contains("cosmetic"))
    ));
}
