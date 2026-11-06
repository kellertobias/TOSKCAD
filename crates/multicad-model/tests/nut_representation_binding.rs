//! Opposite dormant literals catch manufacturing bypasses in configured nuts.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    exchange::*,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, ResolvedMechanicalPart,
        ThreadParameters, ThreadRepresentation, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache, export::Exporter};
use std::f64::consts::PI;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn fixture(
    value: u8,
    literal: ThreadRepresentation,
    diameter: f64,
    pitch: f64,
    flats: f64,
) -> (Session, EntityId, MechanicalPartRequest, EntityId) {
    let mut document = Document::new("Bound native nut").unwrap();
    let choice = Entity::new(
        "thread_representation",
        EntityData::Parameter {
            expression: value.to_string(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter = choice.id;
    document.insert(choice).unwrap();
    let mut session = Session::new(document).unwrap();
    let request = MechanicalPartRequest::new(
        "Configured nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(flats),
            thread: Box::new(ThreadParameters {
                major_diameter: length(diameter),
                pitch: length(pitch),
                length: length(3.25 * pitch),
                minimum_wall: length(0.5),
                representation: literal,
                representation_parameter: Some(parameter),
                ..Default::default()
            }),
        },
    );
    let prepared = model::prepare_mechanical_part(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let set = EntityId::new();
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        set,
        "Manufacturing",
        &SetDefinition {
            recipes: vec![
                Recipe::step("STEP"),
                Recipe {
                    id: EntityId::new(),
                    name: "STL".into(),
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
        },
    )
    .unwrap();
    session.commit(prepared).unwrap();
    (session, parameter, request, set)
}
fn choice(session: &mut Session, parameter: EntityId, value: u8) {
    let mut entity = session.document().get(parameter).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        panic!("parameter")
    };
    *expression = value.to_string();
    let prepared = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Set thread representation".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    session.commit(prepared).unwrap();
}
fn print_request(body: EntityId) -> PrintRequest {
    PrintRequest {
        source: PrintSource::Parts { parts: vec![body] },
        aids: AidPolicy::IncludeAll,
        quality: MeshQuality::default(),
        stl_per_part: true,
    }
}
fn effective(session: &Session, feature: EntityId) -> ThreadRepresentation {
    let source = model::read_mechanical_part(session.document(), feature).unwrap();
    let ResolvedMechanicalPart::HexNut { thread, .. } =
        source.parameters.resolve(session.parameters()).unwrap()
    else {
        panic!("nut")
    };
    thread.representation
}
fn manufacturing(session: &Session, request: &MechanicalPartRequest, set: EntityId, modeled: bool) {
    let before = session.document().clone();
    // This is the same source-history guard used by direct MCP STEP export.
    assert_eq!(
        model::cosmetic_thread_in_history(session.document(), request.feature).unwrap(),
        (!modeled).then_some(request.feature)
    );
    assert_eq!(
        model::has_cosmetic_threads(session.document()).unwrap(),
        !modeled
    );
    let exporter = Exporter::default();
    let preview = exporter
        .preview(session.document(), session.revision(), set, &actor(), None)
        .unwrap();
    assert_eq!(preview.can_run(), modeled);
    if !modeled {
        assert!(preview.artifacts.is_empty());
        assert!(preview.recipes.iter().all(|r| matches!(&r.status,
            RecipeStatus::Unsupported(reason) if reason.contains("cosmetic"))));
    } else {
        assert!(!preview.artifacts.is_empty());
    }
    let print = exporter.print_preview(
        session.document(),
        session.revision(),
        &print_request(request.body),
        &actor(),
        None,
    );
    if modeled {
        assert!(print.is_ok(), "modeled Print refused: {:?}", print.err());
    } else {
        assert!(print.err().unwrap().message.contains("cosmetic"));
    }
    assert_eq!(session.document(), &before);
}
fn material_fraction(solid: &Solid, center: [f64; 3], pitch: f64) -> f64 {
    let size = 0.002 * pitch;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(
            &cube,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / size.powi(3)
}
fn modeled_geometry(solid: &Solid, diameter: f64, pitch: f64, flats: f64) {
    let policy = GeometryPolicy::default();
    let source_brep = solid.to_brep().unwrap();
    // Boolean probe oracles operate on an independent BRep, preserving the source.
    let independent = Solid::from_brep(&source_brep, policy).unwrap();
    assert_eq!(independent.solid_count().unwrap(), 1);
    let height = 3.25 * pitch;
    let bounds = independent.bounds().unwrap();
    for (actual, expected) in [
        (bounds.min[0], -flats / 2.),
        (bounds.max[0], flats / 2.),
        (bounds.min[2], 0.),
        (bounds.max[2], height),
    ] {
        assert!((actual - expected).abs() < 1e-6);
    }
    let radius = diameter / 2.;
    let pitch_radius = radius - 3. * 3_f64.sqrt() * pitch / 16.;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("modeled-nut.step");
    independent.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, policy).unwrap();
    for body in [&independent, &readback] {
        assert_eq!(body.solid_count().unwrap(), 1);
        for turn in [1., 2.] {
            assert!(
                material_fraction(body, [pitch_radius, 0., (turn + 0.2) * pitch], pitch) < 1e-5,
                "thread crest must remove receiving material"
            );
            assert!(
                material_fraction(body, [pitch_radius, 0., (turn + 0.3) * pitch], pitch) > 0.999,
                "space between cutter ridges must retain material"
            );
        }
        assert!(material_fraction(body, [flats / 2. - 0.1, 0., height / 2.], pitch) > 0.999);
        let root = radius - 5. * 3_f64.sqrt() * pitch / 16.;
        let integral = (radius * radius - root * root) / 16.
            + 2. / (3_f64.sqrt() * pitch)
                * (radius * (radius * radius - root * root) / 2.
                    - (radius.powi(3) - root.powi(3)) / 3.);
        let expected =
            height * (flats * flats * 3_f64.sqrt() / 2. - PI * root * root - 2. * PI * integral);
        let area: f64 = body.faces().unwrap().iter().map(|f| f.area_mm2).sum();
        let budget = policy.linear_tolerance_mm * area + 1e-7 * expected.abs();
        assert!(
            (body.volume_mm3().unwrap() - expected).abs() <= budget,
            "native/STEP volume differs from independent profile integral"
        );
    }
    assert!(
        solid.to_brep().unwrap() == source_brep,
        "metrology mutated source BRep"
    );
}

#[test]
fn bound_cosmetic_authority_blocks_manufacturing_and_parameter_edits_change_cached_m3_geometry() {
    let (mut session, parameter, request, set) =
        fixture(0, ThreadRepresentation::Modeled, 3., 0.5, 5.5);
    let before = session.document().clone();
    assert_eq!(
        effective(&session, request.feature),
        ThreadRepresentation::CosmeticEnvelope
    );
    let mut cache = EvaluationCache::default();
    let cosmetic = compiler()
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    let volume = cosmetic
        .solid(request.feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    // The cosmetic internal envelope is the unthreaded minor-diameter bore.
    let bore_radius = 1.5 - 5. * 3_f64.sqrt() * 0.5 / 16.;
    let expected = 1.625 * (5.5_f64.powi(2) * 3_f64.sqrt() / 2. - PI * bore_radius.powi(2));
    eprintln!(
        "M3 cosmetic: native={volume:.15} expected={expected:.15} major_radius=1.5 minor_bore_radius={bore_radius:.15}"
    );
    assert!(
        (volume - expected).abs() < 1e-6,
        "minor-bore envelope volume differs"
    );
    manufacturing(&session, &request, set, false);
    assert_eq!(session.document(), &before);
    choice(&mut session, parameter, 1);
    assert_eq!(
        effective(&session, request.feature),
        ThreadRepresentation::Modeled
    );
    let modeled = compiler()
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert!(
        cache.last_compile().evaluated.contains(&request.feature),
        "binding change reused cosmetic feature"
    );
    assert!(
        modeled
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            < volume
    );
    modeled_geometry(modeled.solid(request.feature).unwrap(), 3., 0.5, 5.5);
    manufacturing(&session, &request, set, true);
    let modeled_source = session.document().clone();
    choice(&mut session, parameter, 0);
    let cosmetic_again = compiler()
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert!(
        (cosmetic_again
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume)
            .abs()
            < 1e-6
    );
    manufacturing(&session, &request, set, false);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &modeled_source);
    session.redo(session.revision(), actor()).unwrap();
    let saved = session.document().clone();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let independent = Session::new(reopened.project.document).unwrap();
    assert_eq!(independent.document(), &saved);
    assert_eq!(
        model::read_mechanical_part(independent.document(), request.feature)
            .unwrap()
            .output
            .body,
        request.body
    );
    assert_eq!(
        effective(&independent, request.feature),
        ThreadRepresentation::CosmeticEnvelope
    );
    manufacturing(&independent, &request, set, false);
}

#[test]
fn bound_modeled_authority_overrides_cosmetic_literal_for_m6_native_step_and_atomic_failures() {
    let (session, _, request, set) =
        fixture(1, ThreadRepresentation::CosmeticEnvelope, 6., 1., 10.);
    let before = session.document().clone();
    assert_eq!(
        effective(&session, request.feature),
        ThreadRepresentation::Modeled
    );
    let native = compiler()
        .compile(session.document(), session.revision())
        .unwrap();
    modeled_geometry(native.solid(request.feature).unwrap(), 6., 1., 10.);
    manufacturing(&session, &request, set, true);
    for bad in ["wall", "pitch", "length"] {
        let mut parameters = request.parameters.clone();
        let MechanicalPartParameters::HexNut {
            across_flats,
            thread,
        } = &mut parameters
        else {
            panic!("nut")
        };
        match bad {
            "wall" => *across_flats = length(6.5),
            "pitch" => thread.pitch = length(0.0005),
            _ => thread.length = length(0.5),
        }
        assert!(
            model::prepare_mechanical_part_edit(
                &session,
                session.revision(),
                actor(),
                request.feature,
                &parameters
            )
            .is_err(),
            "unsupported {bad} must refuse before source mutation"
        );
        assert_eq!(session.document(), &before);
    }
}
