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
fn shifted_gear_with_intrinsic_hub_and_dp_source_exports_one_closed_bored_print_object() {
    use multicad_core::constraint_sketch::Dimension;
    use multicad_core::model::{
        GearFamily, GearHand, GearHub, GearParameters, GearPitchInput, GearRequest, angle, ratio,
    };
    for (family, module_system) in [
        (GearFamily::Spur, model::GearModuleSystem::Normal),
        (GearFamily::Helical, model::GearModuleSystem::Normal),
        (GearFamily::Herringbone, model::GearModuleSystem::Normal),
        (GearFamily::DoubleHelical, model::GearModuleSystem::Normal),
        (GearFamily::Helical, model::GearModuleSystem::Transverse),
        (GearFamily::Herringbone, model::GearModuleSystem::Transverse),
        (
            GearFamily::DoubleHelical,
            model::GearModuleSystem::Transverse,
        ),
    ] {
        let mut s = Session::new(Document::new("Retained DP hub print").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let request = GearRequest::new(
            "Shifted DP hub",
            GearParameters {
                family,
                module_system,
                helix_angle: angle(if family == GearFamily::Spur { 0. } else { 20. }),
                hand: if family == GearFamily::Spur {
                    GearHand::None
                } else {
                    GearHand::Right
                },
                center_gap: length(if family == GearFamily::DoubleHelical {
                    0.5
                } else {
                    0.
                }),
                pitch_input: GearPitchInput::DiametralPitch,
                module: Dimension::Literal {
                    value: 25.4,
                    unit: "teeth_per_in".into(),
                },
                face_width: length(4.),
                bore_diameter: length(2.),
                backlash: length(0.02),
                root_fillet_ratio: ratio(0.2),
                profile_shift: ratio(0.2),
                minimum_wall: length(0.5),
                hub: Some(GearHub {
                    diameter: length(8.),
                    axial_start: length(-2.),
                    axial_end: length(6.),
                }),
                ..Default::default()
            },
        );
        let prepared = model::prepare_gear(&s, 0, actor.clone(), &request).unwrap();
        s.commit(prepared.prepared).unwrap();
        let before = s.document().clone();
        let print = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![request.body],
            },
            aids: AidPolicy::ExcludeRemovable,
            quality: MeshQuality::default(),
            stl_per_part: true,
        };
        let exporter = Exporter::default();
        let preview = exporter
            .print_preview(s.document(), 1, &print, &actor, None)
            .unwrap();
        assert!(preview.can_write());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gear.3mf");
        let receipt = exporter
            .write_print(s.document(), 1, &print, &preview.key, &path, &actor, None)
            .unwrap();
        assert_eq!(receipt.files.len(), 2);
        let parsed = parse(&std::fs::read(&path).unwrap());
        assert_eq!(parsed.objects.len(), 1);
        assert_eq!(parsed.items.len(), 1);
        let mesh = &parsed.objects[0];
        assert_eq!(manifold(mesh), 1);
        let vertices = mesh.vertices.len() as i64;
        let faces = mesh.triangles.len() as i64;
        assert_eq!(
            vertices - 3 * faces / 2 + faces,
            0,
            "A single through bore retains a toroidal shell"
        );
        let volume = mesh::volume(&mesh.vertices, &mesh.triangles);
        assert!(volume > 1000.);
        let min_z = mesh
            .vertices
            .iter()
            .map(|v| v[2])
            .fold(f64::INFINITY, f64::min);
        let max_z = mesh
            .vertices
            .iter()
            .map(|v| v[2])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((min_z + 2.).abs() < 1e-6);
        assert!((max_z - 6.).abs() < 1e-6);
        let stl = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|s| s == "stl"))
            .unwrap();
        let stl = mesh::read_stl(&std::fs::read(stl).unwrap()).unwrap();
        assert!((stl.volume_mm3() - volume).abs() < 0.005);
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 1);
    }
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
    let manufacturing = [
        mesh::tessellate_manufacturing(&worm, &MeshQuality::default(), "ZA worm").unwrap(),
        mesh::tessellate_manufacturing(&wheel, &MeshQuality::default(), "Generated ZA wheel")
            .unwrap(),
    ];
    let stored = parse(&write(
        &manufacturing,
        &[BuildItem::identity(0), BuildItem::identity(1)],
    ));
    for (source, stored) in source.iter().zip(&stored.objects) {
        assert_eq!(manifold(stored), 1);
        assert_preserved(source, stored);
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

#[test]
fn transformed_fused_plate_and_blind_cross_holes_use_proven_native_seams_without_moving_the_solid()
{
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform,
    };
    let policy = GeometryPolicy::default();
    // Independent native construction from the shipped front-panel dimensions:
    // a 4mm plate, its 16mm plug and two 8mm-deep transverse insert pilots.
    let plate = Solid::box_solid([43., 86., 4.], policy).unwrap();
    let plug = Solid::box_solid([39.7, 82.4, 16.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [3.3, 1.8, 4.],
            ..Default::default()
        })
        .unwrap();
    let mut body = plate
        .boolean(&plug, BooleanOperation::Union, policy)
        .unwrap()
        .unwrap();
    for y in [0.8, 76.2] {
        let hole = Solid::cylinder(
            PlaneFrame {
                origin: [23., y, 12.],
                u: [0., 0., 1.],
                v: [1., 0., 0.],
            },
            2.,
            9.,
            policy,
        )
        .unwrap();
        body = body
            .boolean(&hole, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
    }
    let pose = RigidTransform {
        rotation: [[0.36, -0.8, 0.48], [0.48, 0.6, 0.64], [-0.8, 0., 0.6]],
        translation_mm: [19., -23., 41.],
    };
    let body = body.transformed(pose).unwrap();
    assert_eq!(body.solid_count().unwrap(), 1);
    let owner = EntityId::new();
    let before = body.topology_snapshot(owner, 9).unwrap();
    let bounds = body.bounds().unwrap();
    let volume = body.volume_mm3().unwrap();
    let area: f64 = body.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    let analytic =
        43. * 86. * 4. + 39.7 * 82.4 * 16. - 2. * std::f64::consts::PI * 2_f64.powi(2) * 8.;
    assert!((volume - analytic).abs() <= policy.linear_tolerance_mm * area + 1e-7 * analytic);
    let quality = MeshQuality::default();
    let options = MeshOptions {
        linear_deflection_mm: quality.chord_deviation_mm,
        angular_deflection_rad: quality.angular_deviation_deg.to_radians(),
        maximum_triangles: 2_000_000,
    };
    let raw = mesh::tessellate(&body, &quality, "Raw face-seam fixture").unwrap();
    assert!(
        mesh::three_mf(
            std::slice::from_ref(&raw),
            &[BuildItem::identity(0)],
            LengthUnit::Millimeter
        )
        .is_err(),
        "fixture must reproduce the exact-coordinate-only writer refusal"
    );
    let native = body.manufacturing_mesh_with_receipt(options).unwrap();
    assert!(native.edge_pairs > 0);
    assert!(native.vertex_links > 0);
    assert!(
        native.maximum_displacement_mm > 0.,
        "fixture must contain non-exact native face seams"
    );
    assert!(
        native.maximum_displacement_mm
            <= policy
                .linear_tolerance_mm
                .min(quality.chord_deviation_mm / 16.)
    );
    let corrected =
        mesh::tessellate_manufacturing(&body, &quality, "Native seam-proven print").unwrap();
    let package = parse(&write(
        std::slice::from_ref(&corrected),
        &[BuildItem::identity(0)],
    ));
    let stored = &package.objects[0];
    assert_eq!(manifold(stored), 1);
    assert_preserved(&corrected, stored);
    assert_eq!(stored.triangles.len(), raw.triangles.len());
    // Geometry correction is bounded by the explicit native receipt, never a
    // spatial weld chosen by the file writer. Triangle order and winding stay.
    for (a, b) in raw.triangles.iter().zip(&corrected.triangles) {
        for (&a, &b) in a.iter().zip(b) {
            let pa = raw.vertices[a as usize];
            let pb = corrected.vertices[b as usize];
            let displacement = (0..3).map(|i| (pa[i] - pb[i]).powi(2)).sum::<f64>().sqrt();
            assert!(displacement <= native.maximum_displacement_mm + f64::EPSILON * 128.);
        }
    }
    let signed_volume: f64 = stored
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| stored.vertices[i as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.
        })
        .sum();
    assert!(signed_volume > 0.);
    assert!((signed_volume - volume).abs() <= quality.chord_deviation_mm * area + 1e-7 * volume);
    assert_eq!(body.topology_snapshot(owner, 9).unwrap(), before);
    assert_eq!(body.bounds().unwrap(), bounds);
    assert_eq!(body.volume_mm3().unwrap(), volume);
    let (object, transform) = package.items[0];
    assert_eq!(object, 0);
    assert_eq!(transform, [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.]);
}

#[test]
fn actual_positive_load_right_hand_buttress_omits_only_exact_duplicate_native_xyz_triangles() {
    use multicad_core::model::{ButtressLoadFlankSide, ThreadHand, ThreadRequest};
    use multicad_kernel::{GeometryPolicy, MeshOptions};
    let mut session =
        Session::new(Document::new("Buttress exact native triangle").unwrap()).unwrap();
    let mut parameters = model::buttress_thread_presets()
        .remove(0)
        .parameters(ButtressLoadFlankSide::PositiveAxisSide);
    parameters.hand = ThreadHand::Right;
    parameters.length = length(13.48);
    parameters.radial_allowance = length(0.005);
    let request = ThreadRequest::new("Positive-side right-hand buttress", parameters);
    let prepared =
        model::prepare_thread(&session, 0, Actor::local(Channel::Mcp), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let document = session.document().clone();
    let compiled = multicad_model::Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(session.document(), session.revision())
    .unwrap();
    let shape = compiled.solid(request.feature).unwrap();
    let native_before = shape.to_brep().unwrap();
    let topology = shape
        .topology_snapshot(request.feature, session.revision())
        .unwrap();
    let quality = MeshQuality {
        chord_deviation_mm: 0.003,
        angular_deviation_deg: 5.,
    };
    let options = MeshOptions {
        linear_deflection_mm: quality.chord_deviation_mm,
        angular_deflection_rad: quality.angular_deviation_deg.to_radians(),
        maximum_triangles: 2_000_000,
    };
    let raw = shape.mesh(options).unwrap();
    let vertices = raw.vertices.as_chunks::<3>().0;
    let original_faces = raw.face_indices.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(original_faces.len(), shape.faces().unwrap().len());
    let mut retained_faces = BTreeSet::new();
    let mut survivors = vec![];
    let mut removed = 0;
    for (triangle, face) in raw.indices.as_chunks::<3>().0.iter().zip(&raw.face_indices) {
        let p = triangle.map(|v| vertices[v as usize]);
        if p[0] == p[1] || p[1] == p[2] || p[2] == p[0] {
            removed += 1;
        } else {
            survivors.push(p);
            retained_faces.insert(*face);
        }
    }
    eprintln!(
        "buttress_positive_right_exact_xyz_triangles_removed={removed}; observed_native_faces={}",
        original_faces.len()
    );
    assert!(
        removed > 0,
        "fixture must reproduce actual exact duplicate XYZ, not a near-equal or small-area triangle"
    );
    assert_eq!(
        retained_faces, original_faces,
        "no native face may disappear"
    );
    let raw_object = mesh::tessellate(shape, &quality, "Unfiltered native buttress").unwrap();
    assert_eq!(raw_object.triangles.len(), raw.indices.len() / 3);
    assert!(
        mesh::three_mf(
            &[raw_object],
            &[BuildItem::identity(0)],
            LengthUnit::Millimeter
        )
        .is_err(),
        "the direct writer remains strict for arbitrary mesh input"
    );
    let manufacturing =
        mesh::tessellate_manufacturing(shape, &quality, "Manufacturing buttress").unwrap();
    let actual = manufacturing
        .triangles
        .iter()
        .map(|t| t.map(|v| manufacturing.vertices[v as usize]))
        .collect::<Vec<_>>();
    assert_eq!(
        actual, survivors,
        "all and only exact-duplicate triangles may be omitted; survivor positions/order/winding must be identical"
    );
    assert_eq!(
        manufacturing.triangles.len() + removed,
        raw.indices.len() / 3
    );
    let parsed = parse(&write(
        std::slice::from_ref(&manufacturing),
        &[BuildItem::identity(0)],
    ));
    assert_eq!(parsed.objects.len(), 1);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(manifold(&parsed.objects[0]), 1);
    assert_preserved(&manufacturing, &parsed.objects[0]);
    let stored = &parsed.objects[0];
    let signed: f64 = stored
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|v| stored.vertices[v as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.
        })
        .sum();
    assert!(signed > 0.);
    let volume = shape.volume_mm3().unwrap();
    let area: f64 = shape.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    assert!(
        (signed - volume).abs()
            <= quality.chord_deviation_mm * area
                + GeometryPolicy::default().linear_tolerance_mm * area
                + 1e-7 * volume
    );
    assert_eq!(
        shape.mesh(options).unwrap(),
        raw,
        "the ordinary native/display mesh must retain its original triangles"
    );
    assert_eq!(shape.to_brep().unwrap(), native_before);
    assert_eq!(
        shape
            .topology_snapshot(request.feature, session.revision())
            .unwrap(),
        topology
    );
    assert_eq!(session.document(), &document);
    assert_eq!(session.revision(), 1);
}

#[test]
fn actual_positive_load_left_hand_buttress_retriangulates_detached_internal_constraints_only() {
    use multicad_core::model::{ButtressLoadFlankSide, ThreadHand, ThreadRequest};
    let mut session =
        Session::new(Document::new("Positive left buttress manufacturing").unwrap()).unwrap();
    let mut parameters = model::buttress_thread_presets()
        .remove(0)
        .parameters(ButtressLoadFlankSide::PositiveAxisSide);
    parameters.hand = ThreadHand::Left;
    parameters.length = length(13.48);
    parameters.radial_allowance = length(0.005);
    let request = ThreadRequest::new("Positive-side left-hand buttress", parameters);
    let prepared =
        model::prepare_thread(&session, 0, Actor::local(Channel::Mcp), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let compiled = multicad_model::Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(session.document(), session.revision())
    .unwrap();
    let shape = compiled.solid(request.feature).unwrap();
    let quality = MeshQuality {
        chord_deviation_mm: 0.003,
        angular_deviation_deg: 5.,
    };
    let options = multicad_kernel::MeshOptions {
        linear_deflection_mm: quality.chord_deviation_mm,
        angular_deflection_rad: quality.angular_deviation_deg.to_radians(),
        maximum_triangles: 2_000_000,
    };
    let raw = shape.mesh(options).unwrap();
    let source = session.document().clone();
    let before = shape.to_brep().unwrap();
    let topology = shape
        .topology_snapshot(request.feature, session.revision())
        .unwrap();
    let bounds = shape.bounds().unwrap();
    let volume = shape.volume_mm3().unwrap();
    let raw_object = mesh::tessellate(shape, &quality, "Raw left buttress").unwrap();
    assert!(
        mesh::three_mf(
            &[raw_object],
            &[BuildItem::identity(0)],
            LengthUnit::Millimeter
        )
        .is_err(),
        "original raw mesh must reproduce indexed closure refusal"
    );
    let native = shape.manufacturing_mesh_with_receipt(options).unwrap();
    assert!(
        shape.to_brep().unwrap() == before,
        "direct manufacturing tessellation must preserve source BRep bytes"
    );
    eprintln!(
        "buttress_positive_left_removed_meshing_internal_edges={}; raw_triangles={}; corrected_triangles={}; maximum_displacement_mm={}",
        native.removed_meshing_internal_edges,
        raw.indices.len() / 3,
        native.mesh.indices.len() / 3,
        native.maximum_displacement_mm
    );
    assert!(native.remeshed_without_internal_constraints);
    assert!(native.removed_meshing_internal_edges > 0);
    assert!(native.removed_meshing_constraint_wire_containers > 0);
    assert_eq!(
        native
            .mesh
            .face_indices
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        raw.face_indices.iter().copied().collect::<BTreeSet<_>>(),
        "every original face must remain represented"
    );
    let tolerance = multicad_kernel::GeometryPolicy::default()
        .linear_tolerance_mm
        .min(quality.chord_deviation_mm / 16.);
    assert!(native.maximum_displacement_mm <= tolerance);
    let expected = native
        .mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            t.map(|v| {
                let i = 3 * v as usize;
                [
                    native.mesh.vertices[i],
                    native.mesh.vertices[i + 1],
                    native.mesh.vertices[i + 2],
                ]
            })
        })
        .collect::<Vec<_>>();
    let manufacturing =
        mesh::tessellate_manufacturing(shape, &quality, "Detached retriangulated buttress")
            .unwrap();
    assert!(
        shape.to_brep().unwrap() == before,
        "model manufacturing helper must preserve source BRep bytes"
    );
    let actual = manufacturing
        .triangles
        .iter()
        .map(|t| t.map(|v| manufacturing.vertices[v as usize]))
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "helper must preserve the actual native cleanup tessellation exactly, not invent facets across holes"
    );
    let package = parse(&write(
        std::slice::from_ref(&manufacturing),
        &[BuildItem::identity(0)],
    ));
    assert_eq!(package.objects.len(), 1);
    assert_eq!(package.items.len(), 1);
    let stored = &package.objects[0];
    assert_eq!(manifold(stored), 1);
    assert_preserved(&manufacturing, stored);
    let signed: f64 = stored
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|v| stored.vertices[v as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.
        })
        .sum();
    assert!(signed > 0.);
    let area: f64 = shape.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    assert!((signed - volume).abs() <= quality.chord_deviation_mm * area + 1e-7 * volume);
    for axis in 0..3 {
        let lo = stored
            .vertices
            .iter()
            .map(|p| p[axis])
            .reduce(f64::min)
            .unwrap();
        let hi = stored
            .vertices
            .iter()
            .map(|p| p[axis])
            .reduce(f64::max)
            .unwrap();
        assert!((lo - bounds.min[axis]).abs() <= quality.chord_deviation_mm + tolerance);
        assert!((hi - bounds.max[axis]).abs() <= quality.chord_deviation_mm + tolerance);
    }
    // The raw graph had actual holes. Compare the final mesh's material with
    // independent native Boolean cubes, including the threaded radial band.
    // Reparse the unchanged native BRep into independent OCCT topology.
    // Proper identity transforms intentionally share TShapes in production,
    // so they would not isolate Boolean bookkeeping flags for this oracle.
    let oracle = Solid::from_brep(&before, Default::default()).unwrap();
    assert!(
        shape.to_brep().unwrap() == before,
        "independent oracle creation must preserve source bytes"
    );
    for point in [
        [1., 0., 6.74],
        [12., 0., 6.74],
        [9., 0., 6.74],
        [0., 9., 6.74],
    ] {
        let side: f64 = 0.01;
        let cube = Solid::box_solid([side; 3], Default::default())
            .unwrap()
            .transformed(multicad_kernel::RigidTransform {
                translation_mm: point.map(|v| v - side / 2.),
                ..Default::default()
            })
            .unwrap();
        let fraction = oracle
            .boolean(
                &cube,
                multicad_kernel::BooleanOperation::Intersect,
                Default::default(),
            )
            .unwrap()
            .map_or(0., |s| s.volume_mm3().unwrap() / side.powi(3));
        assert!(
            fraction < 1e-4 || fraction > 1. - 1e-4,
            "material sample must lie away from a source boundary: {point:?}, {fraction}"
        );
        assert_eq!(
            mesh_contains(stored, point),
            fraction > 0.5,
            "native material mismatch at {point:?}"
        );
    }
    assert_eq!(shape.mesh(options).unwrap(), raw);
    assert!(
        shape.to_brep().unwrap() == before,
        "source BRep bytes must remain unchanged after independent Boolean oracles"
    );
    assert_eq!(
        shape
            .topology_snapshot(request.feature, session.revision())
            .unwrap(),
        topology
    );
    assert_eq!(shape.bounds().unwrap(), bounds);
    assert_eq!(shape.volume_mm3().unwrap(), volume);
    assert_eq!(session.document(), &source);
    assert_eq!(session.revision(), 1);
}
fn mesh_contains(mesh: &MeshObject, point: [f64; 3]) -> bool {
    let direction = [1., 0.17320508075688773, 0.113798013];
    let sub = |a: [f64; 3], b: [f64; 3]| std::array::from_fn(|i| a[i] - b[i]);
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let mut hits = vec![];
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|v| mesh.vertices[v as usize]);
        let e1 = sub(b, a);
        let e2 = sub(c, a);
        let p = cross(direction, e2);
        let determinant = dot(e1, p);
        if determinant.abs() < 1e-14 {
            continue;
        }
        let t = sub(point, a);
        let u = dot(t, p) / determinant;
        if !(0. ..=1.).contains(&u) {
            continue;
        }
        let q = cross(t, e1);
        let v = dot(direction, q) / determinant;
        if v < 0. || u + v > 1. {
            continue;
        }
        let distance = dot(e2, q) / determinant;
        if distance > 1e-10 {
            hits.push(distance);
        }
    }
    hits.sort_by(f64::total_cmp);
    hits.dedup_by(|a, b| (*a - *b).abs() < 1e-10);
    hits.len() % 2 == 1
}
