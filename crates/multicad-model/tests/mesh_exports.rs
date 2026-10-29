//! Real OBJ and 3MF output from exact native solids, read back by readers
//! that share no code with the writers: a local OBJ parser, a separate ZIP
//! reader with CRC checks, quick-xml for the 3MF model and, when present, the
//! system `unzip`.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    exchange::*,
    materials::{self, TagEdit, TagQuery},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    project_file::ContentHash,
};
use multicad_model::export::Exporter;
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
    for (recipe, reason) in [
        (
            mesh(
                MeshFormat::Glb,
                LengthUnit::Millimeter,
                HierarchyMode::SeparateParts,
            ),
            "CAD-131",
        ),
        (
            Recipe {
                output: RecipeOutput::Mesh {
                    format: MeshFormat::Obj,
                    units: LengthUnit::Millimeter,
                    hierarchy: HierarchyMode::SeparateParts,
                    names: NameHandling::FileNamesOnly,
                    aids: AidPolicy::ExcludeRemovable,
                    quality: MeshQuality::default(),
                },
                ..mesh(
                    MeshFormat::Obj,
                    LengthUnit::Millimeter,
                    HierarchyMode::SeparateParts,
                )
            },
            "Removable-aid",
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
