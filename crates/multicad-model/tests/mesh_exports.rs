//! Real OBJ, 3MF, STL and GLB output from exact solids, read back by
//! readers that share no code with the writers: a local OBJ parser, a
//! separate ZIP reader with CRC checks, quick-xml for the 3MF model, the
//! system `unzip` when present, a binary STL parser and a glTF reader that
//! composes node matrices itself.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    exchange::*,
    materials::{self, TagEdit, TagQuery},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    project_file::ContentHash,
};
use multicad_model::{
    GeometryBackend,
    export::{ExportWorker, Exporter},
};
use quick_xml::{Reader, events::Event};
use std::{collections::BTreeMap, f64::consts::PI, path::Path};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}

struct Fixture {
    session: Session,
    panel: PrimitiveRequest,
    pin: PrimitiveRequest,
    instances: [EntityId; 2],
    assembly: EntityId,
    set: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let mut session = Session::new(Document::new("Mesh exports").unwrap()).unwrap();
        let panel = PrimitiveRequest::new(
            "Panel",
            PrimitiveParameters::Box {
                size: [length(10.), length(20.), length(30.)],
                frame: Default::default(),
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, 0, actor(), &panel)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
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
        session
            .commit(
                model::prepare_primitive(&session, session.revision(), actor(), &pin)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        session
            .commit(
                materials::prepare_tags(
                    &session,
                    session.revision(),
                    actor(),
                    &[panel.body],
                    &TagEdit::Add(["cnc".into()].into()),
                )
                .unwrap(),
            )
            .unwrap();
        let q = (std::f64::consts::FRAC_PI_4).sin();
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
        let ids = [instances[0].id, instances[1].id];
        let assembly_id = assembly.id;
        let mut edits: Vec<_> = instances.into_iter().map(Mutation::Insert).collect();
        edits.push(Mutation::Insert(assembly));
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Place copies".into(),
                edits,
            })
            .unwrap();
        let set = EntityId::new();
        let mut f = Self {
            session,
            panel,
            pin,
            instances: ids,
            assembly: assembly_id,
            set,
        };
        f.define(SetDefinition::default());
        f
    }
    fn define(&mut self, definition: SetDefinition) {
        let p = prepare_set(
            &self.session,
            self.session.revision(),
            actor(),
            self.set,
            "Batch",
            &definition,
        )
        .unwrap();
        self.session.commit(p).unwrap();
    }
    fn preview(&self) -> ExportPreview {
        Exporter::default()
            .preview(
                self.session.document(),
                self.session.revision(),
                self.set,
                &actor(),
                None,
            )
            .unwrap()
    }
    fn run(&self, preview: &ExportPreview, root: &Path) -> Result<RunReceipt, ExportError> {
        Exporter::default().run(
            self.session.document(),
            self.session.revision(),
            &preview.key,
            root,
            &actor(),
            None,
        )
    }
}
fn mesh(format: MeshFormat, units: LengthUnit, hierarchy: HierarchyMode) -> Recipe {
    Recipe {
        id: EntityId::new(),
        name: format!("{format:?} {units:?}"),
        output: RecipeOutput::Mesh {
            format,
            units,
            hierarchy,
            names: NameHandling::ProductNames,
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality {
                chord_deviation_mm: 0.05,
                angular_deviation_deg: 10.,
            },
        },
    }
}

/// Independent OBJ reading: objects with their world triangles in file units.
type Triangles = Vec<[[f64; 3]; 3]>;
fn parse_obj(text: &str) -> (Option<String>, BTreeMap<String, Triangles>) {
    let (mut unit, mut vertices, mut objects) = (None, vec![], BTreeMap::new());
    let mut current = String::new();
    for line in text.lines() {
        if let Some(u) = line.strip_prefix("# units: ") {
            unit = Some(u.to_owned());
        } else if let Some(name) = line.strip_prefix("o ") {
            current = name.to_owned();
            objects.insert(current.clone(), vec![]);
        } else if let Some(v) = line.strip_prefix("v ") {
            let p: Vec<f64> = v.split(' ').map(|x| x.parse().unwrap()).collect();
            vertices.push([p[0], p[1], p[2]]);
        } else if let Some(f) = line.strip_prefix("f ") {
            let i: Vec<usize> = f.split(' ').map(|x| x.parse().unwrap()).collect();
            objects.get_mut(&current).unwrap().push([
                vertices[i[0] - 1],
                vertices[i[1] - 1],
                vertices[i[2] - 1],
            ]);
        }
    }
    (unit, objects)
}
fn volume(triangles: &[[[f64; 3]; 3]]) -> f64 {
    triangles
        .iter()
        .map(|[a, b, c]| {
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        .abs()
        / 6.
}
fn bounds(triangles: &[[[f64; 3]; 3]]) -> [[f64; 3]; 2] {
    let mut b = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for p in triangles.iter().flatten() {
        for i in 0..3 {
            b[0][i] = b[0][i].min(p[i]);
            b[1][i] = b[1][i].max(p[i]);
        }
    }
    b
}
/// Independent ZIP reading (local headers, then the central directory).
fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let u16le = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32le = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let eocd = (0..bytes.len() - 21)
        .rev()
        .find(|&i| u32le(i) == 0x0605_4b50)
        .expect("end of central directory");
    let (count, mut at) = (u16le(eocd + 10), u32le(eocd + 16));
    let mut files = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(u32le(at), 0x0201_4b50);
        let (crc, size, name_len) = (u32le(at + 16), u32le(at + 24), u16le(at + 28));
        let extra_comment = u16le(at + 30) + u16le(at + 32);
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec()).unwrap();
        let local = u32le(at + 42);
        let data_at = local + 30 + u16le(local + 26) + u16le(local + 28);
        let data = bytes[data_at..data_at + size].to_vec();
        assert_eq!(crc32fast::hash(&data) as usize, crc, "{name} CRC");
        files.insert(name, data);
        at += 46 + name_len + extra_comment;
    }
    files
}
/// Independent 3MF model reading with quick-xml: unit, objects and placed
/// build items as world triangles in file units.
/// Name, vertices and triangles of one 3MF object.
type ObjectRecord = (String, Vec<[f64; 3]>, Vec<[usize; 3]>);
struct ThreeMf {
    unit: String,
    objects: Vec<ObjectRecord>,
    items: Vec<(usize, [f64; 12])>,
}
fn parse_3mf(xml: &[u8]) -> ThreeMf {
    let mut reader = Reader::from_reader(xml);
    let mut model = ThreeMf {
        unit: String::new(),
        objects: vec![],
        items: vec![],
    };
    let mut buffer = vec![];
    loop {
        match reader.read_event_into(&mut buffer).unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                let attr = |name: &[u8]| {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == name)
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                };
                let f = |name: &[u8]| attr(name).unwrap().parse::<f64>().unwrap();
                match e.name().as_ref() {
                    b"model" => model.unit = attr(b"unit").unwrap(),
                    b"object" => {
                        model
                            .objects
                            .push((attr(b"name").unwrap_or_default(), vec![], vec![]))
                    }
                    b"vertex" => {
                        model
                            .objects
                            .last_mut()
                            .unwrap()
                            .1
                            .push([f(b"x"), f(b"y"), f(b"z")])
                    }
                    b"triangle" => model.objects.last_mut().unwrap().2.push([
                        f(b"v1") as usize,
                        f(b"v2") as usize,
                        f(b"v3") as usize,
                    ]),
                    b"item" => {
                        let m: Vec<f64> = attr(b"transform")
                            .unwrap()
                            .split(' ')
                            .map(|v| v.parse().unwrap())
                            .collect();
                        model
                            .items
                            .push((f(b"objectid") as usize - 1, m.try_into().unwrap()));
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    model
}
impl ThreeMf {
    fn placed(&self, item: usize) -> Vec<[[f64; 3]; 3]> {
        let (object, m) = &self.items[item];
        let (_, vertices, triangles) = &self.objects[*object];
        let place = |p: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|j| p[0] * m[j] + p[1] * m[3 + j] + p[2] * m[6 + j] + m[9 + j])
        };
        triangles
            .iter()
            .map(|t| t.map(|i| place(vertices[i])))
            .collect()
    }
}
fn system_unzip_accepts(path: &Path) -> Option<bool> {
    std::process::Command::new("unzip")
        .arg("-tq")
        .arg(path)
        .output()
        .ok()
        .map(|o| o.status.success())
}

#[test]
fn separate_obj_and_inch_3mf_files_match_exact_world_geometry() {
    let mut f = Fixture::new();
    f.define(SetDefinition {
        recipes: vec![
            mesh(
                MeshFormat::Obj,
                LengthUnit::Millimeter,
                HierarchyMode::SeparateParts,
            ),
            mesh(
                MeshFormat::ThreeMf,
                LengthUnit::Inch,
                HierarchyMode::SeparateParts,
            ),
        ],
        ..Default::default()
    });
    let preview = f.preview();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    // Panel, its two placed copies and the pin, in both formats.
    assert_eq!(preview.artifacts.len(), 8);
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview, root.path()).unwrap();
    assert_eq!(receipt.files.len(), 8);
    for file in &receipt.files {
        let path = Path::new(&receipt.directory).join(&file.artifact.output.filename);
        let bytes = std::fs::read(&path).unwrap();
        let a = &file.artifact;
        let exact = a.bounds_mm;
        let expected_volume = if a.solid == f.pin.body || a.producer == f.pin.feature {
            PI * 25. * 10.
        } else {
            6000.
        };
        let (triangles, scale) = if a.output.filename.ends_with(".obj") {
            let (unit, objects) = parse_obj(std::str::from_utf8(&bytes).unwrap());
            assert_eq!(unit.as_deref(), Some("millimeter"));
            assert_eq!(objects.len(), 1);
            (objects.into_values().next().unwrap(), 1.)
        } else {
            if let Some(ok) = system_unzip_accepts(&path) {
                assert!(ok, "system unzip rejects {}", path.display());
            }
            let parts = unzip(&bytes);
            assert!(parts.contains_key("[Content_Types].xml") && parts.contains_key("_rels/.rels"));
            let model = parse_3mf(&parts["3D/3dmodel.model"]);
            assert_eq!(model.unit, "inch");
            assert_eq!((model.objects.len(), model.items.len()), (1, 1));
            (model.placed(0), 25.4)
        };
        let world: Vec<_> = triangles
            .iter()
            .map(|t| t.map(|p| p.map(|v| v * scale)))
            .collect();
        let b = bounds(&world);
        for i in 0..3 {
            near(b[0][i], exact[0][i], 0.051);
            near(b[1][i], exact[1][i], 0.051);
        }
        // A tessellated box is exact; a cylinder is inscribed within the chord.
        let v = volume(&world);
        if expected_volume == 6000. {
            near(v, 6000., 1e-6);
        } else {
            assert!(v < expected_volume && v > expected_volume * 0.99, "{v}");
        }
        assert_eq!(a.mesh.as_ref().unwrap().triangles, world.len());
    }
    // The rotated copy's OBJ lies where its STEP lies: x 30–50, y 5–15.
    let rotated = receipt
        .files
        .iter()
        .find(|file| {
            file.artifact.solid == f.instances[0] && file.artifact.output.filename.ends_with(".obj")
        })
        .unwrap();
    let text = std::fs::read_to_string(
        Path::new(&receipt.directory).join(&rotated.artifact.output.filename),
    )
    .unwrap();
    let (_, objects) = parse_obj(&text);
    let b = bounds(objects.values().next().unwrap());
    near(b[0][0], 30., 1e-6);
    near(b[1][0], 50., 1e-6);
    near(b[0][1], 5., 1e-6);
    near(b[1][2], 32., 1e-6);
    assert!(objects.keys().next().unwrap().starts_with("Panel"));
}

#[test]
fn assembly_3mf_shares_one_definition_placed_by_build_items_without_double_transforms() {
    let mut f = Fixture::new();
    f.define(SetDefinition {
        scope: Scope {
            root: ScopeRoot::Assembly {
                assembly: f.assembly,
            },
            tags: None,
        },
        recipes: vec![
            mesh(
                MeshFormat::ThreeMf,
                LengthUnit::Millimeter,
                HierarchyMode::PreserveAssembly,
            ),
            mesh(
                MeshFormat::Obj,
                LengthUnit::Centimeter,
                HierarchyMode::PreserveAssembly,
            ),
        ],
        ..Default::default()
    });
    let preview = f.preview();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    assert_eq!(
        preview.artifacts.len(),
        2,
        "one file per format for the whole assembly"
    );
    let three = preview
        .artifacts
        .iter()
        .find(|a| a.output.filename.ends_with(".3mf"))
        .unwrap();
    let evidence = three.mesh.as_ref().unwrap();
    assert_eq!((evidence.objects, evidence.build_items), (1, 2));
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview, root.path()).unwrap();
    let path = |a: &ExportArtifact| Path::new(&receipt.directory).join(&a.output.filename);
    let model = parse_3mf(&unzip(&std::fs::read(path(three)).unwrap())["3D/3dmodel.model"]);
    assert_eq!(model.unit, "millimeter");
    assert_eq!(model.objects.len(), 1);
    // The shared object is the local definition (0–10, 0–20, 0–30).
    let local = bounds(
        &model.objects[0]
            .2
            .iter()
            .map(|t| t.map(|i| model.objects[0].1[i]))
            .collect::<Vec<_>>(),
    );
    assert_eq!(local, [[0.; 3], [10., 20., 30.]]);
    let mut placed: Vec<_> = (0..2).map(|i| bounds(&model.placed(i))).collect();
    placed.sort_by(|a, b| a[0][0].total_cmp(&b[0][0]));
    let expect = [
        [[-20., 40., 0.], [-10., 60., 30.]],
        [[30., 5., 2.], [50., 15., 32.]],
    ];
    for (b, e) in placed.iter().zip(expect) {
        for side in 0..2 {
            for i in 0..3 {
                near(b[side][i], e[side][i], 1e-6);
            }
        }
    }
    for item in 0..2 {
        near(volume(&model.placed(item)), 6000., 1e-6);
    }
    // OBJ has no instancing: two world-space objects, here in centimeters.
    let obj = preview
        .artifacts
        .iter()
        .find(|a| a.output.filename.ends_with(".obj"))
        .unwrap();
    let (unit, objects) = parse_obj(&std::fs::read_to_string(path(obj)).unwrap());
    assert_eq!(unit.as_deref(), Some("centimeter"));
    assert_eq!(objects.len(), 2);
    let mut total = 0.;
    for triangles in objects.values() {
        total += volume(triangles);
    }
    near(total, 2. * 6000. / 1000., 1e-9);
}

#[test]
fn tag_scopes_are_reproducible_and_unsupported_or_failed_runs_leave_everything_unchanged() {
    let mut f = Fixture::new();
    f.define(SetDefinition {
        scope: Scope {
            root: ScopeRoot::Project,
            tags: Some(TagQuery {
                all: ["cnc".into()].into(),
                ..Default::default()
            }),
        },
        recipes: vec![mesh(
            MeshFormat::ThreeMf,
            LengthUnit::Meter,
            HierarchyMode::SeparateParts,
        )],
        ..Default::default()
    });
    let preview = f.preview();
    // The tag selects the Panel and its two copies, never the untagged pin.
    assert_eq!(preview.artifacts.len(), 3);
    assert!(
        preview
            .artifacts
            .iter()
            .all(|a| a.producer == f.panel.feature)
    );
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let first = f.run(&preview, first_root.path()).unwrap();
    let second = f.run(&f.preview(), second_root.path()).unwrap();
    let bytes = |r: &RunReceipt| -> Vec<Vec<u8>> {
        r.files
            .iter()
            .map(|file| {
                std::fs::read(Path::new(&r.directory).join(&file.artifact.output.filename)).unwrap()
            })
            .collect()
    };
    assert_eq!(
        bytes(&first),
        bytes(&second),
        "identical source gives identical files"
    );
    let again = f.run(&preview, first_root.path()).unwrap();
    assert!(again.reused);

    let before = ContentHash::document(f.session.document()).unwrap();
    // Removable-aid exclusion is supported now; see tests/fdm_aids.rs.
    for (recipe, reason) in [
        (
            mesh(
                MeshFormat::Glb,
                LengthUnit::Millimeter,
                HierarchyMode::SeparateParts,
            ),
            "meters",
        ),
        (
            Recipe {
                output: RecipeOutput::Mesh {
                    format: MeshFormat::Obj,
                    units: LengthUnit::Millimeter,
                    hierarchy: HierarchyMode::SeparateParts,
                    names: NameHandling::FileNamesOnly,
                    aids: AidPolicy::ExcludeRemovable,
                    quality: MeshQuality {
                        chord_deviation_mm: 20.,
                        angular_deviation_deg: 15.,
                    },
                },
                ..mesh(
                    MeshFormat::Obj,
                    LengthUnit::Millimeter,
                    HierarchyMode::SeparateParts,
                )
            },
            "Mesh quality",
        ),
    ] {
        f.define(SetDefinition {
            recipes: vec![recipe],
            ..Default::default()
        });
        let preview = f.preview();
        assert!(!preview.can_run());
        assert!(
            matches!(&preview.recipes[0].status, RecipeStatus::Unsupported(r) if r.contains(reason))
        );
        let root = tempfile::tempdir().unwrap();
        assert!(f.run(&preview, root.path()).is_err());
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            0,
            "nothing written"
        );
    }
    assert_ne!(
        ContentHash::document(f.session.document()).unwrap(),
        before,
        "only the set edits changed"
    );
    // The earlier bundle is untouched by the failed runs.
    assert_eq!(bytes(&first), bytes(&second));
}

/// Independent binary STL reading: header text and triangles in file units.
fn parse_stl(bytes: &[u8]) -> (String, Triangles) {
    let header = String::from_utf8_lossy(&bytes[..80]).trim_end().to_owned();
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + count * 50, "binary STL length");
    let f = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as f64;
    let triangles = (0..count)
        .map(|t| {
            let at = 84 + t * 50 + 12;
            std::array::from_fn(|v| std::array::from_fn(|i| f(at + v * 12 + i * 4)))
        })
        .collect();
    (header, triangles)
}
/// Independent GLB reading: the JSON document, and every drawn triangle as
/// (node name, mesh index, triangle) converted back to Z-up millimeters by
/// composing the node matrices and rotations from the scene root.
struct Glb {
    json: serde_json::Value,
    drawn: Vec<(String, usize, [[f64; 3]; 3])>,
}
fn parse_glb(bytes: &[u8]) -> Glb {
    use serde_json::Value;
    let u32le = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"glTF");
    assert_eq!(u32le(4), 2);
    assert_eq!(u32le(8), bytes.len());
    let json_len = u32le(12);
    assert_eq!(&bytes[16..20], b"JSON");
    let json: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    let bin_at = 20 + json_len;
    assert_eq!(&bytes[bin_at + 4..bin_at + 8], b"BIN\0");
    let bin = &bytes[bin_at + 8..bin_at + 8 + u32le(bin_at)];
    let accessor = |index: usize| -> Vec<f64> {
        let a = &json["accessors"][index];
        let view = &json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let width = if a["type"] == "VEC3" { 3 } else { 1 };
        let count = a["count"].as_u64().unwrap() as usize * width;
        (0..count)
            .map(|i| {
                let raw: [u8; 4] = bin[offset + i * 4..offset + i * 4 + 4].try_into().unwrap();
                match a["componentType"].as_u64().unwrap() {
                    5126 => f32::from_le_bytes(raw) as f64,
                    5125 => u32::from_le_bytes(raw) as f64,
                    other => panic!("component type {other}"),
                }
            })
            .collect()
    };
    type M = [[f64; 4]; 4];
    let mul = |a: &M, b: &M| -> M {
        std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum()))
    };
    let local = |node: &Value| -> M {
        if let Some(m) = node["matrix"].as_array() {
            let m: Vec<f64> = m.iter().map(|v| v.as_f64().unwrap()).collect();
            std::array::from_fn(|i| std::array::from_fn(|j| m[j * 4 + i]))
        } else {
            let q: Vec<f64> = node["rotation"]
                .as_array()
                .map(|r| r.iter().map(|v| v.as_f64().unwrap()).collect())
                .unwrap_or(vec![0., 0., 0., 1.]);
            let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
            [
                [
                    1. - 2. * (y * y + z * z),
                    2. * (x * y - z * w),
                    2. * (x * z + y * w),
                    0.,
                ],
                [
                    2. * (x * y + z * w),
                    1. - 2. * (x * x + z * z),
                    2. * (y * z - x * w),
                    0.,
                ],
                [
                    2. * (x * z - y * w),
                    2. * (y * z + x * w),
                    1. - 2. * (x * x + y * y),
                    0.,
                ],
                [0., 0., 0., 1.],
            ]
        }
    };
    let mut drawn = vec![];
    let identity: M = std::array::from_fn(|i| std::array::from_fn(|j| f64::from(i == j)));
    let mut stack: Vec<(usize, M)> = json["scenes"][0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n.as_u64().unwrap() as usize, identity))
        .collect();
    while let Some((index, parent)) = stack.pop() {
        let node = &json["nodes"][index];
        let world = mul(&parent, &local(node));
        if let Some(mesh) = node["mesh"].as_u64() {
            let primitive = &json["meshes"][mesh as usize]["primitives"][0];
            let p = accessor(primitive["attributes"]["POSITION"].as_u64().unwrap() as usize);
            let i = accessor(primitive["indices"].as_u64().unwrap() as usize);
            for t in i.chunks(3) {
                let tri = std::array::from_fn(|v| {
                    let at = t[v] as usize * 3;
                    let q: [f64; 3] = std::array::from_fn(|r| {
                        (0..3).map(|c| world[r][c] * p[at + c]).sum::<f64>() + world[r][3]
                    });
                    // Y-up meters back to Z-up millimeters.
                    [q[0] * 1000., -q[2] * 1000., q[1] * 1000.]
                });
                drawn.push((
                    node["name"].as_str().unwrap().to_owned(),
                    mesh as usize,
                    tri,
                ));
            }
        }
        for child in node["children"].as_array().into_iter().flatten() {
            stack.push((child.as_u64().unwrap() as usize, world));
        }
    }
    Glb { json, drawn }
}
fn written(receipt: &RunReceipt, extension: &str) -> Vec<Vec<u8>> {
    receipt
        .files
        .iter()
        .filter(|f| f.artifact.output.filename.ends_with(extension))
        .map(|f| {
            std::fs::read(Path::new(&receipt.directory).join(&f.artifact.output.filename)).unwrap()
        })
        .collect()
}
fn drawn_by(glb: &Glb, name: &str) -> Triangles {
    glb.drawn
        .iter()
        .filter(|(n, ..)| n == name)
        .map(|(.., t)| *t)
        .collect()
}
fn assert_bounds(b: [[f64; 3]; 2], e: [[f64; 3]; 2], tolerance: f64) {
    for side in 0..2 {
        for i in 0..3 {
            near(b[side][i], e[side][i], tolerance);
        }
    }
}

fn next_reply(worker: &ExportWorker) -> ExportResult {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(reply) = worker.poll() {
            return reply.result;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "export worker timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn assert_closed_winding(triangles: &Triangles) {
    let mut edges = BTreeMap::new();
    for triangle in triangles {
        let ids = triangle.map(|p| p.map(|v| if v == 0. { 0 } else { v.to_bits() }));
        for (a, b) in [(ids[0], ids[1]), (ids[1], ids[2]), (ids[2], ids[0])] {
            assert_ne!(a, b, "collapsed edge in written mesh");
            let uses = edges.entry([a.min(b), a.max(b)]).or_insert([0; 2]);
            uses[usize::from(a > b)] += 1;
        }
    }
    assert!(!edges.is_empty());
    assert!(edges.values().all(|uses| *uses == [1, 1]));
}

#[test]
fn rust_export_worker_round_trips_four_certified_mesh_formats_and_assembly_placements() {
    let mut f = Fixture::new();
    f.define(SetDefinition {
        recipes: [
            (MeshFormat::Obj, LengthUnit::Millimeter),
            (MeshFormat::Stl, LengthUnit::Millimeter),
            (MeshFormat::ThreeMf, LengthUnit::Millimeter),
            (MeshFormat::Glb, LengthUnit::Meter),
        ]
        .into_iter()
        .map(|(format, unit)| mesh(format, unit, HierarchyMode::PreserveAssembly))
        .collect(),
        ..Default::default()
    });
    let document = f.session.document();
    let revision = f.session.revision();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    worker.preview(document, revision, f.set, actor()).unwrap();
    let ExportResult::Preview(preview) = next_reply(&worker) else {
        panic!("expected mesh preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    assert!(preview.provider.contains("mesh-files.rust-certified.1"));
    assert!(!preview.provider.contains("occt:"));
    assert_eq!(preview.artifacts.len(), 4);
    let root = tempfile::tempdir().unwrap();
    worker
        .run(document, revision, &preview.key, root.path(), actor())
        .unwrap();
    let ExportResult::Run(receipt) = next_reply(&worker) else {
        panic!("expected mesh publication");
    };
    let receipt = receipt.unwrap();
    assert_eq!(receipt.files.len(), 4);
    for file in &receipt.files {
        let name = &file.artifact.output.filename;
        let bytes = std::fs::read(Path::new(&receipt.directory).join(name)).unwrap();
        let evidence = file.artifact.mesh.as_ref().unwrap();
        assert_eq!(evidence.build_items, 4);
        let triangles: Triangles = if name.ends_with(".obj") {
            let (unit, objects) = parse_obj(std::str::from_utf8(&bytes).unwrap());
            assert_eq!(unit.as_deref(), Some("millimeter"));
            assert_eq!(objects.len(), 4);
            objects.into_values().flatten().collect()
        } else if name.ends_with(".stl") {
            let (header, triangles) = parse_stl(&bytes);
            assert!(header.contains("units=millimeter"), "{header}");
            triangles
        } else if name.ends_with(".3mf") {
            let entries = unzip(&bytes);
            let package = parse_3mf(&entries["3D/3dmodel.model"]);
            assert_eq!(package.unit, "millimeter");
            assert_eq!((package.objects.len(), package.items.len()), (3, 4));
            (0..package.items.len())
                .flat_map(|i| package.placed(i))
                .collect()
        } else {
            assert!(name.ends_with(".glb"), "unexpected artifact {name}");
            let glb = parse_glb(&bytes);
            assert_eq!(glb.json["meshes"].as_array().unwrap().len(), 3);
            let rotated = drawn_by(&glb, "Panel Rotated copy");
            assert_bounds(bounds(&rotated), [[30., 5., 2.], [50., 15., 32.]], 1e-4);
            glb.drawn
                .into_iter()
                .map(|(_, _, triangle)| triangle)
                .collect()
        };
        assert_eq!(triangles.len(), evidence.triangles);
        assert_closed_winding(&triangles);
        assert_bounds(
            bounds(&triangles),
            [[-20., -5., 0.], [105., 60., 32.]],
            0.051,
        );
        near(volume(&triangles), 3. * 6000. + PI * 25. * 10., 6.);
    }
}

#[test]
fn rust_export_worker_writes_watertight_metric_threads_to_stl_and_3mf() {
    rust_export_metric_thread(false);
}

#[test]
fn rust_export_worker_writes_watertight_metric_receivers_to_stl_and_3mf() {
    rust_export_metric_thread(true);
}

#[test]
fn rust_export_worker_writes_one_closed_headed_screw_with_a_hex_socket_floor() {
    let mut session = Session::new(Document::new("Headed screw exports").unwrap()).unwrap();
    let request =
        model::MechanicalPartRequest::new("M3 socket screw", model::default_machine_screw());
    let prepared = model::prepare_mechanical_part(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let set = EntityId::new();
    let definition = SetDefinition {
        recipes: [MeshFormat::Stl, MeshFormat::ThreeMf]
            .into_iter()
            .map(|format| mesh(format, LengthUnit::Millimeter, HierarchyMode::SeparateParts))
            .collect(),
        ..Default::default()
    };
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        set,
        "Manufacturing",
        &definition,
    )
    .unwrap();
    session.commit(prepared).unwrap();
    let document = session.document();
    let revision = session.revision();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    worker.preview(document, revision, set, actor()).unwrap();
    let ExportResult::Preview(preview) = next_reply(&worker) else {
        panic!("Expected screw mesh preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let directory = tempfile::tempdir().unwrap();
    worker
        .run(document, revision, &preview.key, directory.path(), actor())
        .unwrap();
    let ExportResult::Run(receipt) = next_reply(&worker) else {
        panic!("Expected screw mesh export");
    };
    let receipt = receipt.unwrap();
    assert_eq!(receipt.files.len(), 2);
    for file in &receipt.files {
        let artifact = &file.artifact;
        assert_eq!(artifact.producer, request.feature);
        assert_eq!(artifact.solid_count, 1);
        let evidence = artifact.mesh.as_ref().unwrap();
        assert_eq!((evidence.objects, evidence.build_items), (1, 1));
        let name = &artifact.output.filename;
        let bytes = std::fs::read(Path::new(&receipt.directory).join(name)).unwrap();
        let triangles = if name.ends_with(".stl") {
            parse_stl(&bytes).1
        } else {
            assert!(name.ends_with(".3mf"));
            let entries = unzip(&bytes);
            let package = parse_3mf(&entries["3D/3dmodel.model"]);
            assert_eq!((package.objects.len(), package.items.len()), (1, 1));
            package.placed(0)
        };
        assert_eq!(triangles.len(), evidence.triangles);
        assert_closed_winding(&triangles);
        assert_bounds(
            bounds(&triangles),
            [[-2.84, -2.84, -3.], [2.84, 2.84, 10.]],
            0.05,
        );
        near(volume(&triangles), evidence.mesh_volume_mm3, 0.001);
        let floor_area: f64 = triangles
            .iter()
            .filter(|points| points.iter().all(|p| (p[2] + 1.7).abs() < 1e-6))
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.
            })
            .sum();
        near(floor_area, 3_f64.sqrt() * 2.55_f64.powi(2) / 2., 1e-5);
        assert!(
            triangles
                .iter()
                .flatten()
                .any(|p| (2. ..8.).contains(&p[2]) && p[0].hypot(p[1]) < 1.24)
        );
    }
}

fn rust_export_metric_thread(internal: bool) {
    use model::{BodyInput, ThreadHand, ThreadMode, ThreadParameters, ThreadRequest};

    let mut session = Session::new(Document::new("Thread manufacturing exports").unwrap()).unwrap();
    let parameters = ThreadParameters {
        length: length(9.2),
        hand: ThreadHand::Left,
        ..Default::default()
    };
    let thread = if internal {
        let stock = PrimitiveRequest::new(
            "Round stock",
            PrimitiveParameters::Cylinder {
                radius: length(6.),
                height: length(9.2),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&session, 0, actor(), &stock).unwrap();
        session.commit(prepared.prepared).unwrap();
        ThreadRequest::receiving(
            "M8 × 1.25 left bore",
            ThreadMode::CutInternal,
            BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
            parameters,
        )
        .unwrap()
    } else {
        ThreadRequest::new("M8 × 1.25 left shaft", parameters)
    };
    let prepared = model::prepare_thread(&session, session.revision(), actor(), &thread).unwrap();
    session.commit(prepared.prepared).unwrap();
    let set = EntityId::new();
    let definition = SetDefinition {
        recipes: [MeshFormat::Stl, MeshFormat::ThreeMf]
            .into_iter()
            .map(|format| mesh(format, LengthUnit::Millimeter, HierarchyMode::SeparateParts))
            .collect(),
        ..Default::default()
    };
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        set,
        "Manufacturing",
        &definition,
    )
    .unwrap();
    session.commit(prepared).unwrap();

    let document = session.document();
    let revision = session.revision();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    worker.preview(document, revision, set, actor()).unwrap();
    let ExportResult::Preview(preview) = next_reply(&worker) else {
        panic!("expected thread mesh preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    assert!(preview.provider.contains("mesh-files.rust-certified.1"));
    let directory = tempfile::tempdir().unwrap();
    worker
        .run(document, revision, &preview.key, directory.path(), actor())
        .unwrap();
    let ExportResult::Run(receipt) = next_reply(&worker) else {
        panic!("expected thread mesh publication");
    };
    let receipt = receipt.unwrap();
    assert_eq!(receipt.files.len(), 2);
    for file in &receipt.files {
        let artifact = &file.artifact;
        let name = &artifact.output.filename;
        assert_eq!(artifact.producer, thread.feature);
        assert_eq!(artifact.solid_count, 1);
        let evidence = artifact.mesh.as_ref().unwrap();
        assert_eq!((evidence.objects, evidence.build_items), (1, 1));
        assert_eq!(evidence.chord_deviation_mm, 0.05);

        let bytes = std::fs::read(Path::new(&receipt.directory).join(name)).unwrap();
        let triangles = if name.ends_with(".stl") {
            let (header, triangles) = parse_stl(&bytes);
            assert!(header.contains("units=millimeter"), "{header}");
            triangles
        } else {
            assert!(name.ends_with(".3mf"), "unexpected artifact {name}");
            let entries = unzip(&bytes);
            let package = parse_3mf(&entries["3D/3dmodel.model"]);
            assert_eq!(package.unit, "millimeter");
            assert_eq!((package.objects.len(), package.items.len()), (1, 1));
            package.placed(0)
        };
        assert_eq!(triangles.len(), evidence.triangles);
        assert_closed_winding(&triangles);
        let radius = if internal { 6. } else { 4. };
        assert_bounds(
            bounds(&triangles),
            [[-radius, -radius, 0.], [radius, radius, 9.2]],
            0.05,
        );
        near(volume(&triangles), evidence.mesh_volume_mm3, 0.001);
        near(
            evidence.mesh_volume_mm3,
            artifact.volume_mm3,
            if internal { 5. } else { 2. },
        );
        // The threaded boundary has a distinct crest and root even in the
        // receiver, which also has a constant-radius outer wall.
        let mut wall_radii = triangles
            .iter()
            .flatten()
            .filter(|p| (1. ..8.).contains(&p[2]))
            .map(|p| p[0].hypot(p[1]));
        assert!(
            wall_radii
                .clone()
                .any(|radius| (3.99..4.01).contains(&radius))
        );
        assert!(wall_radii.any(|radius| (3.3..3.4).contains(&radius)));
        // For a left-hand thread the crest phase stays constant as Z falls
        // with increasing angle, with a tolerance for its physical flat width.
        let crest_vertices = triangles
            .iter()
            .flatten()
            .filter(|p| (1. ..8.).contains(&p[2]) && (p[0].hypot(p[1]) - 4.).abs() < 1e-5);
        assert!(crest_vertices.clone().count() > 100);
        for p in crest_vertices {
            let phase = (p[2] + p[1].atan2(p[0]) * 1.25 / std::f64::consts::TAU).rem_euclid(1.25);
            assert!(phase.min(1.25 - phase) < 1.25 / 16. + 1e-5);
        }
    }
}

#[test]
fn rust_export_refuses_collapsed_float_positions_and_keeps_glb_resource_placements_separate() {
    let mut f = Fixture::new();
    let distant = [9_000_000., 0., 0.];
    let mut thin_box = |origin: [f64; 3]| {
        let part = PrimitiveRequest::new(
            "Thin part",
            PrimitiveParameters::Box {
                size: [length(0.01), length(2.), length(3.)],
                frame: PrimitiveFrame {
                    origin: origin.map(length),
                    ..Default::default()
                },
            },
        );
        let prepared =
            model::prepare_primitive(&f.session, f.session.revision(), actor(), &part).unwrap();
        f.session.commit(prepared.prepared).unwrap();
        part.body
    };
    let distant_part = thin_box(distant);
    let local_part = thin_box([0.; 3]);
    let copy = Entity::new(
        "Distant copy",
        EntityData::Instance {
            component: local_part,
            placement: Placement {
                translation_mm: distant,
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let instance = copy.id;
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Place distant thin copy".into(),
            edits: vec![Mutation::Insert(copy)],
        })
        .unwrap();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    for entity in [distant_part, instance] {
        f.define(SetDefinition {
            scope: Scope {
                root: ScopeRoot::Explicit {
                    entities: vec![entity],
                },
                tags: None,
            },
            recipes: [
                (MeshFormat::Stl, LengthUnit::Millimeter),
                (MeshFormat::Glb, LengthUnit::Meter),
                (MeshFormat::ThreeMf, LengthUnit::Millimeter),
            ]
            .into_iter()
            .map(|(format, unit)| mesh(format, unit, HierarchyMode::SeparateParts))
            .collect(),
            ..Default::default()
        });
        let document = f.session.document();
        let revision = f.session.revision();
        worker.preview(document, revision, f.set, actor()).unwrap();
        let ExportResult::Preview(preview) = next_reply(&worker) else {
            panic!("expected precision-checked preview");
        };
        let preview = preview.unwrap();
        let [stl, glb, three_mf] = preview.recipes.as_slice() else {
            panic!("expected the three mesh recipes");
        };
        let is_collapsed = |status: &RecipeStatus| matches!(status, RecipeStatus::Unsupported(reason) if reason.contains("single-precision output collapses distinct vertices"));
        assert!(is_collapsed(&stl.status), "{stl:?}");
        if entity == distant_part {
            assert!(is_collapsed(&glb.status), "{glb:?}");
        } else {
            assert_eq!(glb.status, RecipeStatus::Ready);
        }
        assert_eq!(three_mf.status, RecipeStatus::Ready);
        assert!(!preview.can_run());
        let root = tempfile::tempdir().unwrap();
        worker
            .run(document, revision, &preview.key, root.path(), actor())
            .unwrap();
        assert!(matches!(next_reply(&worker), ExportResult::Run(Err(_))));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn stl_files_hold_exact_world_meshes_in_the_stated_unit_and_repeat_byte_for_byte() {
    let mut f = Fixture::new();
    f.define(SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![f.panel.body, f.instances[0]],
            },
            tags: None,
        },
        recipes: vec![
            mesh(
                MeshFormat::Stl,
                LengthUnit::Millimeter,
                HierarchyMode::SeparateParts,
            ),
            mesh(
                MeshFormat::Stl,
                LengthUnit::Inch,
                HierarchyMode::PreserveAssembly,
            ),
        ],
        ..Default::default()
    });
    let preview = f.preview();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview, root.path()).unwrap();
    let mut separate = vec![];
    let mut combined = vec![];
    for file in &receipt.files {
        let bytes =
            std::fs::read(Path::new(&receipt.directory).join(&file.artifact.output.filename))
                .unwrap();
        let (header, triangles) = parse_stl(&bytes);
        if header.contains("units=millimeter") {
            separate.push(triangles);
        } else {
            assert!(
                header.contains("units=inch (not stored by STL)"),
                "{header}"
            );
            combined.push(triangles);
        }
    }
    // Separate parts: the panel at its origin and the rotated copy in world.
    assert_eq!(separate.len(), 2);
    let mut boxes: Vec<_> = separate.iter().map(|t| bounds(t)).collect();
    boxes.sort_by(|a, b| a[0][0].total_cmp(&b[0][0]));
    assert_bounds(boxes[0], [[0.; 3], [10., 20., 30.]], 1e-4);
    assert_bounds(boxes[1], [[30., 5., 2.], [50., 15., 32.]], 1e-4);
    for t in &separate {
        near(volume(t), 6000., 1e-2);
    }
    // One assembly-preserving STL in inches: both bodies, no instancing.
    assert_eq!(combined.len(), 1);
    near(volume(&combined[0]), 2. * 6000. / 25.4f64.powi(3), 1e-5);
    // Facet normals agree with the vertex winding (outward for a closed body).
    let bytes = written(&receipt, ".stl");
    let n = |at: usize| f32::from_le_bytes(bytes[0][at..at + 4].try_into().unwrap()) as f64;
    let t = &parse_stl(&bytes[0]).1[0];
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    assert!(cross[0] * n(84) + cross[1] * n(88) + cross[2] * n(92) > 0.);
    // Reproducible tessellation: a second run into another root is identical.
    let other = tempfile::tempdir().unwrap();
    let again = Exporter::default()
        .run(
            f.session.document(),
            f.session.revision(),
            &preview.key,
            other.path(),
            &Actor::local(Channel::Mcp),
            None,
        )
        .unwrap();
    assert_eq!(written(&receipt, ".stl"), written(&again, ".stl"));
}

#[test]
fn assembly_glb_shares_one_mesh_between_named_placed_instances_at_meter_scale() {
    let mut f = Fixture::new();
    f.define(SetDefinition {
        scope: Scope {
            root: ScopeRoot::Assembly {
                assembly: f.assembly,
            },
            tags: None,
        },
        recipes: vec![mesh(
            MeshFormat::Glb,
            LengthUnit::Meter,
            HierarchyMode::PreserveAssembly,
        )],
        ..Default::default()
    });
    let preview = f.preview();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    assert_eq!(preview.artifacts.len(), 1);
    let evidence = preview.artifacts[0].mesh.as_ref().unwrap();
    assert_eq!((evidence.objects, evidence.build_items), (1, 2));
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview, root.path()).unwrap();
    let glb = parse_glb(&written(&receipt, ".glb")[0]);
    let json = &glb.json;
    assert_eq!(json["asset"]["version"], "2.0");
    assert_eq!(
        json["meshes"].as_array().unwrap().len(),
        1,
        "one shared mesh"
    );
    // Root (Z-up to Y-up) → "Bench" group → the two named instances.
    let nodes = json["nodes"].as_array().unwrap();
    let root_node = &nodes[json["scenes"][0]["nodes"][0].as_u64().unwrap() as usize];
    assert_eq!(root_node["extras"]["units"], "meter");
    let bench = &nodes[root_node["children"][0].as_u64().unwrap() as usize];
    assert_eq!(bench["name"], "Bench");
    let leaves: Vec<_> = bench["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &nodes[c.as_u64().unwrap() as usize])
        .collect();
    assert_eq!(leaves.len(), 2);
    assert!(leaves.iter().all(|l| l["mesh"] == 0));
    // Mesh positions are the local definition in meters.
    let accessor = &json["accessors"][json["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
        .as_u64()
        .unwrap() as usize];
    let max: Vec<f64> = accessor["max"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    for (m, e) in max.iter().zip([0.010, 0.020, 0.030]) {
        near(*m, e, 1e-7);
    }
    // Placed exactly once: world bounds match the 3MF and STEP placements.
    let rotated = drawn_by(&glb, "Panel Rotated copy");
    let moved = drawn_by(&glb, "Panel Moved copy");
    assert_bounds(bounds(&rotated), [[30., 5., 2.], [50., 15., 32.]], 1e-4);
    assert_bounds(bounds(&moved), [[-20., 40., 0.], [-10., 60., 30.]], 1e-4);
    near(volume(&rotated), 6000., 1e-2);
    near(volume(&moved), 6000., 1e-2);
    // Basic appearance: the default appearance as linear PBR factors.
    let material = &json["materials"][0]["pbrMetallicRoughness"];
    let linear = |c: f64| ((c / 255. + 0.055) / 1.055f64).powf(2.4);
    near(
        material["baseColorFactor"][0].as_f64().unwrap(),
        linear(137.),
        1e-9,
    );
    near(material["roughnessFactor"].as_f64().unwrap(), 0.6, 1e-12);
}

#[test]
fn project_and_tag_scopes_keep_glb_membership_and_part_appearances() {
    let mut f = Fixture::new();
    let steel = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [200, 40, 40, 255],
            section_hatch: None,
        },
    );
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Add steel".into(),
            edits: vec![Mutation::Insert(steel.clone())],
        })
        .unwrap();
    let assigned = materials::prepare_assignment(
        &f.session,
        f.session.revision(),
        actor(),
        &[f.pin.body],
        &materials::Assignment {
            material: Some(steel.id),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )
    .unwrap();
    f.session.commit(assigned).unwrap();
    let glb_for = |f: &mut Fixture, tags: Option<TagQuery>| {
        f.define(SetDefinition {
            scope: Scope {
                root: ScopeRoot::Project,
                tags,
            },
            recipes: vec![mesh(
                MeshFormat::Glb,
                LengthUnit::Meter,
                HierarchyMode::PreserveAssembly,
            )],
            ..Default::default()
        });
        let preview = f.preview();
        assert!(preview.can_run(), "{:?}", preview.recipes);
        let root = tempfile::tempdir().unwrap();
        let receipt = f.run(&preview, root.path()).unwrap();
        parse_glb(&written(&receipt, ".glb")[0])
    };
    // Whole project: panel, pin and the two assembly copies, each once.
    let all = glb_for(&mut f, None);
    let names: std::collections::BTreeSet<_> = all.drawn.iter().map(|(n, ..)| n.clone()).collect();
    assert_eq!(names.len(), 4, "{names:?}");
    let total: f64 = names.iter().map(|n| volume(&drawn_by(&all, n))).sum();
    near(total, 3. * 6000. + PI * 25. * 10., 2.);
    // Two appearances: the default and steel red.
    let materials = all.json["materials"].as_array().unwrap();
    assert_eq!(materials.len(), 2);
    let red = ((200. / 255. + 0.055) / 1.055f64).powf(2.4);
    assert!(materials.iter().any(|m| {
        (m["pbrMetallicRoughness"]["baseColorFactor"][0]
            .as_f64()
            .unwrap()
            - red)
            .abs()
            < 1e-9
    }));
    // The cnc tag selects the panel and its copies, not the pin.
    let tagged = glb_for(
        &mut f,
        Some(TagQuery {
            all: ["cnc".into()].into(),
            ..Default::default()
        }),
    );
    // The placed Part is written in world; its two copies share one mesh.
    assert_eq!(tagged.json["meshes"].as_array().unwrap().len(), 2);
    let drawn_nodes = tagged.json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n.get("mesh").is_some())
        .count();
    assert_eq!(drawn_nodes, 3);
    let total: f64 = tagged
        .drawn
        .iter()
        .map(|(n, ..)| n.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .iter()
        .map(|n| volume(&drawn_by(&tagged, n)))
        .sum();
    near(total, 3. * 6000., 1e-1);
    assert!(drawn_by(&tagged, "Pin").is_empty());
}
