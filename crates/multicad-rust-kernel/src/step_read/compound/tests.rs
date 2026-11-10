use super::*;
use crate::{
    Rational,
    helical_tube_cut::{Cut, Spec},
    stadium::Stadium,
    step::{MAX_COMPOUND_SOLIDS, Writer, write_box},
};
use std::fmt::Write;

const TOLERANCE: f64 = 1e-7;
const U: Point = [1., 0., 0.];
const V: Point = [0., 1., 0.];

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn box_pair() -> String {
    let mut writer = Writer::new();
    writer.append_box([n(4); 3], [0.; 3], U, V).unwrap();
    writer.append_box([n(4); 3], [0.; 3], U, V).unwrap();
    writer.finish().unwrap()
}

fn records<'a>(text: &'a str, kind: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
    text.lines().filter_map(move |line| {
        line.split_once('=')
            .filter(|(_, args)| args.starts_with(kind))
    })
}

fn append(text: &str, records: &str) -> String {
    text.replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        &format!("{records}\nENDSEC;\nEND-ISO-10303-21;"),
    )
}

#[test]
fn compound_preserves_member_order_volume_and_overlap_as_independent_solids() {
    let mut writer = Writer::new();
    let profile = Stadium::new([[0.; 2], [4., 0.]], 1.).unwrap();
    writer.append_box([n(5); 3], [0.; 3], U, V).unwrap();
    writer
        .append_stadium_prism(&profile, 3., [2., 2., 1.], U, V, false)
        .unwrap();
    let text = writer.finish().unwrap();
    let decoded = read_import_compound(&text, TOLERANCE).unwrap();
    let [first, second] = decoded.as_slice() else {
        panic!("expected two independent STEP members");
    };
    assert_eq!(first.0.bounds_mm, [[0.; 3], [5.; 3]]);
    assert_eq!(first.0.volume_mm3, 125.);
    assert_eq!(second.0.bounds_mm, [[1., 1., 1.], [7., 3., 4.]]);
    assert!((second.0.volume_mm3 - 3. * profile.area_mm2()).abs() < TOLERANCE);
    assert!(matches!(first.1.profile, ImportProfile::Polygon { .. }));
    assert!(matches!(second.1.profile, ImportProfile::Stadium { .. }));
    assert!(decoded.iter().all(|(shape, _)| shape.solid_count == 1));
    assert_eq!(
        read_analytic_compound(&text, TOLERANCE).unwrap(),
        [first.0.clone(), second.0.clone()]
    );
    assert_eq!(
        read_planar_solid(&text, TOLERANCE),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        read_analytic_solid(&text, TOLERANCE),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        read_import_solid(&text, TOLERANCE),
        Err(ReadError::Unsupported)
    );

    let ids: Vec<_> = records(&text, "MANIFOLD_SOLID_BREP(")
        .map(|(id, _)| id)
        .collect();
    let reordered = text.replace(&ids.join(","), &[ids[1], ids[0]].join(","));
    assert_eq!(
        read_analytic_compound(&reordered, TOLERANCE).unwrap(),
        [second.0.clone(), first.0.clone()]
    );

    let single = write_box([n(5); 3], [0.; 3], U, V).unwrap();
    assert_eq!(
        read_import_compound(&single, TOLERANCE).unwrap(),
        vec![read_import_solid(&single, TOLERANCE).unwrap()]
    );
}

#[test]
fn compound_preserves_the_verified_cut_receipt_and_other_analytic_members() {
    let cut = Cut::new(
        Spec {
            stock_radius_mm: 8.,
            stock_height_mm: 30.,
            tube_radius_mm: 0.6,
            pitch_mm: 2.,
            path_height_mm: 15.,
            start_height_mm: 16.2,
            start_angle_rad: 0.1,
        },
        TOLERANCE,
    )
    .unwrap();
    let origin = [4., 5., 6.];
    let v = [0., 0., 1.];
    let mut writer = Writer::new();
    writer
        .append_helical_tube_cut(cut, origin, U, v, TOLERANCE)
        .unwrap();
    writer.append_box([n(4); 3], [30., 0., 0.], U, V).unwrap();
    let source = writer.finish().unwrap();
    let receipts = read_analytic_compound_with_helical_cuts(&source, TOLERANCE).unwrap();
    let [(cut_proof, Some(import)), (box_proof, None)] = receipts.as_slice() else {
        panic!("expected a verified helical cut followed by ordinary box readback");
    };
    assert!((cut_proof.volume_mm3 - cut.volume_mm3()).abs() < TOLERANCE);
    assert_eq!(box_proof.volume_mm3, 64.);
    assert_eq!([import.origin_mm, import.u, import.v], [origin, U, v]);
    assert!(matches!(
        import.profile,
        ImportProfile::HelicalTubeCut { .. }
    ));
    assert_eq!(
        read_analytic_compound(&source, TOLERANCE).unwrap(),
        [cut_proof.clone(), box_proof.clone()]
    );

    let solids: Vec<_> = records(&source, "MANIFOLD_SOLID_BREP(").collect();
    let shells: Vec<_> = records(&source, "CLOSED_SHELL(")
        .map(|(id, _)| id)
        .collect();
    let ids = [solids[0].0, solids[1].0].join(",");
    let reordered = source.replace(&ids, &[solids[1].0, solids[0].0].join(","));
    assert_eq!(
        read_analytic_compound_with_helical_cuts(&reordered, TOLERANCE).unwrap(),
        [receipts[1].clone(), receipts[0].clone()]
    );
    let repeated = source.replace(&ids, &[solids[0].0; 2].join(","));
    let shared = source.replace(solids[1].1, &solids[1].1.replace(shells[1], shells[0]));
    let orphan = append(&source, "#999999=VERTEX_POINT('',#1);");
    for invalid in [repeated, shared, orphan] {
        assert_eq!(
            read_analytic_compound_with_helical_cuts(&invalid, TOLERANCE),
            Err(ReadError::Invalid)
        );
    }
}

#[test]
fn rejects_duplicate_or_missing_members_shared_shells_and_orphan_topology() {
    let text = box_pair();
    let solids: Vec<_> = records(&text, "MANIFOLD_SOLID_BREP(").collect();
    let shells: Vec<_> = records(&text, "CLOSED_SHELL(").map(|(id, _)| id).collect();
    let ids = [solids[0].0, solids[1].0].join(",");
    let duplicate = text.replace(&ids, &[solids[0].0; 2].join(","));
    let missing = text.replace(&ids, solids[0].0);
    let shared = text.replace(solids[1].1, &solids[1].1.replace(shells[1], shells[0]));
    let orphan = append(&text, "#999999=VERTEX_POINT('',#1);");
    let inward_member = records(&text, "ADVANCED_FACE(").nth(6).unwrap();
    let inward = text.replace(
        &format!("{}={}", inward_member.0, inward_member.1),
        &format!(
            "{}={}",
            inward_member.0,
            inward_member.1.replace(".T.", ".F.")
        ),
    );
    for invalid in [duplicate, missing, shared, orphan, inward] {
        assert_eq!(
            read_import_compound(&invalid, TOLERANCE),
            Err(ReadError::Invalid)
        );
    }
    let nested = append(
        &text,
        "#999999=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#1,#2);",
    );
    assert_eq!(
        read_analytic_compound(&nested, TOLERANCE),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn coincident_members_can_share_geometric_points_without_sharing_vertex_records() {
    let text = box_pair();
    let vertices: Vec<_> = records(&text, "VERTEX_POINT(").collect();
    let (first, a) = vertices[0];
    let (second, b) = vertices[8];
    let shared_support = text.replace(&format!("{second}={b}"), &format!("{second}={a}"));
    assert_eq!(
        read_analytic_compound(&shared_support, TOLERANCE),
        read_analytic_compound(&text, TOLERANCE)
    );

    let mut shared_vertex = text.clone();
    for (id, edge) in records(&text, "EDGE_CURVE(") {
        if edge.contains(&format!(",{second},")) {
            let cross_member = edge.replace(&format!(",{second},"), &format!(",{first},"));
            shared_vertex =
                shared_vertex.replace(&format!("{id}={edge}"), &format!("{id}={cross_member}"));
        }
    }
    assert_eq!(
        read_analytic_compound(&shared_vertex, TOLERANCE),
        Err(ReadError::Invalid)
    );
}

#[test]
fn rejects_empty_and_excessive_representations_before_reading_their_shells() {
    let text = write_box([n(4); 3], [0.; 3], U, V).unwrap();
    let (solid, _) = records(&text, "MANIFOLD_SOLID_BREP(").next().unwrap();
    let (shell, _) = records(&text, "CLOSED_SHELL(").next().unwrap();
    let (_, representation) = records(&text, "ADVANCED_BREP_SHAPE_REPRESENTATION(")
        .next()
        .unwrap();
    let empty = text.replace(
        representation,
        &representation.replace(&format!(",{solid}"), ""),
    );
    assert_eq!(
        read_analytic_compound(&empty, TOLERANCE),
        Err(ReadError::Unsupported)
    );

    let mut extra_ids = solid.to_owned();
    let mut extra_entities = String::new();
    for offset in 0..MAX_COMPOUND_SOLIDS {
        let id = 900_000 + offset;
        write!(extra_ids, ",#{id}").unwrap();
        writeln!(extra_entities, "#{id}=MANIFOLD_SOLID_BREP('',{shell});").unwrap();
    }
    let expanded = text.replace(representation, &representation.replace(solid, &extra_ids));
    assert_eq!(
        read_analytic_compound(&append(&expanded, &extra_entities), TOLERANCE),
        Err(ReadError::Limit)
    );
}

#[test]
fn complex_geometry_may_not_hide_topology_in_the_ownership_walk() {
    let text = write_box([n(4); 3], [0.; 3], U, V).unwrap();
    let mut entities = parse_entities(&text, TOLERANCE).unwrap();
    let initial = Shell::new(&entities, TOLERANCE, true);
    let solid = initial.represented_solids(true).unwrap()[0];
    let face = initial.member_faces(solid).unwrap()[0];
    let support = reference(&initial.args(face, "ADVANCED_FACE").unwrap()[2]).unwrap();
    entities.get_mut(&support).unwrap().push(Entity {
        name: "EDGE_CURVE",
        args: vec![],
    });
    let shell = Shell::new(&entities, TOLERANCE, true);
    assert_eq!(
        claim_topology(&shell, solid, &mut BTreeMap::new()),
        Err(ReadError::Unsupported),
    );
}
