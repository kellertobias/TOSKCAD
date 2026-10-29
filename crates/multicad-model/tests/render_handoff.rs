//! Render scenes and the Blender handoff: a scene stored in the document
//! with camera, lights, background and display style; a GLB built from the
//! same native model with the camera, KHR_lights_punctual lights, shared
//! instance meshes, appearances and scene extras; a manifest recording scope,
//! units and revision; and refusals. An ignored test imports the handoff
//! with Blender in background mode as an independent consumer.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    materials::{self, Appearance, Assignment, LibraryDefinition},
    model::{
        self, AidDefinition, AidRequest, AidShape, BodyOutput, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, Projection, RenderCamera, RenderDisplay, RenderLight, RenderScene, angle,
        length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, export::Exporter};
use serde_json::Value;
use std::{f64::consts::PI, path::Path};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}

struct Fixture {
    s: Session,
    panel: EntityId,
    pin: EntityId,
    aid: EntityId,
}
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Render").unwrap()).unwrap();
    let panel = PrimitiveRequest::new(
        "Panel",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let p = model::prepare_primitive(&s, 0, actor(), &panel).unwrap();
    s.commit(p.prepared).unwrap();
    let pin = PrimitiveRequest::new(
        "Pin",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(10.),
            frame: PrimitiveFrame {
                origin: [length(100.), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(&s, s.revision(), actor(), &pin).unwrap();
    s.commit(p.prepared).unwrap();
    // Red steel appearance on the pin, distinct from any Material.
    let steel = LibraryDefinition::Appearance(Appearance {
        color_rgba: [200, 40, 40, 255],
        roughness: 0.3,
        metallic: 1.,
    })
    .entity(EntityId::new(), "Red steel")
    .unwrap();
    let steel_id = steel.id;
    s.commit(materials::prepare_library(&s, s.revision(), actor(), &[steel]).unwrap())
        .unwrap();
    s.commit(
        materials::prepare_assignment(
            &s,
            s.revision(),
            actor(),
            &[pin.body],
            &Assignment {
                material: None,
                stock: None,
                appearance: Some(steel_id),
                hatch: None,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let q = (PI / 4.).sin();
    let instances = [
        Entity::new(
            "Rotated copy",
            EntityData::Instance {
                component: panel.body,
                placement: Placement {
                    translation_mm: [50., 5., 2.],
                    rotation_xyzw: [0., 0., q, q],
                },
                placement_source: None,
            },
        ),
        Entity::new(
            "Moved copy",
            EntityData::Instance {
                component: panel.body,
                placement: Placement {
                    translation_mm: [-20., 40., 0.],
                    ..Default::default()
                },
                placement_source: None,
            },
        ),
    ];
    let assembly = Entity::new(
        "Bench",
        EntityData::Assembly {
            instances: instances.iter().map(|i| i.id).collect(),
        },
    );
    let mut edits: Vec<_> = instances.into_iter().map(Mutation::Insert).collect();
    edits.push(Mutation::Insert(assembly));
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Place copies".into(),
        edits,
    })
    .unwrap();
    // A breakaway tab on the pin: a removable aid the handoff leaves out.
    let tab = AidRequest::new(
        "Pin tab",
        AidDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            target: pin.body,
            shape: AidShape::Tab {
                origin: [length(105.), length(0.), length(0.)],
                angle: angle(0.),
                length: length(8.),
                width: length(6.),
                thickness: length(0.4),
                overlap: length(2.),
            },
            process: None,
        },
    );
    let p = model::prepare_aid(&s, s.revision(), actor(), &tab).unwrap();
    s.commit(p.prepared).unwrap();
    Fixture {
        s,
        panel: panel.body,
        pin: pin.body,
        aid: tab.body,
    }
}
fn scene() -> RenderScene {
    RenderScene {
        camera: RenderCamera {
            eye_mm: [300., -400., 200.],
            target_mm: [0., 0., 0.],
            projection: Projection::Perspective { fov_deg: 40. },
        },
        background_rgb: [30, 60, 90],
        lights: vec![
            RenderLight::Sun {
                name: "Key sun".into(),
                direction: [0., 0., -1.],
                strength_w_m2: 3.,
                color_rgb: [255, 255, 255],
            },
            RenderLight::Point {
                name: "Fill lamp".into(),
                position_mm: [0., 0., 500.],
                power_w: 1000.,
                color_rgb: [255, 128, 0],
            },
        ],
        display: RenderDisplay::ShadedEdges,
        parts: vec![],
        exclude_aids: true,
    }
}
fn set_scene(f: &mut Fixture, id: Option<EntityId>, value: &RenderScene) -> EntityId {
    let (id, p) =
        model::prepare_render_scene(&f.s, f.s.revision(), actor(), id, "Studio", value).unwrap();
    f.s.commit(p).unwrap();
    id
}
/// The JSON chunk of a GLB, read independently of the writer.
fn glb_json(bytes: &[u8]) -> Value {
    assert_eq!(&bytes[0..4], b"glTF");
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    assert_eq!(&bytes[16..20], b"JSON");
    serde_json::from_slice(&bytes[20..20 + length]).unwrap()
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / n)
}

#[test]
fn handoff_glb_carries_camera_lights_instances_and_appearances() {
    let mut f = fixture();
    let id = set_scene(&mut f, None, &scene());
    let package = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), id, "studio", None)
        .unwrap();
    let gltf = glb_json(&package.glb);

    // Four drawn nodes (panel, pin, two copies); the copies share a mesh.
    let nodes = gltf["nodes"].as_array().unwrap();
    let drawn: Vec<_> = nodes.iter().filter(|n| n.get("mesh").is_some()).collect();
    assert_eq!(drawn.len(), 4, "{drawn:?}");
    assert_eq!(gltf["meshes"].as_array().unwrap().len(), 3);
    assert!(!nodes.iter().any(|n| n["name"] == "Pin tab"));

    // Camera: yfov 40°, at the eye in meters, local −Z toward the target.
    let camera = &gltf["cameras"][0];
    assert_eq!(camera["type"], "perspective");
    near(
        camera["perspective"]["yfov"].as_f64().unwrap(),
        40f64.to_radians(),
        1e-12,
    );
    let cam_node = nodes.iter().find(|n| n.get("camera").is_some()).unwrap();
    let m: Vec<f64> = serde_json::from_value(cam_node["matrix"].clone()).unwrap();
    for (i, e) in [0.3, -0.4, 0.2].into_iter().enumerate() {
        near(m[12 + i], e, 1e-12);
    }
    let view = unit([-300., 400., -200.]);
    for (i, v) in view.into_iter().enumerate() {
        near(-m[8 + i], v, 1e-12);
    }
    // The camera's local +Y has a positive model-Z component: +Z stays up.
    assert!(m[6] > 0.);

    // Lights in KHR_lights_punctual units for Blender's default import.
    assert_eq!(gltf["extensionsUsed"][0], "KHR_lights_punctual");
    let lights = gltf["extensions"]["KHR_lights_punctual"]["lights"]
        .as_array()
        .unwrap();
    assert_eq!(lights[0]["type"], "directional");
    near(lights[0]["intensity"].as_f64().unwrap(), 3. * 683., 1e-9);
    assert_eq!(lights[1]["type"], "point");
    near(
        lights[1]["intensity"].as_f64().unwrap(),
        1000. * 683. / (4. * PI),
        1e-9,
    );
    let orange = lights[1]["color"].as_array().unwrap();
    near(orange[0].as_f64().unwrap(), 1., 1e-12);
    near(orange[2].as_f64().unwrap(), 0., 1e-12);

    // Scene extras carry the background in linear RGB and the scene id.
    let extras = &gltf["scenes"][0]["extras"];
    assert_eq!(extras["multicad_render_scene"], id.to_string());
    let bg = extras["multicad_background_linear"].as_array().unwrap();
    near(
        bg[2].as_f64().unwrap(),
        ((90. / 255. + 0.055) / 1.055f64).powf(2.4),
        1e-12,
    );

    // Two materials: the default appearance and the red steel.
    let materials = gltf["materials"].as_array().unwrap();
    assert_eq!(materials.len(), 2);
    assert!(materials.iter().any(|m| {
        m["pbrMetallicRoughness"]["metallicFactor"] == 1.0
            && m["pbrMetallicRoughness"]["roughnessFactor"] == 0.3
    }));

    // The manifest records scope, units, revision and the excluded aid.
    let manifest = &package.manifest;
    assert_eq!(manifest.revision, f.s.revision());
    assert_eq!(manifest.units, "meter");
    assert_eq!(manifest.glb, "studio.glb");
    assert_eq!(manifest.parts.len(), 4);
    assert_eq!(manifest.excluded_aids.len(), 1);
    assert_eq!(manifest.excluded_aids[0].part, f.aid);
    let pin = manifest.parts.iter().find(|p| p.part == f.pin).unwrap();
    assert_eq!(pin.appearance_rgba, [200, 40, 40, 255]);
    assert_eq!(
        manifest
            .parts
            .iter()
            .filter(|p| p.part == f.panel && !p.occurrence_path.is_empty())
            .count(),
        2
    );
    near(manifest.lights[1].blender_value, 1000., 0.);
    let text: Value = serde_json::from_slice(&package.manifest_json()).unwrap();
    assert_eq!(text["display"], "shaded_edges");
    assert_eq!(text["projection"]["kind"], "perspective");

    // The same source gives identical bytes.
    let again = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), id, "studio", None)
        .unwrap();
    assert_eq!(again.glb, package.glb);

    // Including aids adds the tab Part as a fifth drawn node.
    let mut with_aids = scene();
    with_aids.exclude_aids = false;
    set_scene(&mut f, Some(id), &with_aids);
    let package = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), id, "studio", None)
        .unwrap();
    let gltf = glb_json(&package.glb);
    let drawn = gltf["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n.get("mesh").is_some())
        .count();
    assert_eq!(drawn, 5);
}

#[test]
fn render_scenes_are_document_records_with_undo_save_and_refusals() {
    let mut f = fixture();
    let id = set_scene(&mut f, None, &scene());
    let stored = model::read_render_scene(f.s.document(), id).unwrap();
    assert_eq!(stored, scene());
    // An edit replaces the record; Undo restores it.
    let mut ortho = scene();
    ortho.camera.projection = Projection::Orthographic { height_mm: 250. };
    ortho.display = RenderDisplay::Xray;
    set_scene(&mut f, Some(id), &ortho);
    assert_eq!(
        model::read_render_scene(f.s.document(), id)
            .unwrap()
            .display,
        RenderDisplay::Xray
    );
    let package = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), id, "ortho", None)
        .unwrap();
    let gltf = glb_json(&package.glb);
    assert_eq!(gltf["cameras"][0]["type"], "orthographic");
    near(
        gltf["cameras"][0]["orthographic"]["ymag"].as_f64().unwrap(),
        0.125,
        1e-12,
    );
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(
        model::read_render_scene(f.s.document(), id).unwrap(),
        scene()
    );

    // Save and reopen keep the scene and its capability.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert!(
        opened
            .capabilities()
            .contains(model::RENDER_SCENE_CAPABILITY)
    );
    assert_eq!(
        model::read_render_scene(&opened.document, id).unwrap(),
        scene()
    );

    // Invalid scenes are refused before anything changes.
    let before = f.s.revision();
    let try_scene = |value: RenderScene| {
        model::prepare_render_scene(&f.s, before, actor(), None, "Bad", &value)
            .map(|_| ())
            .unwrap_err()
            .message
    };
    let mut same = scene();
    same.camera.target_mm = same.camera.eye_mm;
    assert!(try_scene(same).contains("differ"));
    let mut down = scene();
    down.camera.eye_mm = [0., 0., 500.];
    assert!(try_scene(down).contains("straight"));
    let mut wide = scene();
    wide.camera.projection = Projection::Perspective { fov_deg: 179. };
    assert!(try_scene(wide).contains("Field of view"));
    let mut twins = scene();
    twins.lights.push(twins.lights[0].clone());
    assert!(try_scene(twins).contains("distinct"));
    let mut not_part = scene();
    not_part.parts = vec![id];
    assert!(try_scene(not_part).contains("not a Part"));
    assert_eq!(f.s.revision(), before);

    // A scene naming only the aid, with aids excluded, has nothing to hand off.
    let mut aid_only = scene();
    aid_only.parts = vec![f.aid];
    let only = set_scene(&mut f, None, &aid_only);
    let error = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), only, "x", None)
        .unwrap_err();
    assert!(error.message.contains("no Part"), "{}", error.message);
    // Handoff names must be plain.
    let error = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), id, "../x", None)
        .unwrap_err();
    assert!(error.message.contains("handoff name"), "{}", error.message);
    // The project still evaluates: the scene holds no geometry.
    Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
}

/// Independent consumer: Blender's own glTF importer. Run with
/// `cargo test -p multicad-model --test render_handoff -- --ignored` on a
/// machine with Blender; set MULTICAD_BLENDER to its executable if it is not
/// `blender` on PATH.
#[test]
#[ignore = "needs a local Blender executable"]
fn blender_imports_the_handoff_with_scale_hierarchy_instances_camera_and_lights() {
    let blender = std::env::var("MULTICAD_BLENDER").unwrap_or_else(|_| "blender".into());
    let mut f = fixture();
    let id = set_scene(&mut f, None, &scene());
    let package = Exporter::default()
        .render_handoff(f.s.document(), f.s.revision(), id, "studio", None)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let glb = dir.path().join("studio.glb");
    let report = dir.path().join("report.json");
    std::fs::write(&glb, &package.glb).unwrap();
    std::fs::write(
        dir.path().join("studio.render.json"),
        package.manifest_json(),
    )
    .unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/blender/render_handoff_report.py");
    let status = std::process::Command::new(&blender)
        .args([
            "-b",
            "--factory-startup",
            "--python-exit-code",
            "1",
            "--python",
        ])
        .arg(&script)
        .arg("--")
        .arg(&glb)
        .arg(&report)
        .output()
        .expect("run Blender");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let r: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    if let Ok(keep) = std::env::var("MULTICAD_KEEP_REPORT") {
        std::fs::copy(&report, Path::new(&keep).join("render_handoff_report.json")).unwrap();
        std::fs::copy(&glb, Path::new(&keep).join("studio.glb")).unwrap();
        std::fs::write(
            Path::new(&keep).join("studio.render.json"),
            package.manifest_json(),
        )
        .unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&r).unwrap());
    let objects = r["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 4, "four mesh objects");
    let object = |name: &str| {
        objects
            .iter()
            .find(|o| o["name"].as_str().unwrap().starts_with(name))
            .unwrap_or_else(|| panic!("no {name}"))
    };
    let mm = |o: &Value, key: &str| -> Vec<f64> {
        o[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() * 1000.)
            .collect()
    };
    // Scale and placement: world bounds in millimeters after import.
    let rotated = object("Panel Rotated copy");
    let (lo, hi) = (mm(rotated, "min_m"), mm(rotated, "max_m"));
    for (v, e) in lo.iter().zip([30., 5., 2.]) {
        near(*v, e, 1e-3);
    }
    for (v, e) in hi.iter().zip([50., 15., 32.]) {
        near(*v, e, 1e-3);
    }
    let moved = object("Panel Moved copy");
    for (v, e) in mm(moved, "min_m").iter().zip([-20., 40., 0.]) {
        near(*v, e, 1e-3);
    }
    // Repeated instances share one mesh; the hierarchy keeps the assembly.
    assert_eq!(rotated["mesh"], moved["mesh"]);
    assert_eq!(rotated["mesh_users"], 2);
    assert!(
        rotated["parents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "Bench")
    );
    // Basic material: the pin's red steel appearance.
    let pin = object("Pin");
    near(pin["metallic"].as_f64().unwrap(), 1., 1e-6);
    near(pin["roughness"].as_f64().unwrap(), 0.3, 1e-6);
    near(
        pin["base_color"][0].as_f64().unwrap(),
        ((200. / 255. + 0.055) / 1.055f64).powf(2.4),
        1e-4,
    );
    // Camera: perspective, 40° vertical, at the eye, looking at the target.
    let camera = &r["cameras"][0];
    assert_eq!(camera["projection"], "PERSP");
    near(
        camera["angle_y"].as_f64().unwrap(),
        40f64.to_radians(),
        1e-5,
    );
    for (v, e) in camera["location_m"]
        .as_array()
        .unwrap()
        .iter()
        .zip([0.3, -0.4, 0.2])
    {
        near(v.as_f64().unwrap(), e, 1e-6);
    }
    let view = unit([-300., 400., -200.]);
    for (v, e) in camera["forward"].as_array().unwrap().iter().zip(view) {
        near(v.as_f64().unwrap(), e, 1e-5);
    }
    // Lights: Blender's own units come back as authored.
    let lights = r["lights"].as_array().unwrap();
    let sun = lights.iter().find(|l| l["light_type"] == "SUN").unwrap();
    near(sun["energy"].as_f64().unwrap(), 3., 1e-4);
    for (v, e) in sun["forward"].as_array().unwrap().iter().zip([0., 0., -1.]) {
        near(v.as_f64().unwrap(), e, 1e-5);
    }
    let lamp = lights.iter().find(|l| l["light_type"] == "POINT").unwrap();
    near(lamp["energy"].as_f64().unwrap(), 1000., 1e-2);
    near(lamp["location_m"][2].as_f64().unwrap(), 0.5, 1e-6);
    // Scene extras arrive as scene custom properties.
    let props = &r["scene_properties"];
    assert_eq!(props["multicad_render_scene"], id.to_string());
    assert_eq!(props["multicad_display"], "shaded_edges");
    near(
        props["multicad_background_linear"][2].as_f64().unwrap(),
        ((90. / 255. + 0.055) / 1.055f64).powf(2.4),
        1e-6,
    );
}
