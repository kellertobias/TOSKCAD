use super::*;
use multicad_core::{
    model::{AssemblyLeafReceipt, CutoutFace, OccurrencePath, SheetFaceFrame},
    references::{AnalyticGeometry, Resolution, TopologyElement, resolve},
};

/// Map a face of a rendered compound back to the exact current local sheet
/// face. The native leaf receipt carries the actual world pose, including
/// nested placements. Aggregate ordinals are never used as source ordinals.
pub(super) fn aggregate_sheet_face(
    document: &Document,
    scene: &Scene,
    hit: &crate::viewport::Hit,
) -> Result<(TopologySnapshot, TopologyKey, OccurrencePath, Transform), String> {
    let aggregate = scene
        .topology_snapshot(hit.instance)
        .and_then(|snapshot| snapshot.elements.into_iter().find(|e| e.key == hit.key))
        .ok_or("The picked face has no native topology")?;
    let Some(AnalyticGeometry::Plane { frame: surface }) = aggregate.geometry else {
        return Err("Choose a planar face on the sheet base or final flange leg".into());
    };
    let root = document
        .get(hit.instance)
        .and_then(|entity| match entity.data {
            EntityData::Instance { component, .. } => Some(component),
            EntityData::Assembly { .. } => Some(entity.id),
            _ => None,
        })
        .ok_or("Only geometry from an active modeling feature can host a sketch")?;
    let mut leaves = scene
        .assembly_evaluation()
        .into_iter()
        .flat_map(|info| &info.leaves)
        .filter(|leaf| {
            leaf.path.root == hit.instance
                && !leaf.path.members.is_empty()
                && member_provenance(document, root, leaf, &aggregate)
        });
    let leaf = leaves
        .next()
        .ok_or("The picked face does not identify an assembly member")?;
    if leaves.next().is_some() {
        return Err("The picked face does not identify one assembly member".into());
    }
    if scene.source_producer(leaf.component) != Some(leaf.producer)
        || !scene
            .sheet_faces()
            .iter()
            .any(|face| face.part == leaf.component)
    {
        return Err("Only a folded sheet member can host a sketch in this assembly".into());
    }
    let transform = Transform::from_rigid_pose(leaf.pose)?;
    let snapshot = scene
        .geometry_snapshot(leaf.producer)
        .ok_or("The sheet member's current producer is unavailable")?;
    let mut candidates = snapshot.elements.iter().filter(|local| {
        local.key.kind == hit.key.kind
            && !local.origins.is_empty()
            && local.origins.is_subset(&aggregate.origins)
            && local.boundary.is_subset(&aggregate.boundary)
            && matches!(local.geometry, Some(AnalyticGeometry::Plane { frame })
                if coplanar(surface, transform.frame(frame)))
    });
    let element = candidates
        .next()
        .ok_or("The picked face does not resolve to a current sheet face")?;
    if candidates.next().is_some() {
        return Err("The picked face does not identify one current sheet face".into());
    }
    let key = element.key;
    Ok((snapshot, key, leaf.path.clone(), transform))
}

fn member_provenance(
    document: &Document,
    mut component: EntityId,
    leaf: &AssemblyLeafReceipt,
    element: &TopologyElement,
) -> bool {
    for member in &leaf.path.members {
        let Some(EntityData::Assembly { instances }) =
            document.get(component).map(|entity| &entity.data)
        else {
            return false;
        };
        if !instances.contains(member)
            || !element.origins.contains(&Origin {
                source: component,
                role: format!("assembly/member/{member}"),
            })
        {
            return false;
        }
        let Some(EntityData::Instance {
            component: next, ..
        }) = document.get(*member).map(|entity| &entity.data)
        else {
            return false;
        };
        component = *next;
    }
    component == leaf.component
}

fn coplanar(a: ReferenceFrame, b: ReferenceFrame) -> bool {
    const TOLERANCE: f64 = 1e-5;
    let normal = a.normal();
    (1. - dot(normal, b.normal()).abs()).abs() < TOLERANCE
        && dot(sub(a.origin_mm, b.origin_mm), normal).abs() < TOLERANCE
}

/// Recognize a canonical face-attached sketch on a sheet. A sheet face that
/// cannot be developed must fail explicitly instead of becoming a plain solid
/// cut. Other sketch supports retain the ordinary Extrude workflow.
pub(crate) fn attached_sheet_face(
    document: &Document,
    sketch: EntityId,
    scene: &Scene,
) -> Result<Option<(EntityId, CutoutFace)>, String> {
    let Some((_, link)) = multicad_core::sketch_regions::supporting_face(document, sketch) else {
        return Ok(None);
    };
    let Some(intent) = &link.geometry else {
        return Ok(None);
    };
    let mut parts = document.entities().filter(|entity| {
        matches!(&entity.data, EntityData::Part { features, .. } if features.contains(&link.target))
            && multicad_core::model::sheet_feature(document, entity.id).is_some()
    });
    let Some(part) = parts.next() else {
        return Ok(None);
    };
    if parts.next().is_some() {
        return Err("The sketch support identifies more than one sheet Part".into());
    }
    if scene.stamp().document != document.id() || !scene.is_full_model() {
        return Err("Wait for the current folded sheet to resolve its sketch support".into());
    }
    let snapshot = scene
        .geometry_snapshot(link.target)
        .ok_or("The sheet sketch's support geometry is unavailable")?;
    let Resolution::Resolved {
        key,
        geometry: Some(AnalyticGeometry::Plane { frame }),
        ..
    } = resolve(intent, &snapshot, scene.stamp().revision)
    else {
        return Err("Repair the sheet sketch's planar face attachment before cutting".into());
    };
    let element = snapshot
        .elements
        .iter()
        .find(|element| element.key == key)
        .ok_or("The sheet sketch's support geometry is unavailable")?;
    let mut matches = scene.sheet_faces().iter().filter(|face| {
        face.part == part.id && supported_plane(face, Transform::default(), element, frame)
    });
    let face = matches
        .next()
        .ok_or("Sketch on the flat base or final flange leg to make a developed sheet cut")?;
    if matches.next().is_some() {
        return Err("The sketch support does not identify one developed sheet face".into());
    }
    Ok(Some((part.id, face.face)))
}

fn supported_plane(
    face: &SheetFaceFrame,
    transform: Transform,
    element: &TopologyElement,
    surface: ReferenceFrame,
) -> bool {
    let flange = match face.face {
        CutoutFace::Base => None,
        CutoutFace::Flange { index } => Some([
            format!("sheet.flange.{index}.leg.a"),
            format!("sheet.flange.{index}.leg.b"),
        ]),
    };
    let origin_matches = element.origins.iter().any(|origin| {
        if origin.source != face.feature {
            return false;
        }
        let role = origin.role.rsplit('/').next().unwrap_or_default();
        match &flange {
            None => matches!(role, "cap.start" | "cap.end"),
            Some(roles) => roles.iter().any(|candidate| candidate == role),
        }
    });
    if !origin_matches {
        return false;
    }
    let normal = surface.normal();
    let through = transform.vector(face.through_mm);
    let signed = dot(through, normal);
    let thickness = dot(through, through).sqrt();
    let origin = transform.point(face.origin_mm);
    let offset = dot(sub(surface.origin_mm, origin), normal);
    const TOLERANCE_MM: f64 = 1e-5;
    (signed.abs() - thickness).abs() < TOLERANCE_MM
        && (offset.abs() < TOLERANCE_MM || (offset - signed).abs() < TOLERANCE_MM)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{NumericTransform, angle, length, prepare_instance_placement_edit},
        project_file,
        sketch::CreationPlane,
    };

    #[test]
    fn bundled_aggregate_picks_base_and_flange_with_local_anchors_and_world_pose() {
        let mut bytes = &include_bytes!("../../../../examples/front-panel.tcd")[..];
        let source = project_file::read(
            &mut bytes,
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let mut session = Session::new(source.project.document).unwrap();
        let driver = session
            .document()
            .entities()
            .find_map(|entity| {
                if entity.name != "Sheet-metal front panel" {
                    return None;
                }
                match entity.data {
                    EntityData::Instance {
                        placement_source, ..
                    } => placement_source,
                    _ => None,
                }
            })
            .expect("The bundled sheet member has a placement driver");
        let moved = prepare_instance_placement_edit(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            driver,
            &NumericTransform {
                translation: [length(24.), length(-12.), length(15.)],
                axis: [0., 0., 1.],
                angle: angle(30.),
                ..Default::default()
            },
        )
        .unwrap();
        session.commit(moved.prepared).unwrap();
        let compiler = multicad_model::Compiler::default();
        let model = compiler
            .compile(session.document(), session.revision())
            .unwrap();
        let mut scene =
            crate::modeling::native_fixture::scene(session.document(), model.snapshot(), 1)
                .unwrap();
        Arc::get_mut(&mut scene)
            .unwrap()
            .set_sheet_faces(model.sheet_authoring_faces(session.document()));
        let base = scene
            .sheet_faces()
            .iter()
            .find(|face| face.face == CutoutFace::Base)
            .unwrap();
        let leaf = scene
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .find(|leaf| {
                leaf.component == base.part
                    && !leaf.path.members.is_empty()
                    && scene.displays_instance(leaf.path.root)
            })
            .expect("Shipped panel is placed inside a visible compound");
        let pose = Transform::from_rigid_pose(leaf.pose).unwrap();
        let root = leaf.path.root;
        assert_ne!(
            pose,
            Transform::default(),
            "test must exercise an actual leaf pose"
        );
        let mut chosen = Vec::new();
        for face in scene
            .sheet_faces()
            .iter()
            .filter(|face| face.part == base.part)
            .take(2)
        {
            let thickness = dot(face.through_mm, face.through_mm).sqrt();
            let normal = pose.vector(face.through_mm.map(|v| v / thickness));
            let context = PlaneContext {
                document: session.document(),
                revision: session.revision(),
                scene: Some(&scene),
                report: None,
                occurrence: None,
                section: None,
            };
            let mut exact = None;
            'pick: for u in [8., 16., 25., 40., 60., 90., 120., 160., 200., 240., 280.] {
                for v in [2., 6., 10., 16., 23., 32., 48., 64., 80.] {
                    let point = pose.point(std::array::from_fn(|axis| {
                        face.origin_mm[axis] + u * face.u[axis] + v * face.v[axis]
                    }));
                    let ray = crate::viewport::Ray::new(
                        std::array::from_fn(|axis| point[axis] - normal[axis] * 0.05),
                        normal,
                    )
                    .unwrap();
                    let Some(hit) = scene.pick_face(ray).filter(|hit| {
                        hit.instance == root && (hit.distance_mm - 0.05).abs() < 1e-4
                    }) else {
                        continue;
                    };
                    let picked =
                        selected_face(context, &hit).expect("visible sheet face must resolve");
                    let PlaneChoice::Face(picked) = picked.choice else {
                        unreachable!()
                    };
                    assert_eq!(picked.producer, face.feature);
                    assert_eq!(picked.occurrence_path.as_ref(), Some(&leaf.path));
                    assert_eq!(picked.occurrence_transform, pose);
                    assert_ne!(picked.producer, hit.geometry);
                    assert_eq!(
                        picked.world_frame.origin,
                        pose.point(picked.local_frame.origin)
                    );
                    exact = Some((picked, face.face));
                    break 'pick;
                }
            }
            chosen.push(exact.expect("find exposed planar sheet face"));
        }
        assert!(chosen.iter().any(|(_, face)| *face == CutoutFace::Base));
        assert!(
            chosen
                .iter()
                .any(|(_, face)| matches!(face, CutoutFace::Flange { .. }))
        );

        // Create each plane against its own current native snapshot. The
        // canonical links never name the display root or its placed member.
        for (picked, expected) in chosen {
            let native = compiler
                .compile(session.document(), session.revision())
                .unwrap();
            let mut current =
                crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 2)
                    .unwrap();
            Arc::get_mut(&mut current)
                .unwrap()
                .set_sheet_faces(native.sheet_authoring_faces(session.document()));
            let snapshot = current.geometry_snapshot(picked.producer).unwrap();
            let created = sketch::prepare_new(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                "Folded face sketch",
                CreationPlane::Face {
                    snapshot: &snapshot,
                    key: picked.key,
                    anchor: Some(&picked.anchor),
                },
                Default::default(),
            )
            .unwrap();
            let sketch = created.sketch;
            session.commit(created.prepared).unwrap();
            let native = compiler
                .compile(session.document(), session.revision())
                .unwrap();
            let mut reopened =
                crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 3)
                    .unwrap();
            Arc::get_mut(&mut reopened)
                .unwrap()
                .set_sheet_faces(native.sheet_authoring_faces(session.document()));
            let (_, anchor) =
                multicad_core::sketch_regions::supporting_face(session.document(), sketch).unwrap();
            assert_eq!(anchor.target, picked.producer);
            assert_eq!(
                attached_sheet_face(session.document(), sketch, &reopened).unwrap(),
                Some((base.part, expected))
            );
            assert_eq!(
                crate::sketch_context::occurrence(session.document(), sketch, &reopened, None),
                Some((root, pose))
            );
        }
    }
}
