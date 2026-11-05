//! Parse the stored XML, without the production mesh reader, and test topology
//! using vertex links and directed indexed edges. Coordinate coincidence alone
//! cannot establish a printable indexed shell.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::{AidPolicy, LengthUnit, MeshQuality, PrintRequest, PrintSource},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
};
use multicad_kernel::{GeneratedZaWormWheel, Solid, ZaWorm};
use multicad_model::export::{
    Exporter,
    mesh::{self, BuildItem, MeshObject},
};
use quick_xml::{Reader, events::Event};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Parsed {
    objects: Vec<MeshObject>,
    items: Vec<(usize, [f64; 12])>,
}
fn parse(bytes: &[u8]) -> Parsed {
    // Only stored ZIP records are accepted. This walks local file records and
    // checks CRC, rather than using the production ZIP or 3MF readers.
    let u16le = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap()) as usize;
    let u32le = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let mut at = 0;
    let mut models = vec![];
    while u32le(at) == 0x0403_4b50 {
        assert_eq!(u16le(at + 6), 0);
        assert_eq!(u16le(at + 8), 0);
        let n = u16le(at + 26);
        let extra = u16le(at + 28);
        let size = u32le(at + 18);
        assert_eq!(size, u32le(at + 22));
        let name = std::str::from_utf8(&bytes[at + 30..at + 30 + n]).unwrap();
        let begin = at + 30 + n + extra;
        let payload = &bytes[begin..begin + size];
        assert_eq!(crc32fast::hash(payload) as usize, u32le(at + 14));
        if name == "3D/3dmodel.model" {
            models.push(payload);
        }
        at = begin + size;
    }
    assert_eq!(models.len(), 1);
    assert_eq!(u32le(at), 0x0201_4b50);
    let mut reader = Reader::from_reader(models[0]);
    let mut out = Parsed::default();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                let attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            String::from_utf8(a.key.0.to_vec()).unwrap(),
                            a.normalized_value(quick_xml::XmlVersion::Explicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let f = |name: &str| attrs[name].parse::<f64>().unwrap();
                match e.name().as_ref() {
                    b"model" => assert_eq!(attrs["unit"], "millimeter"),
                    b"object" => {
                        assert_eq!(attrs["id"].parse::<usize>().unwrap(), out.objects.len() + 1);
                        out.objects.push(MeshObject {
                            name: attrs["name"].clone(),
                            vertices: vec![],
                            triangles: vec![],
                        });
                    }
                    b"vertex" => {
                        out.objects
                            .last_mut()
                            .unwrap()
                            .vertices
                            .push([f("x"), f("y"), f("z")])
                    }
                    b"triangle" => out
                        .objects
                        .last_mut()
                        .unwrap()
                        .triangles
                        .push(["v1", "v2", "v3"].map(|k| attrs[k].parse().unwrap())),
                    b"item" => {
                        let m = attrs["transform"]
                            .split_whitespace()
                            .map(|v| v.parse().unwrap())
                            .collect::<Vec<_>>();
                        out.items.push((
                            attrs["objectid"].parse::<usize>().unwrap() - 1,
                            m.try_into().unwrap(),
                        ));
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert!(
        out.objects
            .iter()
            .flat_map(|m| m.vertices.iter().flatten())
            .all(|v| v.is_finite())
    );
    out
}
fn manifold(m: &MeshObject) -> usize {
    let mut edges = BTreeMap::<(u32, u32), Vec<(usize, bool)>>::new();
    let mut links = vec![BTreeMap::<u32, BTreeSet<u32>>::new(); m.vertices.len()];
    for (face, t) in m.triangles.iter().enumerate() {
        assert!(t.iter().all(|&i| (i as usize) < m.vertices.len()));
        assert_eq!(t.iter().collect::<BTreeSet<_>>().len(), 3);
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            edges
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((face, a < b));
            let (center, a, b) = (t[i] as usize, t[(i + 1) % 3], t[(i + 2) % 3]);
            links[center].entry(a).or_default().insert(b);
            links[center].entry(b).or_default().insert(a);
        }
    }
    let mut adjacency = vec![BTreeSet::new(); m.triangles.len()];
    for incidence in edges.values() {
        assert_eq!(incidence.len(), 2, "open or nonmanifold indexed edge");
        assert_ne!(
            incidence[0].1, incidence[1].1,
            "same-direction indexed seam"
        );
        adjacency[incidence[0].0].insert(incidence[1].0);
        adjacency[incidence[1].0].insert(incidence[0].0);
    }
    for link in links {
        assert!(!link.is_empty(), "unused vertex");
        assert!(
            link.values().all(|n| n.len() == 2),
            "vertex link must be a cycle"
        );
        let mut todo = vec![*link.keys().next().unwrap()];
        let mut seen = BTreeSet::new();
        while let Some(v) = todo.pop() {
            if seen.insert(v) {
                todo.extend(link[&v].iter().copied());
            }
        }
        assert_eq!(
            seen.len(),
            link.len(),
            "coincident distinct vertex fans were joined"
        );
    }
    let mut seen = BTreeSet::new();
    let mut components = 0;
    for first in 0..adjacency.len() {
        if seen.contains(&first) {
            continue;
        }
        components += 1;
        let mut todo = vec![first];
        while let Some(f) = todo.pop() {
            if seen.insert(f) {
                todo.extend(adjacency[f].iter().copied());
            }
        }
    }
    components
}
type Point = [u64; 3];
fn point(p: [f64; 3]) -> Point {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}
fn facets(m: &MeshObject) -> BTreeMap<[Point; 3], usize> {
    let mut out = BTreeMap::new();
    for t in &m.triangles {
        let p = t.map(|i| point(m.vertices[i as usize]));
        let key = (0..3)
            .map(|i| [p[i], p[(i + 1) % 3], p[(i + 2) % 3]])
            .min()
            .unwrap();
        *out.entry(key).or_default() += 1;
    }
    out
}
fn assert_preserved(source: &MeshObject, stored: &MeshObject) {
    assert_eq!(source.triangles.len(), stored.triangles.len());
    assert_eq!(
        facets(source),
        facets(stored),
        "positions, winding and multiplicity must remain exact"
    );
}
fn write(objects: &[MeshObject], items: &[BuildItem]) -> Vec<u8> {
    mesh::three_mf(objects, items, LengthUnit::Millimeter).unwrap()
}
#[test]
fn native_faceted_box_exports_an_indexed_shell_and_exact_world_build_transforms() {
    let native = Solid::box_solid([10., 20., 30.], Default::default()).unwrap();
    let source = mesh::tessellate(&native, &MeshQuality::default(), "Native faceted box").unwrap();
    assert!(
        source.vertices.len() > 8,
        "fixture must contain native per-face vertex copies"
    );
    let pose = BuildItem {
        object: 0,
        rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
        translation_mm: [7., -3., 11.],
    };
    let bytes = write(
        std::slice::from_ref(&source),
        &[BuildItem::identity(0), pose.clone()],
    );
    let p = parse(&bytes);
    assert_eq!(p.objects.len(), 1);
    assert_eq!(p.items.len(), 2);
    let stored = &p.objects[0];
    assert_eq!(stored.vertices.len(), 8);
    assert_eq!(manifold(stored), 1);
    assert_preserved(&source, stored);
    for (i, (object, m)) in p.items.iter().enumerate() {
        assert_eq!(*object, 0);
        let expected = if i == 0 {
            BuildItem::identity(0)
        } else {
            pose.clone()
        };
        for &v in &stored.vertices {
            let actual =
                std::array::from_fn(|j| v[0] * m[j] + v[1] * m[j + 3] + v[2] * m[j + 6] + m[j + 9]);
            let wanted = std::array::from_fn(|j| {
                (0..3).map(|k| expected.rotation[j][k] * v[k]).sum::<f64>()
                    + expected.translation_mm[j]
            });
            assert_eq!(point(actual), point(wanted));
        }
    }
}
#[test]
fn canonical_print_export_has_real_closed_indices_without_mutating_source() {
    let mut s = Session::new(Document::new("Indexed print package").unwrap()).unwrap();
    let actor = Actor::local(Channel::Ui);
    let request = PrimitiveRequest::new(
        "Print block",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, 0, actor.clone(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    let before = s.document().clone();
    let print = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![request.body],
        },
        aids: AidPolicy::ExcludeRemovable,
        quality: MeshQuality::default(),
        stl_per_part: false,
    };
    let exporter = Exporter::default();
    let preview = exporter
        .print_preview(s.document(), s.revision(), &print, &actor, None)
        .unwrap();
    assert!(preview.can_write());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("block.3mf");
    let receipt = exporter
        .write_print(
            s.document(),
            s.revision(),
            &print,
            &preview.key,
            &path,
            &actor,
            None,
        )
        .unwrap();
    assert_eq!(receipt.files.len(), 1);
    let p = parse(&std::fs::read(path).unwrap());
    assert_eq!(p.objects.len(), 1);
    assert_eq!(p.items.len(), 1);
    assert_eq!(manifold(&p.objects[0]), 1);
    assert_eq!(p.objects[0].vertices.len(), 8);
    assert_eq!(p.objects[0].triangles.len(), 12);
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), 1);
}
#[test]
fn actual_za_worm_and_generated_wheel_keep_all_native_triangles_and_indexed_closure() {
    let policy = Default::default();
    let worm = Solid::za_worm_named(
        ZaWorm {
            axial_module_mm: 1.,
            threaded_length_mm: 16.,
            bore_radius_mm: 1.,
            axial_thinning_mm: 0.05,
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let wheel = Solid::generated_za_worm_wheel_named(
        GeneratedZaWormWheel {
            teeth: 32,
            axial_module_mm: 1.,
            face_width_mm: 1.,
            bore_radius_mm: 2.,
            pitch_thinning_mm: 0.05,
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let source = [
        mesh::tessellate(&worm, &MeshQuality::default(), "ZA worm").unwrap(),
        mesh::tessellate(&wheel, &MeshQuality::default(), "Generated ZA wheel").unwrap(),
    ];
    let p = parse(&write(
        &source,
        &[BuildItem::identity(0), BuildItem::identity(1)],
    ));
    assert_eq!(p.objects.len(), 2);
    assert_eq!(p.items.len(), 2);
    for (source, stored) in source.iter().zip(&p.objects) {
        assert_eq!(manifold(stored), 1);
        assert_preserved(source, stored);
        assert!(stored.triangles.len() > 1000);
    }
}
fn tetra(vertices: [[f64; 3]; 4]) -> MeshObject {
    MeshObject {
        name: "Closed tetrahedron".into(),
        vertices: vertices.to_vec(),
        triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
    }
}
fn append(a: &mut MeshObject, b: MeshObject) {
    let offset = a.vertices.len() as u32;
    a.vertices.extend(b.vertices);
    a.triangles
        .extend(b.triangles.into_iter().map(|t| t.map(|i| i + offset)));
}
#[test]
fn coincident_point_keeps_distinct_vertex_fans_and_ambiguous_shared_edge_is_refused() {
    let a = tetra([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
    // Two closed indexed components can occupy identical coordinates. This
    // valid duplicate-vertex exception must not collapse their multiplicity.
    let mut coincident = a.clone();
    append(&mut coincident, a.clone());
    let duplicate = parse(&write(
        std::slice::from_ref(&coincident),
        &[BuildItem::identity(0)],
    ));
    assert_eq!(manifold(&duplicate.objects[0]), 2);
    assert_eq!(duplicate.objects[0].vertices.len(), 8);
    assert_preserved(&coincident, &duplicate.objects[0]);
    let mut point_touch = a.clone();
    append(
        &mut point_touch,
        tetra([[0., 0., 0.], [0., -1., 0.], [-1., 0., 0.], [0., 0., -1.]]),
    );
    let p = parse(&write(
        std::slice::from_ref(&point_touch),
        &[BuildItem::identity(0)],
    ));
    assert_eq!(manifold(&p.objects[0]), 2);
    assert_preserved(&point_touch, &p.objects[0]);
    assert_eq!(p.objects[0].vertices.len(), 8);
    let mut edge_touch = a;
    append(
        &mut edge_touch,
        tetra([[0., 0., 0.], [1., 0., 0.], [0., -1., 0.], [0., 0., -1.]]),
    );
    // Discard original face connectivity, as a face-fragmented native mesh
    // does. Four uses of the coincident edge cannot be paired unambiguously.
    let mut fragmented = MeshObject {
        name: "Ambiguous fragmented edge".into(),
        vertices: vec![],
        triangles: vec![],
    };
    for triangle in &edge_touch.triangles {
        let first = fragmented.vertices.len() as u32;
        fragmented
            .vertices
            .extend(triangle.map(|v| edge_touch.vertices[v as usize]));
        fragmented.triangles.push([first, first + 1, first + 2]);
    }
    assert!(
        mesh::three_mf(
            &[fragmented],
            &[BuildItem::identity(0)],
            LengthUnit::Millimeter
        )
        .is_err()
    );
}
