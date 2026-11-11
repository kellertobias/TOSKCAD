use std::collections::{BTreeMap, BTreeSet};
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
pub fn external_193_motif_input(source: &str) -> String {
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
