//! Cross-family coverage of the installed source projects. Family-specific
//! native tests independently check the actual solids and placement lifecycle.
use multicad_core::{
    document::{Document, EntityData},
    model::{
        self, MechanicalPartDefinition, MechanicalPartParameters, ResolvedMechanicalPart,
        ResolvedScrewDrive, ResolvedScrewHead, ScrewLengthConvention, ThreadFit,
        ThreadRepresentation, WoodPointIntent, WoodScrewRepresentation,
    },
    parameters,
    project_file::{self, ContentHash},
    symbols,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const REQUESTED: &[f64] = &[2., 2.5, 3., 3.5, 4., 4.5, 5., 6., 7., 8., 9., 10.];
const NORMAL: &[f64] = &[2., 2.5, 3., 3.5, 4., 5., 6., 7., 8., 10.];
const ISO_NINE: &[f64] = &[2., 2.5, 3., 3.5, 4., 5., 6., 8., 10.];
const SOCKET: &[f64] = &[2., 2.5, 3., 4., 5., 6., 8., 10.];
const EXTERNAL: &[f64] = &[4., 5., 6., 8., 10.];
const ISO_SIX: &[f64] = &[3., 4., 5., 6., 8., 10.];
const BOLTS: &str = "5c83300a57761f70db87f50c9a1f647c7278a4c2";

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}
fn json(root: &Path, name: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap()
}
fn text(value: &Value) -> &str {
    value.as_str().expect("nonempty textual source field")
}
fn number(value: &Value) -> f64 {
    value.as_f64().expect("numeric source field")
}
fn array(value: &Value) -> &[Value] {
    value.as_array().expect("source array")
}
fn id_list(value: &Value) -> Vec<&str> {
    if value.is_array() {
        array(value).iter().map(text).collect()
    } else {
        value.as_str().into_iter().collect()
    }
}
fn close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 1e-10,
        "{label}: {actual} != {expected}"
    );
}

/// Availability and IDs must agree with the reviewed size set, independently
/// of any supported/unavailable flags in the source manifests themselves.
fn coverage(
    rows: &[&Value],
    cells: &[&Value],
    available: &[f64],
    legacy_screw_family: Option<&str>,
    id_key: &str,
) {
    assert_eq!(cells.len(), REQUESTED.len());
    let mut seen = BTreeSet::new();
    for &nominal in REQUESTED {
        let matches: Vec<_> = cells
            .iter()
            .filter(|c| c["nominal_mm"].as_f64() == Some(nominal))
            .collect();
        let [cell] = matches.as_slice() else {
            panic!("expected exactly one coverage cell for {nominal}");
        };
        let cell = legacy_screw_family.map_or(**cell, |family| &cell[family]);
        let supported = available.contains(&nominal);
        assert_eq!(
            text(&cell["status"]),
            if supported {
                "supported"
            } else {
                "unavailable"
            }
        );
        if !supported {
            assert!(!text(&cell["reason"]).trim().is_empty());
        }
        let selected: BTreeSet<_> = rows
            .iter()
            .filter(|r| r["nominal_mm"].as_f64() == Some(nominal))
            .map(|r| text(&r["id"]))
            .collect();
        let ids = id_list(&cell[id_key]);
        assert_eq!(ids.len(), selected.len());
        assert_eq!(ids.iter().copied().collect::<BTreeSet<_>>(), selected);
        assert_eq!(!selected.is_empty(), supported);
        assert!(ids.into_iter().all(|id| seen.insert(id)));
    }
    assert_eq!(seen.len(), rows.len());
}

#[test]
fn requested_nominals_and_profile_combinations_have_explicit_coverage() {
    let root = library();
    let washers = json(&root, "washers-curation.json");
    let nuts = json(&root, "nuts-curation.json");
    let screws = json(&root, "machine-screws-curation.json");
    let wood = json(&root, "wood-screws-curation.json");
    coverage(
        &row_refs(&washers["entries"]),
        &row_refs(&washers["coverage"]),
        NORMAL,
        None,
        "entry_id",
    );
    for (family, nominal, count) in [("din934-1987", NORMAL, 10), ("iso4032-2012", ISO_NINE, 9)] {
        let rows = family_rows(&nuts["entries"], family);
        assert_eq!(rows.len(), count);
        coverage(
            &rows,
            &family_rows(&nuts["coverage"], family),
            nominal,
            None,
            "entry_id",
        );
    }
    assert_eq!(array(&nuts["coverage"]).len(), 24);
    for (family, nominal, count) in [("socket_cap", SOCKET, 14), ("external_hex", EXTERNAL, 5)] {
        let rows = family_rows(&screws["entries"], family);
        assert_eq!(rows.len(), count);
        coverage(
            &rows,
            &row_refs(&screws["coverage"]),
            nominal,
            Some(family),
            "entries",
        );
    }
    for (family, nominal) in [
        ("iso14583-2011", ISO_NINE),
        ("iso14581-2013", ISO_NINE),
        ("iso7045-2011-h", ISO_NINE),
        ("iso7046-1-2011-h", ISO_NINE),
        ("fila-iso14579", &[6., 8., 10.]),
        ("iso7380-1-2011", ISO_SIX),
        ("iso10642-2004", ISO_SIX),
        ("accu-shb-m3-10-a2-r360", &[3.]),
    ] {
        let rows = family_rows(&screws["profile_entries"], family);
        assert_eq!(rows.len(), nominal.len());
        coverage(
            &rows,
            &family_rows(&screws["profile_coverage"], family),
            nominal,
            None,
            "entry_id",
        );
    }
    assert_eq!(array(&screws["profile_coverage"]).len(), 96);
    coverage(
        &row_refs(&wood["entries"]),
        &family_rows(&wood["coverage"], "bitork-din571"),
        EXTERNAL,
        None,
        "entry_ids",
    );
    assert_eq!(array(&wood["coverage"]).len(), REQUESTED.len());
    let excluded = array(&wood["excluded_families"])
        .iter()
        .map(|r| {
            assert_eq!(r["status"], "unavailable");
            assert!(!text(&r["reason"]).trim().is_empty());
            text(&r["family"])
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        excluded,
        BTreeSet::from(["fischer-powerfast-ii-ctf-bc", "phillips-h-wood-screw"])
    );
}

fn row_refs(value: &Value) -> Vec<&Value> {
    array(value).iter().collect()
}

fn family_rows<'a>(rows: &'a Value, family: &str) -> Vec<&'a Value> {
    array(rows)
        .iter()
        .filter(|r| r["family"] == family)
        .collect()
}

fn hardware(document: &Document) -> MechanicalPartDefinition {
    let features: Vec<_> = document.entities().filter(|e| {
        matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == model::MECHANICAL_PART_TYPE)
    }).collect();
    let [feature] = features.as_slice() else {
        panic!("expected one mechanical source in {}", document.name());
    };
    model::read_mechanical_part(document, feature.id).unwrap()
}

fn screw_profiles(family: &str, head: &ResolvedScrewHead, drive: &ResolvedScrewDrive) {
    use ResolvedScrewDrive as D;
    use ResolvedScrewHead as H;
    assert!(
        match family {
            "socket_cap" => matches!((head, drive), (H::SocketCap { .. }, D::HexSocket { .. })),
            "external_hex" | "bitork-din571" =>
                matches!((head, drive), (H::ExternalHex { .. }, D::ExternalHex)),
            "iso14583-2011" => matches!((head, drive), (H::Pan { .. }, D::Hexalobular { .. })),
            "iso14581-2013" => matches!(
                (head, drive),
                (H::Countersunk { .. }, D::Hexalobular { .. })
            ),
            "iso7045-2011-h" => matches!((head, drive), (H::Pan { .. }, D::PhillipsH { .. })),
            "iso7046-1-2011-h" =>
                matches!((head, drive), (H::Countersunk { .. }, D::PhillipsH { .. })),
            "fila-iso14579" =>
                matches!((head, drive), (H::SocketCap { .. }, D::Hexalobular { .. })),
            "iso7380-1-2011" => matches!((head, drive), (H::Button { .. }, D::HexSocket { .. })),
            "iso10642-2004" =>
                matches!((head, drive), (H::Countersunk { .. }, D::HexSocket { .. })),
            "accu-shb-m3-10-a2-r360" =>
                matches!((head, drive), (H::Button { .. }, D::Hexalobular { .. })),
            _ => false,
        },
        "unexpected head/drive for {family}: {head:?}, {drive:?}"
    );
}

fn compare_geometry(part: &ResolvedMechanicalPart, source: &MechanicalPartParameters, row: &Value) {
    let compare = |actual: f64, key: &str| close(actual, number(&row[key]), key);
    let frame = match part {
        ResolvedMechanicalPart::BearingEnvelope { frame, .. }
        | ResolvedMechanicalPart::Washer { frame, .. } => *frame,
        ResolvedMechanicalPart::HexNut { thread, .. }
        | ResolvedMechanicalPart::MachineScrew { thread, .. } => thread.frame,
        ResolvedMechanicalPart::WoodScrew { parameters } => parameters.frame,
        ResolvedMechanicalPart::CustomConnection { .. } => {
            panic!("engineered custom hardware is not sourced stock")
        }
    };
    assert_eq!(frame.origin, [0., 0., 0.]);
    assert_eq!(frame.u, [1., 0., 0.]);
    assert_eq!(frame.v, [0., 1., 0.]);
    assert_eq!(frame.n, [0., 0., 1.]);
    match part {
        ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            ..
        } => {
            compare(2. * bore_radius_mm, "bore_mm");
            compare(2. * outside_radius_mm, "outside_mm");
            compare(*width_mm, "width_mm");
            assert_eq!(row["protection"], "open");
            assert!(row.get("nominal_mm").is_none());
        }
        ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            ..
        } => {
            compare(2. * bore_radius_mm, "bore_mm");
            compare(2. * outside_radius_mm, "outside_mm");
            compare(*thickness_mm, "thickness_mm");
            assert_ne!(2. * bore_radius_mm, number(&row["nominal_mm"]));
        }
        ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } => {
            compare(*across_flats_mm, "across_flats_mm");
            compare(thread.length_mm, "height_mm");
            compare(2. * thread.nominal_major_radius_mm, "nominal_mm");
            compare(thread.pitch_mm, "pitch_mm");
            assert_eq!(thread.representation, ThreadRepresentation::Modeled);
            assert_eq!(
                source.thread_parameters().unwrap().fit,
                ThreadFit::NominalBasic
            );
        }
        ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            authored_length_mm,
            length_convention,
            shank_radius_mm,
            head,
            drive,
            thread,
        } => {
            let family = text(&row["family"]);
            screw_profiles(family, head, drive);
            compare(2. * thread.nominal_major_radius_mm, "nominal_mm");
            compare(thread.pitch_mm, "pitch_mm");
            compare(2. * shank_radius_mm, "shank_diameter_mm");
            assert_eq!(thread.representation, ThreadRepresentation::Modeled);
            assert_eq!(
                source.thread_parameters().unwrap().fit,
                ThreadFit::NominalBasic
            );
            if row.get("length_convention").is_some() {
                let convention = if matches!(head, ResolvedScrewHead::Countersunk { .. }) {
                    assert_eq!(row["length_convention"], "overall");
                    close(
                        *underhead_length_mm,
                        *authored_length_mm - head.height_mm(),
                        "underhead length",
                    );
                    ScrewLengthConvention::Overall
                } else {
                    assert_eq!(row["length_convention"], "underhead");
                    close(
                        *underhead_length_mm,
                        *authored_length_mm,
                        "underhead length",
                    );
                    ScrewLengthConvention::Underhead
                };
                assert_eq!(*length_convention, convention);
                compare(*authored_length_mm, "length_mm");
                let nominal = number(&row["nominal_mm"]);
                let selected_length = match family {
                    "fila-iso14579" if nominal == 6. => 30.,
                    "fila-iso14579" => 40.,
                    "iso7380-1-2011" if nominal == 10. => 16.,
                    "accu-shb-m3-10-a2-r360" => 10.,
                    _ => 12.,
                };
                close(
                    *authored_length_mm,
                    selected_length,
                    "selected discrete profile length",
                );
                close(
                    thread.length_mm,
                    *underhead_length_mm - number(&row["thread_runout_mm"]),
                    "thread extent",
                );
                let MechanicalPartParameters::MachineScrew { head, drive, .. } = source else {
                    unreachable!()
                };
                assert_eq!(*head, serde_json::from_value(row["head"].clone()).unwrap());
                assert_eq!(
                    *drive,
                    serde_json::from_value(row["drive"].clone()).unwrap()
                );
            } else {
                assert_eq!(*length_convention, ScrewLengthConvention::Underhead);
                compare(*authored_length_mm, "underhead_length_mm");
                compare(thread.length_mm, "thread_length_mm");
                compare(head.height_mm(), "head_height_mm");
                match (head, drive) {
                    (
                        ResolvedScrewHead::SocketCap { radius_mm, .. },
                        ResolvedScrewDrive::HexSocket {
                            across_flats_mm,
                            depth_mm,
                        },
                    ) => {
                        compare(2. * radius_mm, "head_diameter_mm");
                        compare(*across_flats_mm, "drive_across_flats_mm");
                        compare(*depth_mm, "drive_depth_mm");
                    }
                    (
                        ResolvedScrewHead::ExternalHex {
                            across_flats_mm, ..
                        },
                        ResolvedScrewDrive::ExternalHex,
                    ) => compare(*across_flats_mm, "head_across_flats_mm"),
                    _ => unreachable!(),
                }
            }
        }
        ResolvedMechanicalPart::WoodScrew { parameters: wood } => {
            screw_profiles(text(&row["family"]), &wood.head, &wood.drive);
            assert_eq!(wood.length_convention, ScrewLengthConvention::Underhead);
            assert_eq!(wood.point, WoodPointIntent::ManufacturerUnspecified);
            assert_eq!(
                wood.representation,
                WoodScrewRepresentation::SimplifiedEnvelope
            );
            assert!(source.thread_parameters().is_none());
            compare(wood.nominal_diameter_mm, "nominal_mm");
            compare(wood.authored_length_mm, "underhead_length_mm");
            close(
                wood.minimum_tip_inclusive_thread_extent_mm,
                wood.underhead_length_mm * number(&row["minimum_thread_extent_ratio"]),
                "minimum tip-inclusive extent",
            );
            for (actual, key) in [
                (wood.reference_profile.pitch_mm, "pitch_mm"),
                (wood.reference_profile.core_diameter_mm, "core_diameter_mm"),
                (
                    wood.reference_profile.included_angle_deg,
                    "included_angle_deg",
                ),
            ] {
                close(actual, number(&row["v_reference"][key]), key);
            }
            let ResolvedScrewHead::ExternalHex {
                across_flats_mm,
                height_mm,
            } = wood.head
            else {
                unreachable!()
            };
            close(
                across_flats_mm,
                number(&row["head"]["across_flats_max_mm"]),
                "wood across flats",
            );
            close(
                height_mm,
                number(&row["head"]["height_nominal_mm"]),
                "wood head height",
            );
        }
        ResolvedMechanicalPart::CustomConnection { .. } => {
            panic!("engineered custom hardware is not sourced stock")
        }
    }
}

fn source_receipt<'a>(manifest: &str, source: &'a Value, row: &'a Value) -> (&'a str, String) {
    let revision = text(&source["revision"]);
    match manifest {
        "curation.json" => (
            text(&source["data_source"]),
            format!("{BOLTS} / curated {revision}"),
        ),
        "washers-curation.json" => (
            text(&source["data_source"]),
            format!("{BOLTS} / washer curation {revision}"),
        ),
        "nuts-curation.json" => (
            text(&source["data_source"]),
            format!("{BOLTS} / nut curation {revision}"),
        ),
        "machine-screws-curation.json" if row.get("length_convention").is_some() => (
            text(&row["verification_url"]),
            format!(
                "screw profile curation {}",
                text(&source["profile_revision"])
            ),
        ),
        "machine-screws-curation.json" => {
            let file = if row["family"] == "socket_cap" {
                "hex_socket.blt"
            } else {
                "hex.blt"
            };
            let origin = array(&source["data_sources"])
                .iter()
                .find(|s| text(&s["file"]).ends_with(file))
                .unwrap();
            (
                text(&origin["url"]),
                format!("{BOLTS} / machine screw curation {revision}"),
            )
        }
        "wood-screws-curation.json" => {
            let origin = array(&source["sources"])
                .iter()
                .find(|s| s["id"] == row["dimensional_source"])
                .unwrap();
            (
                text(&origin["url"]),
                format!("wood screw curation {revision}"),
            )
        }
        _ => panic!("unexpected source manifest {manifest}"),
    }
}

#[test]
fn checked_projects_reproduce_all_148_curated_default_recipes() {
    let root = library();
    let bundle = json(&root, "library.json");
    let groups = [
        ("curation.json", 4),
        ("washers-curation.json", 10),
        ("nuts-curation.json", 19),
        ("machine-screws-curation.json", 71),
        ("wood-screws-curation.json", 44),
    ];
    let mut rows = BTreeMap::new();
    let mut selections = BTreeSet::new();
    for (manifest, expected) in groups {
        let data = json(&root, manifest);
        let extra = data.get("profile_entries").map_or(&[][..], array);
        assert_eq!(array(&data["entries"]).len() + extra.len(), expected);
        for entry in array(&data["entries"]).iter().chain(extra) {
            let filename = text(&entry["file"]).to_owned();
            let designation = (manifest == "curation.json").then(|| text(&entry["designation"]));
            let length = entry["length_mm"]
                .as_f64()
                .or_else(|| entry["underhead_length_mm"].as_f64());
            assert!(
                selections.insert((
                    manifest,
                    entry["family"].as_str().unwrap_or_default().to_owned(),
                    entry["nominal_mm"].as_f64().map(f64::to_bits),
                    length.map(f64::to_bits),
                    designation.map(str::to_owned),
                )),
                "duplicate source selection {filename}"
            );
            let (source_url, revision) = source_receipt(manifest, &data, entry);
            assert!(
                rows.insert(filename, (entry.clone(), source_url.to_owned(), revision))
                    .is_none()
            );
        }
    }
    assert_eq!(rows.len(), 148);
    let entries = array(&bundle["entries"]);
    assert_eq!(entries.len(), 172);
    let capabilities = project_file::supported_capabilities();
    let mut pins = BTreeSet::new();
    let mut file_names = BTreeSet::new();
    let mut unrelated = 0;
    for entry in entries {
        let name = text(&entry["file"]);
        assert!(file_names.insert(name.to_owned()));
        let bytes = fs::read(root.join(name)).unwrap();
        assert_eq!(
            ContentHash::of(&bytes).to_string(),
            entry["sha256"],
            "{name}"
        );
        let checked =
            project_file::card(&mut bytes.as_slice(), Default::default(), &capabilities).unwrap();
        assert_eq!(checked.document.to_string(), entry["document"], "{name}");
        assert_eq!(checked.symbol_definitions, 1, "{name}");
        assert!(
            checked.info_error.is_none() && checked.info.preview.is_some(),
            "{name}"
        );
        let Some((row, source_url, revision)) = rows.remove(name) else {
            assert!(
                checked.mechanical_parts.is_empty(),
                "unlisted mechanical source {name}"
            );
            unrelated += 1;
            continue;
        };
        let [card] = checked.mechanical_parts.as_slice() else {
            panic!("expected one mechanical card for {name}")
        };
        assert!(
            card.problem.is_none() && card.matches_reference_dimensions,
            "{name}: {:?}",
            card.problem
        );
        let saved = project_file::read(&mut bytes.as_slice(), Default::default(), &capabilities)
            .unwrap()
            .project;
        let definitions = symbols::definitions(&saved.document).unwrap();
        let [(_, definition)] = definitions.as_slice() else {
            panic!("expected one definition for {name}")
        };
        assert!(
            pins.insert(definition.pin().unwrap().to_string()),
            "duplicate mechanical pin {name}"
        );
        assert_eq!(
            card.symbol,
            Some((definition.symbol, definition.revision.clone()))
        );
        let author = hardware(&saved.document);
        let template = hardware(&definition.template);
        assert_eq!(author.provenance, template.provenance, "{name}");
        let provenance = template.provenance.as_ref().unwrap();
        assert_eq!(provenance.source_url, source_url, "{name}");
        assert_eq!(provenance.revision, revision, "{name}");
        assert!(
            provenance
                .catalog_id
                .ends_with(&format!(".{}", text(&row["id"]))),
            "{name}"
        );
        assert_eq!(
            card.source_url.as_deref(),
            Some(provenance.source_url.as_str())
        );
        assert_eq!(card.catalog_facts, provenance.catalog_facts);
        let facts = provenance.catalog_facts.as_ref();
        assert_eq!(
            facts.and_then(|f| f.nominal_fastener_diameter_mm),
            row["nominal_mm"].as_f64(),
            "{name}"
        );
        assert!(
            facts.is_none_or(|f| f.material.is_none() && f.grade.is_none()),
            "supplier SKU text is not an assigned material: {name}"
        );
        let a = parameters::evaluate(&saved.document).unwrap();
        let t = parameters::evaluate(&definition.template).unwrap();
        assert!(author.matches_reference_dimensions(&a).unwrap(), "{name}");
        assert!(template.matches_reference_dimensions(&t).unwrap(), "{name}");
        let resolved = template.parameters.resolve(&t).unwrap();
        assert_eq!(author.parameters.resolve(&a).unwrap(), resolved, "{name}");
        compare_geometry(&resolved, &provenance.reference_geometry, &row);
        let inputs = template.inputs().unwrap();
        assert_eq!(definition.exposed.len(), inputs.len(), "{name}");
        assert!(
            definition
                .exposed
                .iter()
                .all(|id| inputs.iter().any(|input| input.target == *id)),
            "{name}"
        );
        let source = serde_json::to_value(&provenance.reference_geometry).unwrap();
        let frame = source.get("frame").unwrap_or(&source["thread"]["frame"]);
        assert_eq!(
            frame,
            &serde_json::to_value(model::PrimitiveFrame::default()).unwrap(),
            "{name}"
        );
    }
    assert!(rows.is_empty());
    assert_eq!(unrelated, 24);
    let actual = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".tcd"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, file_names);
}

#[test]
fn packaged_source_and_rights_receipts_match_the_retained_revisions() {
    let root = library();
    let bundle = json(&root, "library.json");
    let requires = array(&bundle["requires"])
        .iter()
        .map(text)
        .collect::<BTreeSet<_>>();
    for (curation, original) in [
        ("curation.json", "bearings.blt"),
        ("washers-curation.json", "washer.blt"),
        ("nuts-curation.json", "nut.blt"),
    ] {
        let data = json(&root, curation);
        assert_eq!(data["data_revision"], BOLTS);
        assert_eq!(data["data_license"], "LGPL-2.1-or-later");
        let name = format!("sources/bolts/{original}");
        assert!(requires.contains(name.as_str()));
        assert_eq!(
            ContentHash::of(&fs::read(root.join(name)).unwrap()).to_string(),
            data["data_sha256"]
        );
    }
    let screws = json(&root, "machine-screws-curation.json");
    for source in array(&screws["data_sources"]) {
        let file = text(&source["file"]);
        assert!(requires.contains(file));
        assert_eq!(source["revision"], BOLTS);
        assert_eq!(source["license"], "LGPL-2.1-or-later");
        assert_eq!(
            ContentHash::of(&fs::read(root.join(file)).unwrap()).to_string(),
            source["sha256"]
        );
    }
    let profiles = array(&screws["profile_sources"]);
    assert_eq!(profiles.len(), 11);
    for row in array(&screws["profile_entries"]) {
        let verification = &row["source_facts"]["verification"];
        let matching = profiles
            .iter()
            .filter(|source| {
                source["url"] == row["verification_url"]
                    && source["sha256"] == verification["source_sha256"]
            })
            .collect::<Vec<_>>();
        let [source] = matching.as_slice() else {
            panic!("ambiguous source for {}", row["id"])
        };
        assert!(!text(&source["edition"]).is_empty());
        assert!(!text(&source["rights"]).is_empty());
    }
    let wood = json(&root, "wood-screws-curation.json");
    let ledger = json(&root, "sources/wood-screws/source-ledger.json");
    assert_eq!(wood["sources"], ledger["sources"]);
    for row in array(&wood["entries"]) {
        for key in [
            "dimensional_source",
            "profile_reference_source",
            "minimum_extent_source",
        ] {
            assert!(
                array(&ledger["sources"])
                    .iter()
                    .any(|source| source["id"] == row[key] && !text(&source["rights"]).is_empty())
            );
        }
    }
    for notice in [
        "sources/bolts/COPYING.LESSER",
        "sources/bolts/WASHER-NOTICES.md",
        "sources/bolts/NUT-NOTICES.md",
        "sources/bolts/MACHINE-SCREW-NOTICES.md",
        "sources/bolts/SCREW-PROFILE-NOTICES.md",
        "sources/wood-screws/WOOD-SCREW-NOTICES.md",
        "sources/wood-screws/source-ledger.json",
    ] {
        assert!(
            requires.contains(notice) && root.join(notice).is_file(),
            "{notice}"
        );
    }
    assert!(
        requires
            .iter()
            .all(|file| !file.to_ascii_lowercase().ends_with(".pdf"))
    );
}
