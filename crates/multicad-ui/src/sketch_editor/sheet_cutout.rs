use super::*;
use crate::viewport::{PickFilter, PickOptions, Scene, SectionPlane, Transform};
use multicad_core::{
    model::{CutoutFace, SheetFaceFrame},
    references::{AnalyticGeometry, ReferenceFrame, TopologyElement},
};

impl SketchEditor {
    pub(crate) fn begin_sheet_cutout(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        part: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let target = self
            .tree_selection
            .first()
            .copied()
            .filter(|id| {
                matches!(
                    session.document().get(*id).map(|entity| &entity.data),
                    Some(EntityData::Instance { component, .. }) if *component == part
                )
            })
            .unwrap_or(part);
        self.model
            .begin_sheet_cutout(session, resources, target, channel)
    }

    pub(super) fn sheet_cutout_viewport(
        &mut self,
        session: &Session,
        scene: Option<&Scene>,
        response: &egui::Response,
    ) {
        let Some((part, target)) = self.model.sheet_cutout_target() else {
            return;
        };
        if response.hovered() {
            response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if !response.clicked() {
            return;
        }
        let picked = scene
            .filter(|scene| {
                let stamp = scene.stamp();
                scene.is_full_model()
                    && stamp.document == session.document().id()
                    && stamp.revision == session.revision()
            })
            .zip(response.interact_pointer_pos())
            .ok_or("Wait for the current folded model")
            .and_then(|(scene, pointer)| {
                pick(
                    session.document(),
                    scene,
                    self.camera,
                    response.rect,
                    pointer,
                    self.modeling_section(),
                    (part, target),
                )
            });
        match picked {
            Ok((face, uv)) if self.model.pick_sheet_cutout(face, uv) => {
                self.message =
                    "Cutout center chosen · adjust its dimensions in the Inspector".into();
                response.ctx.request_repaint();
            }
            Ok(_) => {}
            Err(error) => self.message = error.into(),
        }
    }
}

fn pick(
    document: &Document,
    scene: &Scene,
    camera: Camera,
    rect: Rect,
    pointer: Pos2,
    section: Option<SectionPlane>,
    (part, target): (EntityId, EntityId),
) -> Result<(CutoutFace, [f64; 2]), &'static str> {
    let hit = scene
        .pick_screen_with(
            camera,
            rect,
            pointer,
            PickOptions {
                filter: PickFilter::Faces,
                include_hidden: false,
                section,
            },
        )
        .ok_or("Click a visible planar face on the selected sheet")?;
    let topology = scene
        .topology_snapshot(hit.instance)
        .ok_or("The picked face has no native topology")?;
    let element = topology
        .elements
        .iter()
        .find(|element| element.key == hit.key)
        .ok_or("The picked face has no native topology")?;
    let Some(AnalyticGeometry::Plane { frame }) = element.geometry else {
        return Err("Pick the flat base or final flange leg of the selected sheet");
    };
    let direct = hit.instance == part
        || matches!(
            document.get(hit.instance).map(|entity| &entity.data),
            Some(EntityData::Instance { component, .. })
                if *component == part && (target == part || target == hit.instance)
        );
    let transform = if direct {
        scene
            .reference_instances()
            .iter()
            .find(|instance| instance.id == hit.instance)
            .map(|instance| instance.transform)
    } else {
        let mut leaves = scene
            .assembly_evaluation()
            .into_iter()
            .flat_map(|info| &info.leaves)
            .filter(|leaf| {
                leaf.component == part
                    && leaf.path.root == hit.instance
                    && (target == part
                        || leaf.path.root == target
                        || leaf.path.members.contains(&target))
                    && leaf.path.members.last().is_some_and(|member| {
                        let role = format!("assembly/member/{member}");
                        element.origins.iter().any(|origin| origin.role == role)
                    })
            });
        let pose = leaves.next().map(|leaf| leaf.pose);
        if leaves.next().is_some() {
            return Err("Choose one visible occurrence of the sheet in the Model tree");
        }
        pose.and_then(|pose| Transform::from_rigid_pose(pose).ok())
    }
    .ok_or("Pick the visible occurrence of the selected sheet")?;

    let mut faces = scene
        .sheet_faces()
        .iter()
        .filter(|face| face.part == part && supported_plane(face, transform, element, frame));
    let face = faces
        .next()
        .ok_or("Pick the base or final flange leg of the selected sheet")?;
    if faces.next().is_some() {
        return Err("The picked face does not resolve to one sheet face");
    }
    let normal = frame.normal();
    let ray = camera
        .ray(pointer, rect)
        .ok_or("Cannot project the selected point")?;
    let divisor = dot(ray.direction(), normal);
    if divisor.abs() < 1e-8 {
        return Err("Rotate the view to click this face more directly");
    }
    let distance = dot(sub(frame.origin_mm, ray.origin()), normal) / divisor;
    let delta = sub(ray.point(distance), transform.point(face.origin_mm));
    Ok((
        face.face,
        [
            dot(delta, transform.vector(face.u)),
            dot(delta, transform.vector(face.v)),
        ],
    ))
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
