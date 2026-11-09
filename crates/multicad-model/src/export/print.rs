//! Print packages: the finished Parts and placed occurrences a user chose,
//! optionally with the removable FDM aids serving them, tessellated from
//! current exact solids into one 3MF and, on request, one binary STL per
//! packaged object. The 3MF holds one mesh resource per producing solid and
//! one build item per world Part or placed occurrence at its world
//! placement, in millimeters, so copies of one Part share their mesh.
//! A saved print plate uses independent, geometry-checked bed placements.
//! Neither path slices, drives a printer or establishes printability.
use super::{Exporter, GeometryExporter, checkpoint, failure, mesh};
use crate::{GeometryBackend, Model};
use multicad_core::{
    commands::Actor,
    document::{Document, EntityData, EntityId},
    exchange::*,
    model::{self, RigidPose},
    project_file::ContentHash,
};
use multicad_kernel::{BooleanOperation, GeometryErrorKind, GeometryPolicy, RigidTransform, Solid};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Upper bound on one written package file.
pub const MAX_PRINT_FILE_BYTES: usize = 512 * 1024 * 1024;
/// Distance treated as resting on the build plane, in millimeters.
const PLANE_MM: f64 = 1e-6;

struct Built {
    preview: PrintPreview,
    generated: Vec<Vec<u8>>,
    plate_meshes: Option<(Vec<mesh::MeshObject>, Vec<mesh::BuildItem>)>,
}

#[derive(Clone, Copy)]
enum Arrangement<'a> {
    Package,
    PlatePreview(&'a PrintPlateDefinition),
    PlateWrite(&'a PrintPlateDefinition),
}

fn direct_plate_roots(document: &Document, selection: &PrintSelection) -> Option<Vec<EntityId>> {
    let mut roots = Vec::new();
    let mut heads = BTreeSet::new();
    for part in &selection.parts {
        if !part.occurrence_path.is_empty() || part.role != PrintRole::Finished {
            return None;
        }
        let EntityData::Part { features, .. } = &document.get(part.part)?.data else {
            return None;
        };
        let head = features.iter().rev().find(|feature| {
            matches!(
                document.get(**feature).map(|entity| &entity.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        })?;
        roots.push(part.part);
        heads.insert(*head);
    }
    // A consumer outside the direct input closure can remove a body from the
    // final assembly. Keep the full membership check for those plates.
    if document.entities().any(|entity| {
        matches!(&entity.data, EntityData::Feature { inputs, suppressed: false, .. }
            if inputs.iter().any(|input| heads.contains(&input.target)))
    }) {
        return None;
    }
    crate::validate_source_roots(document, &roots).ok()
}
fn io(e: impl ToString) -> ExportError {
    failure(ErrorKind::Io, None, e)
}
/// A filename fragment from a Part name: path separators, reserved and
/// control characters become `_`, so a name never leaves the directory.
fn file_fragment(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    if cleaned.is_empty() {
        "part".into()
    } else {
        cleaned
    }
}
fn stl_suffixes(names: &[&str]) -> Vec<String> {
    let mut used = BTreeSet::new();
    names
        .iter()
        .map(|name| {
            let base = file_fragment(name);
            let mut candidate = format!("-{base}.stl");
            let mut n = 2;
            while !used.insert(candidate.to_lowercase()) {
                candidate = format!("-{base}-{n}.stl");
                n += 1;
            }
            candidate
        })
        .collect()
}
fn process_record(
    document: &Document,
    parameters: &multicad_core::parameters::EvaluatedParameters,
    id: EntityId,
) -> Result<PrintProcess, String> {
    let resolved = model::read_process(document, id)
        .and_then(|d| d.resolve(parameters))
        .map_err(|e| e.to_string())?;
    Ok(PrintProcess {
        id,
        name: document
            .get(id)
            .map_or_else(|| id.to_string(), |e| e.name.clone()),
        layer_height_mm: resolved.layer_height_mm,
        line_width_mm: resolved.line_width_mm,
        min_layers: resolved.min_layers,
        min_lines: resolved.min_lines,
        support_gap_mm: resolved.support_gap_mm,
        fit_clearance_mm: resolved.fit_clearance_mm,
    })
}
/// FDM aid inspection findings for the aids serving the chosen Parts, and
/// plain build-plane facts about the packaged objects.
fn findings(
    document: &Document,
    model: &Model,
    selection: &PrintSelection,
    objects: &[PrintObject],
) -> Vec<PrintFinding> {
    let mut out = vec![];
    let finished: BTreeSet<_> = selection
        .parts
        .iter()
        .filter(|p| p.role == PrintRole::Finished)
        .map(|p| p.part)
        .collect();
    let packaged: BTreeSet<_> = objects.iter().map(|o| o.part).collect();
    match model.fdm_aids(document) {
        Ok(inspections) => {
            for aid in inspections {
                let target = model::read_aid(document, aid.feature)
                    .map(|a| a.target)
                    .ok();
                let included = packaged.contains(&aid.part);
                if !included && !target.is_some_and(|t| finished.contains(&t)) {
                    continue;
                }
                let subject = if included {
                    aid.name.clone()
                } else {
                    format!("{} (left out)", aid.name)
                };
                for issue in aid.issues {
                    out.push(PrintFinding {
                        part: Some(aid.part),
                        subject: subject.clone(),
                        message: issue,
                    });
                }
            }
        }
        Err(e) => out.push(PrintFinding {
            part: e.entity,
            subject: "FDM aid inspection".into(),
            message: e.message,
        }),
    }
    for object in objects {
        let z = object.bounds_mm[0][2];
        if z < -PLANE_MM {
            out.push(PrintFinding {
                part: Some(object.part),
                subject: object.name.clone(),
                message: format!(
                    "Extends {:.3} mm below the build plane (z = 0); the package keeps model coordinates",
                    -z
                ),
            });
        }
    }
    if let Some(lowest) = objects
        .iter()
        .map(|o| o.bounds_mm[0][2])
        .min_by(f64::total_cmp)
        && lowest > PLANE_MM
    {
        out.push(PrintFinding {
            part: None,
            subject: "Package".into(),
            message: format!(
                "Lowest point is z = {lowest:.3} mm; no packaged Part rests on the build plane"
            ),
        });
    }
    out
}
fn transformed(local: &Solid, pose: RigidPose) -> Result<Solid, multicad_kernel::GeometryError> {
    local.transformed(RigidTransform {
        rotation: pose.rotation,
        translation_mm: pose.translation_mm,
    })
}

fn plate_rotation(placement: &PrintPlatePlacement, backend: GeometryBackend) -> [[f64; 3]; 3] {
    let rotation = placement.rotation();
    if backend == GeometryBackend::Rust
        && placement
            .rotation_deg
            .iter()
            .all(|angle| angle.rem_euclid(90.) == 0.)
    {
        // A composition of exact quarter turns is a signed permutation.
        rotation.map(|row| row.map(f64::round))
    } else {
        rotation
    }
}

fn plate_pose(
    local: &Solid,
    rotation: [[f64; 3]; 3],
    [x, y]: [f64; 2],
) -> Result<(RigidPose, [[f64; 3]; 2]), multicad_kernel::GeometryError> {
    let orientation = RigidPose {
        rotation,
        translation_mm: [0.; 3],
    };
    let bounds = if orientation == RigidPose::default() {
        local.bounds()?
    } else {
        transformed(local, orientation)?.bounds()?
    };
    let anchor = [x, y, 0.];
    let translation_mm = std::array::from_fn(|axis| anchor[axis] - bounds.min[axis]);
    let top = std::array::from_fn(|axis| anchor[axis] + (bounds.max[axis] - bounds.min[axis]));
    Ok((
        RigidPose {
            rotation,
            translation_mm,
        },
        [anchor, top],
    ))
}

enum Clearance {
    Separate,
    Overlap,
    Unresolved,
}

/// The caller has already found positive overlap of the Z intervals. For
/// parallel vertical cylinders the remaining exact geometric test is whether
/// the interiors of their two circular cross sections overlap in XY.
fn upright_cylinder_clearance(
    first: (&Solid, RigidPose),
    second: (&Solid, RigidPose),
    objects: [&PrintObject; 2],
    tolerance: f64,
) -> Option<Clearance> {
    let upright_radius = |(solid, pose): (&Solid, RigidPose)| {
        let [x, y, z] = pose.rotation[2];
        if x != 0. || y != 0. || z.abs() != 1. {
            return None;
        }
        Some(solid.axial_cylinder()?.circle().radius().as_f64())
    };
    let combined = upright_radius(first)? + upright_radius(second)?;
    let [a, b] = objects.map(|object| {
        let [min, max] = object.bounds_mm;
        [min[0].midpoint(max[0]), min[1].midpoint(max[1])]
    });
    let distance = (a[0] - b[0]).hypot(a[1] - b[1]);
    // Around tangency the computed distance cannot certify either result at
    // the modeling tolerance. Preserve the blocked preview in that case.
    Some(if distance < combined - tolerance {
        Clearance::Overlap
    } else if distance > combined + tolerance {
        Clearance::Separate
    } else {
        Clearance::Unresolved
    })
}

fn check_plate(
    [width, depth]: [f64; 2],
    objects: &[PrintObject],
    solids: &[(&Solid, RigidPose)],
    policy: GeometryPolicy,
    backend: GeometryBackend,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Option<String>, ExportError> {
    for object in objects {
        let [min, max] = object.bounds_mm;
        if min[0] < -PLANE_MM
            || min[1] < -PLANE_MM
            || max[0] > width + PLANE_MM
            || max[1] > depth + PLANE_MM
        {
            return Ok(Some(format!(
                "{} is outside the {:.1} × {:.1} mm print bed. Move or rotate it, or resize the bed.",
                object.name, width, depth
            )));
        }
    }

    // Build exact transformed solids only for pairs that can intersect. Face
    // contact has zero common volume and is valid for independent build items.
    let policy = GeometryPolicy {
        boolean_fuzzy_mm: 0.,
        ..policy
    };
    let mut posed: Vec<Option<Solid>> = (0..solids.len()).map(|_| None).collect();
    let mut queries = 0;
    for (a, object) in objects.iter().enumerate() {
        for (b, other) in objects.iter().enumerate().skip(a + 1) {
            let intersects = (0..3).all(|axis| {
                object.bounds_mm[0][axis].max(other.bounds_mm[0][axis])
                    < object.bounds_mm[1][axis].min(other.bounds_mm[1][axis])
            });
            if !intersects {
                continue;
            }
            checkpoint(interrupt)?;
            queries += 1;
            if queries > 256 {
                return Ok(Some(
                    "Too many overlapping part envelopes to finish the collision check. Spread the parts apart and preview again.".into(),
                ));
            }
            let overlap = || {
                format!(
                    "{} overlaps {} on the print bed. Move or rotate one of them.",
                    object.name, other.name
                )
            };
            let unverified = || {
                format!(
                    "The Rust kernel cannot verify clearance between {} and {} while their bounds overlap. Separate their bounds and preview again.",
                    object.name, other.name
                )
            };
            if backend == GeometryBackend::Rust
                && let Some(clearance) = upright_cylinder_clearance(
                    solids[a],
                    solids[b],
                    [object, other],
                    policy.linear_tolerance_mm,
                )
            {
                match clearance {
                    Clearance::Separate => continue,
                    Clearance::Overlap => return Ok(Some(overlap())),
                    Clearance::Unresolved => return Ok(Some(unverified())),
                }
            }
            let inspect_error = |e: multicad_kernel::GeometryError| {
                failure(
                    ErrorKind::Invalid,
                    Some(object.part),
                    format!(
                        "Cannot check {} against {} on the print bed: {e}",
                        object.name, other.name
                    ),
                )
            };
            for index in [a, b] {
                if posed[index].is_none() {
                    let (solid, pose) = solids[index];
                    posed[index] = Some(transformed(solid, pose).map_err(inspect_error)?);
                }
            }
            let first = posed[a].as_ref().expect("plate pose");
            let second = posed[b].as_ref().expect("plate pose");
            let common = match backend {
                GeometryBackend::OpenCascade => first.intersect_parallel(second, policy),
                GeometryBackend::Rust => {
                    match first.boolean(second, BooleanOperation::Intersect, policy) {
                        Err(error) if error.kind == GeometryErrorKind::Unsupported => {
                            return Ok(Some(unverified()));
                        }
                        checked => checked,
                    }
                }
            }
            .map_err(inspect_error)?;
            if let Some(common) = common
                && common.volume_mm3().map_err(inspect_error)? > 0.
            {
                return Ok(Some(overlap()));
            }
        }
    }
    Ok(None)
}

/// Read a generated file back and compare it with what was tessellated and
/// with the exact solids' volume, up to the chord deviation.
fn verify(
    file: &PrintFile,
    bytes: &[u8],
    objects: &[&PrintObject],
    quality: &MeshQuality,
    plate_meshes: Option<&[(&mesh::MeshObject, &mesh::BuildItem)]>,
) -> Result<f64, ExportError> {
    let (read, single) = match file.kind {
        PrintFileKind::ThreeMf => (mesh::read_three_mf(bytes), false),
        PrintFileKind::Stl => (mesh::read_stl(bytes), true),
    };
    let read = read.map_err(io)?;
    let allowance = |o: &PrintObject| {
        let [a, b] = o.bounds_mm;
        let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
        2. * (x * y + y * z + z * x) * quality.chord_deviation_mm * 1.5 + o.volume_mm3 * 1e-6
    };
    let relative = if single { 1e-5 } else { 1e-6 };
    let structure = match file.kind {
        PrintFileKind::ThreeMf => read.objects == file.objects && read.items == file.items,
        PrintFileKind::Stl => true,
    };
    // Facets are read back in build-item order, one item per packaged
    // object, so each placed object's volume is compared on its own.
    let mut facets = read.world.as_slice();
    let mut placed_ok = read.triangles == file.triangles;
    for (index, object) in objects.iter().enumerate() {
        if !placed_ok || facets.len() < object.triangles {
            placed_ok = false;
            break;
        }
        let (own, rest) = facets.split_at(object.triangles);
        facets = rest;
        if let Some(plate_meshes) = plate_meshes {
            let (resource, pose) = plate_meshes[index];
            let matches = own
                .iter()
                .zip(&resource.triangles)
                .all(|((a, b, c), triangle)| {
                    [a, b, c]
                        .into_iter()
                        .zip(triangle)
                        .all(|(actual, &vertex)| {
                            let expected = pose.apply(resource.vertices[vertex as usize]);
                            actual.iter().zip(expected).all(|(actual, expected)| {
                                let tolerance = if single {
                                    1e-6 + expected.abs() * 1e-7
                                } else {
                                    1e-8
                                };
                                (actual - expected).abs() <= tolerance
                            })
                        })
                });
            if own.len() != resource.triangles.len() || !matches {
                return Err(failure(
                    ErrorKind::Invalid,
                    Some(object.part),
                    format!(
                        "Readback placement of {} differs from the verified print plate",
                        object.name
                    ),
                ));
            }
        }

        let volume = mesh::ReadMesh {
            unit_scale: 1.,
            objects: 1,
            items: 1,
            triangles: own.len(),
            world: own.to_vec(),
        }
        .volume_mm3();
        placed_ok = (volume - object.mesh_volume_mm3).abs()
            <= object.mesh_volume_mm3.abs() * relative + 1e-6
            && (volume - object.volume_mm3).abs() <= allowance(object);
    }
    if !structure || !placed_ok || !facets.is_empty() || (read.unit_scale - 1.).abs() > 1e-12 {
        return Err(failure(
            ErrorKind::Invalid,
            file.part,
            format!(
                "Readback of the generated {} differs from its objects, millimeter unit, triangles or volume",
                match file.kind {
                    PrintFileKind::ThreeMf => "3MF",
                    PrintFileKind::Stl => "STL",
                }
            ),
        ));
    }
    Ok(read.volume_mm3())
}
impl GeometryExporter<'_> {
    fn print_provider(&self) -> String {
        format!("print-3mf-stl.1;{}", self.provider())
    }
    fn build_print(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        arrangement: Arrangement<'_>,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Built, ExportError> {
        if actor.id.trim().is_empty() || actor.id.len() > 256 {
            return Err(failure(
                ErrorKind::Invalid,
                None,
                "Invalid print actor identity",
            ));
        }
        checkpoint(interrupt)?;
        let plate = match arrangement {
            Arrangement::Package => None,
            Arrangement::PlatePreview(plate) | Arrangement::PlateWrite(plate) => Some(plate),
        };
        let mut selection = match plate {
            Some(plate) => resolve_print_plate(document, revision, plate)?,
            None => resolve_print(document, revision, request)?,
        };
        let manufacturing_limitations = model::has_manufacturing_limitations(document)
            .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let parameters = multicad_core::parameters::evaluate(document)
            .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let mut processes = vec![];
        let mut found = vec![];
        for id in &selection.processes {
            match process_record(document, &parameters, *id) {
                Ok(process) => processes.push(process),
                Err(message) => found.push(PrintFinding {
                    part: None,
                    subject: document
                        .get(*id)
                        .map_or_else(|| id.to_string(), |e| e.name.clone()),
                    message: format!("Process record cannot be resolved: {message}"),
                }),
            }
        }
        let mut objects = vec![];
        let mut meshes = vec![];
        let mut items = vec![];
        let mut plate_blocked = None;
        if !selection.parts.is_empty() {
            let compiler = if plate.is_some() || self.backend == GeometryBackend::Rust {
                crate::Compiler {
                    omit_display: true,
                    ..self.exporter.compiler
                }
            } else {
                self.exporter.compiler
            };
            let roots = match arrangement {
                Arrangement::PlatePreview(_) => direct_plate_roots(document, &selection),
                _ => None,
            };
            let model = match roots {
                Some(roots) => compiler.compile_sources_with_backend(
                    document,
                    revision,
                    &roots,
                    self.backend,
                    interrupt,
                ),
                None => compiler.compile_with_backend(document, revision, self.backend, interrupt),
            }
            .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
            if model.snapshot().stamp.source_hash != selection.stamp.source_hash {
                return Err(failure(
                    ErrorKind::Stale,
                    None,
                    "Model source differs from the print selection",
                ));
            }
            let mut unavailable = vec![];
            // One mesh resource per producing solid: a world Part and its
            // placed occurrences share it, placed by their build items.
            let mut shared: BTreeMap<EntityId, usize> = BTreeMap::new();
            let mut plate_solids = vec![];
            for (part_index, part) in selection.parts.iter().enumerate() {
                checkpoint(interrupt)?;
                let skip = |reason: &str| PrintSkip {
                    entity: part.part,
                    name: part.name.clone(),
                    reason: reason.into(),
                };
                let snapshot = model.snapshot();
                let own = snapshot
                    .bodies
                    .get(&part.part)
                    .and_then(|producer| model.solid(*producer).map(|s| (*producer, s)));
                let invalid = |e: String| failure(ErrorKind::Invalid, Some(part.part), e);
                // (producer, local solid, pose, exact world solid if it differs)
                let placed = match part.occurrence_path.split_first() {
                    None => own.map(|(producer, solid)| {
                        (producer, solid, RigidPose::default(), None::<Solid>)
                    }),
                    Some((root, members)) => {
                        let path = model::OccurrencePath {
                            root: *root,
                            members: members.to_vec(),
                        };
                        match model.leaf_snapshot(&path) {
                            None => None,
                            Some(leaf) if leaf.component == part.part => model
                                .solid(leaf.producer)
                                .map(|local| (leaf.producer, local, leaf.pose, None)),
                            Some(_) if plate.is_some() => None,
                            // An aid riding on its placed target: the aid's
                            // solid shares the target's frame and pose.
                            Some(leaf) => match own {
                                Some((producer, local)) => {
                                    let world = local
                                        .transformed(RigidTransform {
                                            rotation: leaf.pose.rotation,
                                            translation_mm: leaf.pose.translation_mm,
                                        })
                                        .map_err(|e| invalid(e.to_string()))?;
                                    Some((producer, local, leaf.pose, Some(world)))
                                }
                                None => None,
                            },
                        }
                    }
                };
                let Some((producer, local, pose, transformed)) = placed else {
                    unavailable.push(skip(if part.occurrence_path.is_empty() {
                        if model.snapshot().consumed_bodies.contains(&part.part) {
                            "Consumed by a later operation; package the resulting Part"
                        } else {
                            "No current unsuppressed solid"
                        }
                    } else {
                        "No current exact geometry for this placed occurrence"
                    }));
                    continue;
                };
                if manufacturing_limitations
                    && let Some(limitation) =
                        model::manufacturing_limitation_in_history(document, producer)
                            .map_err(|e| invalid(e.to_string()))?
                {
                    let name = document
                        .get(limitation.feature)
                        .map_or_else(|| limitation.feature.to_string(), |e| e.name.clone());
                    return Err(failure(
                        ErrorKind::Unsupported,
                        Some(limitation.feature),
                        format!(
                            "{name}: {} Creating a print package is unavailable.",
                            limitation.message()
                        ),
                    ));
                }
                let (pose, bounds_mm, volume) = if let Some(plate) = plate {
                    let placement = plate
                        .placements
                        .get(part_index)
                        .filter(|placement| {
                            placement.source.part == part.part
                                && placement.source.occurrence_path == part.occurrence_path
                        })
                        .ok_or_else(|| {
                            invalid("Print plate placement and resolved Part disagree".into())
                        })?;
                    let rotation = plate_rotation(placement, self.backend);
                    let (pose, bounds) = plate_pose(local, rotation, placement.position_mm)
                        .map_err(|e| invalid(e.to_string()))?;
                    plate_solids.push((local, pose));
                    (
                        pose,
                        bounds,
                        local.volume_mm3().map_err(|e| invalid(e.to_string()))?,
                    )
                } else {
                    let world = match (&transformed, part.occurrence_path.split_first()) {
                        (Some(world), _) => world,
                        (None, None) => local,
                        (None, Some((root, members))) => model
                            .leaf_solid(&model::OccurrencePath {
                                root: *root,
                                members: members.to_vec(),
                            })
                            .ok_or_else(|| {
                                invalid(
                                    "Exact world geometry of the placed occurrence is unavailable"
                                        .into(),
                                )
                            })?,
                    };
                    let bounds = world.bounds().map_err(|e| invalid(e.to_string()))?;
                    let volume = world.volume_mm3().map_err(|e| invalid(e.to_string()))?;
                    (pose, [bounds.min, bounds.max], volume)
                };
                let index = match shared.get(&producer) {
                    Some(index) => *index,
                    None => {
                        let name = document
                            .get(part.part)
                            .map_or_else(|| part.name.clone(), |e| e.name.clone());
                        meshes.push(
                            mesh::tessellate_manufacturing(local, &request.quality, &name)
                                .map_err(invalid)?,
                        );
                        shared.insert(producer, meshes.len() - 1);
                        meshes.len() - 1
                    }
                };
                let item = mesh::BuildItem {
                    object: index,
                    rotation: pose.rotation,
                    translation_mm: pose.translation_mm,
                };
                let resource = &meshes[index];
                let placed_vertices: Vec<_> =
                    resource.vertices.iter().map(|p| item.apply(*p)).collect();
                objects.push(PrintObject {
                    part: part.part,
                    occurrence_path: part.occurrence_path.clone(),
                    name: part.name.clone(),
                    role: part.role.clone(),
                    bounds_mm,
                    plate_pose: plate.map(|_| pose),
                    volume_mm3: volume,
                    triangles: resource.triangles.len(),
                    mesh_volume_mm3: mesh::volume(&placed_vertices, &resource.triangles).abs(),
                });
                items.push(item);
            }
            if let Some(plate) = plate {
                plate_blocked = if let Some(missing) = unavailable.first() {
                    Some(format!(
                        "{} cannot be placed on the print bed: {}",
                        missing.name, missing.reason
                    ))
                } else {
                    check_plate(
                        plate.bed_mm,
                        &objects,
                        &plate_solids,
                        self.exporter.compiler.policy,
                        self.backend,
                        interrupt,
                    )?
                };
            }
            selection.parts.retain(|p| {
                !unavailable
                    .iter()
                    .any(|u| u.entity == p.part && u.name == p.name)
            });
            selection.skipped.extend(unavailable);
            if plate.is_none() {
                found.extend(findings(document, &model, &selection, &objects));
            }
        }
        let blocked = if plate_blocked.is_some() {
            plate_blocked
        } else if plate.is_some() && objects.is_empty() {
            Some("Add at least one finished Part to the print plate.".into())
        } else if objects.is_empty() {
            Some(match &request.source {
                PrintSource::Parts { parts } if parts.is_empty() => {
                    "Choose finished Parts or placed copies in the Printable Parts list, or choose Parts by tag".into()
                }
                PrintSource::Tag { tag } => {
                    format!(
                        "No world Part or placed occurrence with a current solid carries the tag “{tag}”"
                    )
                }
                PrintSource::Parts { .. } => {
                    "None of the chosen entities resolves to a world Part or placed occurrence with a current solid".into()
                }
            })
        } else {
            None
        };
        let mut files = vec![];
        let mut generated = vec![];
        if blocked.is_none() {
            files.push(PrintFile {
                kind: PrintFileKind::ThreeMf,
                part: None,
                occurrence_path: vec![],
                suffix: ".3mf".into(),
                objects: meshes.len(),
                items: items.len(),
                triangles: objects.iter().map(|o| o.triangles).sum(),
            });
            generated.push(
                mesh::three_mf(&meshes, &items, LengthUnit::Millimeter)
                    .map_err(|e| failure(ErrorKind::Invalid, None, e))?,
            );
            if request.stl_per_part {
                let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
                for ((object, item), suffix) in objects.iter().zip(&items).zip(stl_suffixes(&names))
                {
                    checkpoint(interrupt)?;
                    if self.backend == GeometryBackend::Rust {
                        mesh::validate_float_positions(
                            &meshes[item.object],
                            LengthUnit::Millimeter,
                            Some(item),
                        )
                        .map_err(|e| failure(ErrorKind::Invalid, Some(object.part), e))?;
                    }
                    generated.push(mesh::stl(
                        std::slice::from_ref(&meshes[item.object]),
                        &[mesh::BuildItem {
                            object: 0,
                            ..item.clone()
                        }],
                        LengthUnit::Millimeter,
                        &object.name,
                    ));
                    files.push(PrintFile {
                        kind: PrintFileKind::Stl,
                        part: Some(object.part),
                        occurrence_path: object.occurrence_path.clone(),
                        suffix,
                        objects: 1,
                        items: 1,
                        triangles: object.triangles,
                    });
                }
            }
            if let Some(large) = generated
                .iter()
                .position(|b| b.len() > MAX_PRINT_FILE_BYTES)
            {
                return Err(failure(
                    ErrorKind::Budget,
                    files[large].part,
                    "A print package file exceeds 512 MiB; coarsen the mesh quality or choose fewer Parts",
                ));
            }
        }
        let provider = self.print_provider();
        let plan = serde_json::to_vec(&(
            &provider,
            selection.stamp.source_hash,
            request,
            &objects,
            &files,
            &selection.skipped,
            generated
                .iter()
                .map(|b| ContentHash::of(b))
                .collect::<Vec<_>>(),
        ))
        .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let plan_hash = ContentHash::of(&plan);
        let plan_hash = match plate {
            Some(plate) => ContentHash::of(
                &serde_json::to_vec(&(plan_hash, plate))
                    .map_err(|e| failure(ErrorKind::Invalid, None, e))?,
            ),
            None => plan_hash,
        };
        Ok(Built {
            preview: PrintPreview {
                requested_by: actor.clone(),
                key: PrintKey {
                    source: selection.stamp.clone(),
                    plan_hash,
                },
                provider,
                request: request.clone(),
                selection,
                objects,
                files,
                processes,
                findings: found,
                blocked,
            },
            generated,
            plate_meshes: plate.map(|_| (meshes, items)),
        })
    }
    fn print_preview(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintPreview, ExportError> {
        Ok(self
            .build_print(
                document,
                revision,
                request,
                Arrangement::Package,
                actor,
                interrupt,
            )?
            .preview)
    }
    fn print_plate_preview(
        &self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintPreview, ExportError> {
        Ok(self
            .build_print(
                document,
                revision,
                &plate.summary_request(document),
                Arrangement::PlatePreview(plate),
                actor,
                interrupt,
            )?
            .preview)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_print(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintReceipt, ExportError> {
        self.write_print_plan(
            document, revision, request, None, expected, path, actor, interrupt,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_print_plate(
        &self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        expected: &PrintKey,
        path: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintReceipt, ExportError> {
        self.write_print_plan(
            document,
            revision,
            &plate.summary_request(document),
            Some(plate),
            expected,
            path,
            actor,
            interrupt,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_print_plan(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        plate: Option<&PrintPlateDefinition>,
        expected: &PrintKey,
        path: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintReceipt, ExportError> {
        expected.source.check(document, revision)?;
        let (directory, stem) = package_target(path)?;
        let arrangement = plate.map_or(Arrangement::Package, Arrangement::PlateWrite);
        let built = self.build_print(document, revision, request, arrangement, actor, interrupt)?;
        if built.preview.key != *expected {
            return Err(failure(
                ErrorKind::Stale,
                None,
                "Print package plan changed; preview again before writing",
            ));
        }
        if let Some(reason) = &built.preview.blocked {
            return Err(failure(ErrorKind::Empty, None, reason));
        }
        let preview = &built.preview;
        let mut volumes = vec![];
        for (file, bytes) in preview.files.iter().zip(&built.generated) {
            let object_indices: Vec<_> = preview
                .objects
                .iter()
                .enumerate()
                .filter(|(_, object)| {
                    file.part.is_none_or(|part| {
                        object.part == part && object.occurrence_path == file.occurrence_path
                    })
                })
                .map(|(index, _)| index)
                .collect();
            let objects: Vec<_> = object_indices
                .iter()
                .map(|&index| &preview.objects[index])
                .collect();
            let plate_placements: Option<Vec<_>> =
                built.plate_meshes.as_ref().map(|(meshes, items)| {
                    object_indices
                        .iter()
                        .map(|&index| {
                            let item = &items[index];
                            (&meshes[item.object], item)
                        })
                        .collect()
                });
            volumes.push(verify(
                file,
                bytes,
                &objects,
                &request.quality,
                plate_placements.as_deref(),
            )?);
        }
        checkpoint(interrupt)?;
        let targets: Vec<PathBuf> = preview
            .files
            .iter()
            .map(|f| directory.join(f.filename(&stem)))
            .collect();
        for target in &targets {
            if let Ok(meta) = fs::symlink_metadata(target)
                && (!meta.is_file() || meta.file_type().is_symlink())
            {
                return Err(io(format!(
                    "{} exists and is not a regular file; nothing was written",
                    target.display()
                )));
            }
        }
        let mut staged: Vec<PathBuf> = vec![];
        let cleanup = |staged: &[PathBuf]| {
            for path in staged {
                let _ = fs::remove_file(path);
            }
        };
        for (target, bytes) in targets.iter().zip(&built.generated) {
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let temp = directory.join(format!(".{name}.{}.partial", EntityId::new()));
            let written = (|| -> std::io::Result<()> {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                file.write_all(bytes)?;
                file.sync_all()
            })();
            if let Err(e) = written {
                let _ = fs::remove_file(&temp);
                cleanup(&staged);
                return Err(io(format!(
                    "Could not stage {name}: {e}; nothing was written"
                )));
            }
            staged.push(temp);
        }
        if let Err(e) = checkpoint(interrupt) {
            cleanup(&staged);
            return Err(e);
        }
        let mut files = vec![];
        for (index, (temp, target)) in staged.iter().zip(&targets).enumerate() {
            if let Err(e) = fs::rename(temp, target) {
                cleanup(&staged[index..]);
                let done: Vec<_> = files
                    .iter()
                    .map(|f: &PrintWritten| f.path.clone())
                    .collect();
                return Err(io(format!(
                    "Could not replace {}: {e}; already replaced: {}",
                    target.display(),
                    if done.is_empty() {
                        "none".into()
                    } else {
                        done.join(", ")
                    }
                )));
            }
            let file = &preview.files[index];
            let bytes = &built.generated[index];
            let on_disk = fs::read(target).map_err(io)?;
            if on_disk != *bytes {
                return Err(io(format!(
                    "{} differs from the verified package bytes after writing",
                    target.display()
                )));
            }
            files.push(PrintWritten {
                path: target.to_string_lossy().into_owned(),
                kind: file.kind,
                bytes: bytes.len() as u64,
                content_hash: ContentHash::of(bytes),
                objects: file.objects,
                triangles: file.triangles,
                mesh_volume_mm3: volumes[index],
            });
        }
        Ok(PrintReceipt {
            actor: actor.clone(),
            key: expected.clone(),
            files,
        })
    }
}

impl Exporter {
    /// Resolve and tessellate the package without writing anything.
    pub fn print_preview(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintPreview, ExportError> {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .print_preview(document, revision, request, actor, interrupt)
    }

    /// Verify and tessellate a saved plate without changing its source model.
    pub fn print_plate_preview(
        &self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintPreview, ExportError> {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .print_plate_preview(document, revision, plate, actor, interrupt)
    }

    /// Rebuild and verify the previewed package before publishing its files.
    #[allow(clippy::too_many_arguments)]
    pub fn write_print(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintReceipt, ExportError> {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .write_print(
                document, revision, request, expected, path, actor, interrupt,
            )
    }

    /// Write the exact arrangement from the preview, verifying each placement.
    #[allow(clippy::too_many_arguments)]
    pub fn write_print_plate(
        &self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        expected: &PrintKey,
        path: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintReceipt, ExportError> {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .write_print_plate(document, revision, plate, expected, path, actor, interrupt)
    }
}

/// The existing directory and file stem of a chosen absolute `.3mf` path.
fn package_target(path: &Path) -> Result<(PathBuf, String), ExportError> {
    let invalid = |m: &str| failure(ErrorKind::Invalid, None, m);
    if !path.is_absolute() {
        return Err(invalid("Choose an absolute 3MF file path"));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("3mf"))
    {
        return Err(invalid("A print package file must end in .3mf"));
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid("Name the 3MF print package"))?;
    let directory = path
        .parent()
        .filter(|p| p.is_dir())
        .ok_or_else(|| invalid("The 3MF package folder does not exist"))?;
    Ok((directory.to_owned(), stem))
}

enum JobRequest {
    Package(PrintRequest),
    Plate(PrintPlateDefinition),
}

enum Job {
    Preview {
        document: Document,
        revision: u64,
        request: JobRequest,
        actor: Actor,
    },
    Write {
        document: Document,
        revision: u64,
        request: JobRequest,
        expected: PrintKey,
        path: PathBuf,
        actor: Actor,
    },
}
#[derive(Clone)]
struct Info {
    id: EntityId,
    document: EntityId,
    revision: u64,
    actor: Actor,
    write: bool,
}
impl Info {
    fn reply(&self, result: PrintResult) -> PrintReply {
        PrintReply {
            id: self.id,
            document: self.document,
            revision: self.revision,
            actor: self.actor.clone(),
            result,
        }
    }
    fn failed(&self, message: &str) -> PrintReply {
        let error = failure(ErrorKind::Io, None, message);
        self.reply(if self.write {
            PrintResult::Write(Err(error))
        } else {
            PrintResult::Preview(Err(error))
        })
    }
}
struct Queued {
    info: Info,
    job: Job,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Slots {
    pending: VecDeque<Queued>,
    active: Option<(Info, Arc<AtomicBool>)>,
    replies: VecDeque<PrintReply>,
    closed: bool,
}
struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    stop: AtomicBool,
}
struct Exit(Arc<Shared>);
impl Drop for Exit {
    fn drop(&mut self) {
        let mut slots = self.0.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.closed {
            return;
        }
        slots.closed = true;
        self.0.stop.store(true, Ordering::Release);
        let message = "Print worker exited unexpectedly; accepted jobs failed";
        if let Some((info, _)) = slots.active.take() {
            slots.replies.push_back(info.failed(message));
        }
        while let Some(job) = slots.pending.pop_front() {
            slots.replies.push_back(job.info.failed(message));
        }
        self.0.wake.notify_all();
    }
}
/// A bounded FIFO owner thread for print previews and writes. Exact models
/// are built and dropped on this thread only; the UI thread polls replies.
pub struct PrintWorker {
    shared: Arc<Shared>,
}
const MAX_OUTSTANDING: usize = 4;
impl PrintWorker {
    pub fn new(exporter: Exporter) -> Result<Self, String> {
        Self::with_geometry_backend(exporter, GeometryBackend::OpenCascade)
    }
    pub fn with_geometry_backend(
        exporter: Exporter,
        backend: GeometryBackend,
    ) -> Result<Self, String> {
        Self::spawn(move |job, interrupt| {
            let exporter = exporter.with_backend(backend, None);
            match job {
                Job::Preview {
                    document,
                    revision,
                    request,
                    actor,
                } => {
                    let preview = match request {
                        JobRequest::Package(request) => exporter.print_preview(
                            &document,
                            revision,
                            &request,
                            &actor,
                            Some(interrupt),
                        ),
                        JobRequest::Plate(plate) => exporter.print_plate_preview(
                            &document,
                            revision,
                            &plate,
                            &actor,
                            Some(interrupt),
                        ),
                    };
                    PrintResult::Preview(preview.map(Box::new))
                }
                Job::Write {
                    document,
                    revision,
                    request,
                    expected,
                    path,
                    actor,
                } => {
                    let write = match request {
                        JobRequest::Package(request) => exporter.write_print(
                            &document,
                            revision,
                            &request,
                            &expected,
                            &path,
                            &actor,
                            Some(interrupt),
                        ),
                        JobRequest::Plate(plate) => exporter.write_print_plate(
                            &document,
                            revision,
                            &plate,
                            &expected,
                            &path,
                            &actor,
                            Some(interrupt),
                        ),
                    };
                    PrintResult::Write(write)
                }
            }
        })
    }
    fn spawn(
        handler: impl Fn(Job, &dyn Fn() -> bool) -> PrintResult + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let pair_proofs = crate::current_pair_cycle_proofs();
        std::thread::Builder::new()
            .name("multicad-print".into())
            .spawn(move || {
                let _proof_owner = crate::pair_cycle_proofs::ProofOwner::enter(pair_proofs);
                let _exit = Exit(Arc::clone(&worker));
                // Preview/write jobs retain exact source-keyed native features
                // on this worker only, with the existing bounded cache policy.
                // Drop it before Exit publishes any owner-failure replies.
                let _evaluation_owner = crate::worker_cache::EvaluationOwner::enter();
                loop {
                    let queued = {
                        let mut slots = worker.slots.lock().unwrap_or_else(|e| e.into_inner());
                        while slots.pending.is_empty() && !slots.closed {
                            slots = worker.wake.wait(slots).unwrap_or_else(|e| e.into_inner());
                        }
                        if slots.closed {
                            return;
                        }
                        let queued = slots.pending.pop_front().expect("pending print job");
                        slots.active = Some((queued.info.clone(), Arc::clone(&queued.cancel)));
                        queued
                    };
                    let interrupt = || {
                        worker.stop.load(Ordering::Acquire) || queued.cancel.load(Ordering::Acquire)
                    };
                    let result = handler(queued.job, &interrupt);
                    let mut slots = worker.slots.lock().unwrap_or_else(|e| e.into_inner());
                    slots.active = None;
                    if !slots.closed {
                        slots.replies.push_back(queued.info.reply(result));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared })
    }
    fn enqueue(
        &self,
        job: Job,
        info: impl FnOnce(EntityId) -> Info,
    ) -> Result<EntityId, ExportError> {
        let mut slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.closed {
            return Err(failure(ErrorKind::Io, None, "Print worker is closed"));
        }
        if slots.pending.len() + slots.replies.len() + usize::from(slots.active.is_some())
            >= MAX_OUTSTANDING
        {
            return Err(failure(
                ErrorKind::Budget,
                None,
                "Print queue is full; four accepted jobs are still outstanding",
            ));
        }
        let id = EntityId::new();
        slots.pending.push_back(Queued {
            info: info(id),
            job,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.shared.wake.notify_one();
        Ok(id)
    }
    pub fn preview(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.preview_request(
            document,
            revision,
            JobRequest::Package(request.clone()),
            actor,
        )
    }

    pub fn preview_plate(
        &self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.preview_request(document, revision, JobRequest::Plate(plate.clone()), actor)
    }

    fn preview_request(
        &self,
        document: &Document,
        revision: u64,
        request: JobRequest,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        let info = |id| Info {
            id,
            document: document.id(),
            revision,
            actor: actor.clone(),
            write: false,
        };
        self.enqueue(
            Job::Preview {
                document: document.clone(),
                revision,
                request,
                actor: actor.clone(),
            },
            info,
        )
    }

    pub fn write(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.write_request(
            document,
            revision,
            JobRequest::Package(request.clone()),
            expected,
            path,
            actor,
        )
    }

    pub fn write_plate(
        &self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.write_request(
            document,
            revision,
            JobRequest::Plate(plate.clone()),
            expected,
            path,
            actor,
        )
    }

    fn write_request(
        &self,
        document: &Document,
        revision: u64,
        request: JobRequest,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        let info = |id| Info {
            id,
            document: document.id(),
            revision,
            actor: actor.clone(),
            write: true,
        };
        self.enqueue(
            Job::Write {
                document: document.clone(),
                revision,
                request,
                expected: expected.clone(),
                path: path.to_owned(),
                actor: actor.clone(),
            },
            info,
        )
    }
    pub fn poll(&self) -> Option<PrintReply> {
        self.shared
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .pop_front()
    }
    pub fn pending(&self) -> bool {
        let slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.active.is_some() || !slots.pending.is_empty() || !slots.replies.is_empty()
    }
    pub fn cancel(&self, id: EntityId) -> bool {
        let slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        let flag = slots
            .active
            .as_ref()
            .filter(|(info, _)| info.id == id)
            .map(|(_, flag)| flag)
            .or_else(|| {
                slots
                    .pending
                    .iter()
                    .find(|j| j.info.id == id)
                    .map(|j| &j.cancel)
            });
        if let Some(flag) = flag {
            flag.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
}
impl Drop for PrintWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let mut slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.closed = true;
        slots.pending.clear();
        slots.replies.clear();
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn stl_names_stay_in_the_folder_and_are_unique() {
        assert_eq!(
            stl_suffixes(&["Lid", "lid", "../etc/passwd", "  ", "a:b"]),
            [
                "-Lid.stl",
                "-lid-2.stl",
                "-_etc_passwd.stl",
                "-part.stl",
                "-a_b.stl"
            ]
        );
    }
    #[test]
    fn readback_checks_each_placed_object_and_the_item_count() {
        // A unit cube shared by two items; the second sits 10 mm along x.
        let v = |x: f64, y: f64, z: f64| [x, y, z];
        let cube = mesh::MeshObject {
            name: "Cube".into(),
            vertices: vec![
                v(0., 0., 0.),
                v(1., 0., 0.),
                v(1., 1., 0.),
                v(0., 1., 0.),
                v(0., 0., 1.),
                v(1., 0., 1.),
                v(1., 1., 1.),
                v(0., 1., 1.),
            ],
            triangles: vec![
                [0, 2, 1],
                [0, 3, 2],
                [4, 5, 6],
                [4, 6, 7],
                [0, 1, 5],
                [0, 5, 4],
                [1, 2, 6],
                [1, 6, 5],
                [2, 3, 7],
                [2, 7, 6],
                [3, 0, 4],
                [3, 4, 7],
            ],
        };
        let moved = mesh::BuildItem {
            translation_mm: [10., 0., 0.],
            ..mesh::BuildItem::identity(0)
        };
        let items = [mesh::BuildItem::identity(0), moved];
        let bytes =
            mesh::three_mf(std::slice::from_ref(&cube), &items, LengthUnit::Millimeter).unwrap();
        let object = |x: f64, volume: f64| PrintObject {
            part: EntityId::new(),
            occurrence_path: vec![],
            name: "Cube".into(),
            role: PrintRole::Finished,
            bounds_mm: [[x, 0., 0.], [x + 1., 1., 1.]],
            plate_pose: None,
            volume_mm3: volume,
            triangles: 12,
            mesh_volume_mm3: volume,
        };
        let file = PrintFile {
            kind: PrintFileKind::ThreeMf,
            part: None,
            occurrence_path: vec![],
            suffix: ".3mf".into(),
            objects: 1,
            items: 2,
            triangles: 24,
        };
        let quality = MeshQuality::default();
        let (a, b) = (object(0., 1.), object(10., 1.));
        near(
            verify(&file, &bytes, &[&a, &b], &quality, None).unwrap(),
            2.,
        );
        let positions = [(&cube, &items[0]), (&cube, &items[1])];
        near(
            verify(&file, &bytes, &[&a, &b], &quality, Some(&positions)).unwrap(),
            2.,
        );
        // Rigid movement preserves every volume and count. The arranged
        // receipt must still refuse the altered reopened build item.
        let mut moved_items = items.clone();
        moved_items[1].translation_mm[0] += 0.25;
        let moved_bytes = mesh::three_mf(
            std::slice::from_ref(&cube),
            &moved_items,
            LengthUnit::Millimeter,
        )
        .unwrap();
        assert!(
            verify(&file, &moved_bytes, &[&a, &b], &quality, Some(&positions))
                .unwrap_err()
                .message
                .contains("Readback placement")
        );
        // A placed object whose own volume differs is refused even when the
        // package total would still match.
        let (big, small) = (object(0., 1.5), object(10., 0.5));
        assert!(verify(&file, &bytes, &[&big, &small], &quality, None).is_err());
        // Item and object counts must match the plan.
        let wrong = PrintFile { items: 1, ..file };
        assert!(verify(&wrong, &bytes, &[&a, &b], &quality, None).is_err());
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
    #[test]
    fn package_target_requires_an_absolute_3mf_in_an_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(package_target(Path::new("relative.3mf")).is_err());
        assert!(package_target(&dir.path().join("plate.stl")).is_err());
        assert!(package_target(&dir.path().join("missing/plate.3mf")).is_err());
        let (folder, stem) = package_target(&dir.path().join("plate.3MF")).unwrap();
        assert_eq!((folder.as_path(), stem.as_str()), (dir.path(), "plate"));
    }
    #[test]
    fn worker_is_bounded_and_reports_cancellation() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        let worker = PrintWorker::spawn(move |_, interrupt| {
            let _ = rx.lock().unwrap().recv();
            PrintResult::Preview(if interrupt() {
                Err(failure(ErrorKind::Cancelled, None, "cancelled"))
            } else {
                Err(failure(ErrorKind::Empty, None, "controlled"))
            })
        })
        .unwrap();
        let document = Document::new("Bounded print").unwrap();
        let actor = Actor::local(multicad_core::commands::Channel::Ui);
        let request = PrintRequest::default();
        let ids: Vec<_> = (0..4)
            .map(|_| {
                worker
                    .preview(&document, 0, &request, actor.clone())
                    .unwrap()
            })
            .collect();
        assert_eq!(
            worker
                .preview(&document, 0, &request, actor.clone())
                .unwrap_err()
                .kind,
            ErrorKind::Budget
        );
        assert!(worker.cancel(ids[0]) && worker.cancel(ids[3]));
        for _ in 0..4 {
            tx.send(()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut replies = vec![];
        while replies.len() < 4 {
            assert!(Instant::now() < deadline);
            match worker.poll() {
                Some(r) => replies.push(r),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        assert_eq!(replies.iter().map(|r| r.id).collect::<Vec<_>>(), ids);
        let kinds: Vec<_> = replies
            .iter()
            .map(|r| match &r.result {
                PrintResult::Preview(Err(e)) => e.kind.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ErrorKind::Cancelled,
                ErrorKind::Empty,
                ErrorKind::Empty,
                ErrorKind::Cancelled
            ]
        );
        assert!(!worker.pending());
    }
}
