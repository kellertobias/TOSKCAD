//! Exercise the real external-file admission budget independently of unit-test builds.
use multicad_rust_kernel::{
    normal_relief::{Kind, Relief},
    normal_sweep::{Edge, Sweep},
    step::Writer,
    step_read::{ReadError, read_analytic_solid, read_import_solid},
};
use std::collections::{BTreeMap, BTreeSet};
const TOL: f64 = 1e-7;
fn fixture(centers: impl Iterator<Item = [f64; 2]>) -> Relief {
    let p = [[0., 0.], [100., 0.], [100., 80.], [0., 80.]];
    let base = Sweep::new(
        vec![
            (0..4)
                .map(|i| Edge::Line {
                    a: p[i],
                    b: p[(i + 1) % 4],
                })
                .collect(),
        ],
        4.,
        TOL,
        1e-5,
    )
    .unwrap();
    let motifs = centers
        .map(|center| {
            Sweep::new(
                vec![vec![Edge::Circle {
                    center,
                    radius_mm: 0.5,
                    clockwise: false,
                }]],
                0.5,
                TOL,
                1e-5,
            )
            .unwrap()
        })
        .collect();
    Relief::new(base, motifs, Kind::Raised, TOL, 1e-5).unwrap()
}
fn references(text: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut result = Vec::new();
    while i < bytes.len() {
        if bytes[i] == b'#' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            result.push(text[start..i].parse().unwrap());
        } else {
            i += 1;
        }
    }
    result
}
fn list(body: &str) -> Vec<u32> {
    references(
        body.split_once("('',(")
            .unwrap()
            .1
            .split_once(')')
            .unwrap()
            .0,
    )
}
fn topology(body: &str) -> bool {
    matches!(
        body.split_once('(').unwrap().0,
        "MANIFOLD_SOLID_BREP"
            | "CLOSED_SHELL"
            | "ADVANCED_FACE"
            | "FACE_OUTER_BOUND"
            | "FACE_BOUND"
            | "EDGE_LOOP"
            | "ORIENTED_EDGE"
            | "EDGE_CURVE"
            | "VERTEX_POINT"
    )
}
fn reachable(records: &BTreeMap<u32, String>, seeds: &[u32]) -> BTreeSet<u32> {
    let mut pending = seeds.to_vec();
    let mut found = BTreeSet::new();
    while let Some(id) = pending.pop() {
        let body = &records[&id];
        if topology(body) && found.insert(id) {
            pending.extend(references(body));
        }
    }
    found
}
fn ids(ids: &[u32]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",")
}
/// Join one extra motif's actual shared topology to the first stock cap.
/// Unused geometric supports may remain; unused or shared topology may not.
fn external_193_motif_input(source: &str) -> String {
    let mut records = source
        .lines()
        .filter_map(|line| {
            let (id, body) = line.strip_prefix('#')?.split_once('=')?;
            Some((id.parse::<u32>().unwrap(), body.to_owned()))
        })
        .collect::<BTreeMap<_, _>>();
    let solids = records
        .iter()
        .filter(|(_, body)| body.starts_with("MANIFOLD_SOLID_BREP("))
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    assert_eq!(solids.len(), 2);
    let shells = [solids[0], solids[1]].map(|id| references(&records[&id])[0]);
    let [first, second] = shells.map(|id| list(&records[&id]));
    assert_eq!(second.len(), 8);
    let mut bounds = list(&records[&first[1]]);
    let extra_bound = list(&records[&second[1]])[1];
    bounds.push(extra_bound);
    let suffix = records[&first[1]].split_once("),").unwrap().1.to_owned();
    records.insert(
        first[1],
        format!("ADVANCED_FACE('',({}),{suffix}", ids(&bounds)),
    );
    let mut combined = first;
    combined.extend_from_slice(&second[6..]);
    records.insert(shells[0], format!("CLOSED_SHELL('',({}));", ids(&combined)));
    let removed = reachable(&records, &[solids[1]]);
    let mut keep_seeds = second[6..].to_vec();
    keep_seeds.push(extra_bound);
    let keep = reachable(&records, &keep_seeds);
    for id in removed.difference(&keep) {
        records.remove(id);
    }
    for body in records.values_mut() {
        if body.starts_with("ADVANCED_BREP_SHAPE_REPRESENTATION(") {
            *body = body.replace(&format!(",#{}", solids[1]), "");
        }
    }
    let mut result = String::new();
    for line in source.lines() {
        if let Some((id, _)) = line.strip_prefix('#').and_then(|line| line.split_once('=')) {
            let id = id.parse::<u32>().unwrap();
            if let Some(body) = records.get(&id) {
                result.push_str(&format!("#{id}={body}\n"));
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}
#[test]
fn externally_valid_193_motif_shell_hits_typed_limit_and_public_reader_recovers() {
    let base = fixture((0..192).map(|i| [3. + (i % 16) as f64 * 6., 3. + (i / 16) as f64 * 6.]));
    let extra = fixture(std::iter::once([3., 75.]));
    let mut writer = Writer::new();
    for body in [&base, &extra] {
        writer
            .append_normal_relief(body, [0.; 3], [1., 0., 0.], [0., 1., 0.])
            .unwrap();
    }
    let source = writer.finish().unwrap();
    let external = external_193_motif_input(&source);
    let proof = read_analytic_solid(&external, TOL).unwrap();
    assert_eq!(proof.solid_count, 1);
    assert_eq!(proof.face_count, 392);
    assert!(matches!(
        read_import_solid(&external, TOL),
        Err(ReadError::Limit)
    ));
    let valid = multicad_rust_kernel::step::write_normal_relief(
        &extra,
        [0.; 3],
        [1., 0., 0.],
        [0., 1., 0.],
    )
    .unwrap();
    assert!(read_import_solid(&valid, TOL).is_ok());
}
