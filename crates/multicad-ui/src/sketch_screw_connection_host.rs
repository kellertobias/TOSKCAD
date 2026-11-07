//! Receiving picks use exact predecessor geometry, independently of the
//! connection's view-only section and exploded stack presentation.
use super::*;
use crate::{
    modeling::{HardwareLibraryRequest, ScrewConnectionPick, ScrewConnectionPickKind},
    viewport::{PickFilter, PickOptions, Scene, SceneFeed, SectionPlane, Selection},
};
use multicad_core::{
    project_file::ProjectResources,
    references::{AnalyticGeometry, TopologyKind},
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) struct ConnectionPickView {
    canonical: Arc<Scene>,
    pick: ScrewConnectionPick,
    view: Arc<Scene>,
    camera: Camera,
}

impl SketchEditor {
    pub(crate) fn connection_needs_library(&self) -> bool {
        self.model.connection_needs_library()
    }
    pub(crate) fn connection_library_request(&self) -> Option<HardwareLibraryRequest> {
        self.model.connection_library_request()
    }
    pub(crate) fn accept_connection_library(
        &mut self,
        request: &HardwareLibraryRequest,
        definitions: Result<Vec<multicad_core::symbols::SymbolDefinition>, String>,
    ) -> Result<(), String> {
        self.model.accept_connection_library(request, definitions)
    }

    pub(super) fn connection_pick_scene(
        &mut self,
        session: &Session,
        feed: &SceneFeed,
    ) -> Result<Option<Arc<Scene>>, String> {
        let Some(pick) = self.model.screw_connection_picker() else {
            if let Some(previous) = self.connection_pick_view.take() {
                self.camera = previous.camera;
                self.fit_requested = false;
            }
            return Ok(None);
        };
        let canonical = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
            .ok_or("Wait for the current native receiver before picking")?
            .clone();
        if let Some(cache) = &self.connection_pick_view
            && cache.pick == pick
            && Arc::ptr_eq(&cache.canonical, &canonical)
        {
            return Ok(Some(cache.view.clone()));
        }
        let view = canonical.isolated_source(pick.receiver.producer)?;
        let camera = self
            .connection_pick_view
            .as_ref()
            .map_or(self.camera, |previous| previous.camera);
        self.connection_pick_view = Some(ConnectionPickView {
            canonical,
            pick,
            view: view.clone(),
            camera,
        });
        self.fit_requested = true;
        Ok(Some(view))
    }

    pub(super) fn connection_viewport(
        &mut self,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        rect: Rect,
    ) {
        let Ok(Some(view)) = self.connection_pick_scene(session, feed) else {
            return;
        };
        let Some(cache) = &self.connection_pick_view else {
            return;
        };
        let canonical = cache.canonical.clone();
        let producer = cache.pick.receiver.producer;
        let capture_kind = cache.pick.kind;
        let Some(snapshot) = canonical.geometry_snapshot(producer) else {
            return;
        };
        let hit = response.hover_pos().and_then(|pointer| {
            view.pick_screen_matching(
                self.camera,
                rect,
                pointer,
                PickOptions {
                    filter: PickFilter::Faces,
                    include_hidden: false,
                    section: None,
                },
                |instance, key| {
                    instance == producer
                        && key.kind == TopologyKind::Face
                        && snapshot.elements.iter().any(|element| {
                            element.key == key
                                && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                                && !element.frames.is_empty()
                        })
                },
            )
        });
        self.solid
            .highlight_reference(hit.as_ref().map(|hit| Selection::Face {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            }));
        if !response.clicked() {
            return;
        }
        let Some(hit) = hit else {
            self.message = "Choose a named planar face on this receiving predecessor".into();
            return;
        };
        let result = multicad_core::model::capture_profile_face(&snapshot, hit.key, None).and_then(
            |mut face| {
                if let Some(attachment) = &mut face.intent.attachment
                    && let Some(frame) = snapshot
                        .elements
                        .iter()
                        .find(|element| element.key == hit.key)
                        .and_then(|element| {
                            element
                                .frames
                                .iter()
                                .find(|frame| frame.origin == attachment.anchor)
                        })
                {
                    let delta: [f64; 3] =
                        std::array::from_fn(|i| hit.point_mm[i] - frame.frame.origin_mm[i]);
                    attachment.offset_mm = [
                        delta.iter().zip(frame.frame.u).map(|(a, b)| a * b).sum(),
                        delta.iter().zip(frame.frame.v).map(|(a, b)| a * b).sum(),
                        0.,
                    ];
                }
                self.model
                    .set_connection_capture(session, resources, &canonical, face)
            },
        );
        match result {
            Ok(()) => {
                self.message = match capture_kind {
                    ScrewConnectionPickKind::EntryAxis => "Connection entry face and axis selected · Preview validates the complete connection",
                    ScrewConnectionPickKind::ReceiverEntry => "Receiving entry face captured · Preview follows its current native station",
                    ScrewConnectionPickKind::GripEnd => "Grip end face captured · Preview derives the current native stack grip",
                }.into()
            }
            Err(error) => self.message = error,
        }
    }

    pub(super) fn paint_connection_axis(&self, painter: &egui::Painter, rect: Rect) {
        if self.model.screw_connection_picker().is_some() {
            return;
        }
        let Some(pose) = self.connection_axis_pose else {
            return;
        };
        let project = |point: [f64; 3]| {
            let p = self.camera.project(point.map(|value| value as f32), rect);
            egui::pos2(p[0], p[1])
        };
        let origin = project(pose.translation_mm);
        let end = project(pose.point([0., 0., 25.]));
        if !origin.is_finite() || !end.is_finite() {
            return;
        }
        let color = egui::Color32::from_rgb(245, 190, 70);
        painter.arrow(origin, end - origin, egui::Stroke::new(2., color));
        painter.circle_stroke(origin, 5., egui::Stroke::new(2., color));
        painter.text(
            end,
            egui::Align2::LEFT_BOTTOM,
            "Connection +axis",
            egui::FontId::proportional(12.),
            color,
        );
    }

    pub(super) fn connection_presentation(
        &mut self,
        session: &Session,
        scene: &Arc<Scene>,
    ) -> Result<Arc<Scene>, String> {
        self.connection_section = None;
        self.connection_axis_pose = None;
        let Some((section, explode)) = self.model.connection_view_options() else {
            self.fit_requested |= self.connection_exploded_axis.take().is_some();
            return Ok(scene.clone());
        };
        if self.model.screw_connection_picker().is_some() {
            return Ok(scene.clone());
        }
        let Some(axis) = self.model.connection_axis() else {
            return Ok(scene.clone());
        };
        let pose = axis
            .source
            .resolve(
                session.document(),
                session.parameters(),
                &axis.face,
                session.revision(),
                &scene.reference_snapshots(),
            )
            .map_err(|error| error.to_string())?;
        self.connection_axis_pose = Some(pose);
        if section {
            let normal = pose.vector([1., 0., 0.]);
            let offset = normal
                .into_iter()
                .zip(pose.translation_mm)
                .map(|(a, b)| a * b)
                .sum();
            self.connection_section = Some(SectionPlane::new(normal, offset)?);
        }
        if !explode {
            self.fit_requested |= self.connection_exploded_axis.take().is_some();
            return Ok(scene.clone());
        }
        let roles = self.model.connection_hardware_roles();
        let Some(receipt) = scene.assembly_evaluation() else {
            return Ok(scene.clone());
        };
        let exploded_stations = exploded_role_stations(&roles);
        let mut offsets = BTreeMap::new();
        for leaf in &receipt.leaves {
            if let Some((id, _)) = roles
                .iter()
                .find(|(id, _)| leaf.path.root == *id || leaf.path.members.contains(id))
            {
                offsets.insert(
                    leaf.path.clone(),
                    pose.vector([0., 0., exploded_stations[id]]),
                );
            }
        }
        if offsets.is_empty() {
            return Ok(scene.clone());
        }
        let roots = offsets
            .keys()
            .map(|path| path.root)
            .collect::<std::collections::BTreeSet<_>>();
        for leaf in &receipt.leaves {
            if roots.contains(&leaf.path.root) {
                offsets.entry(leaf.path.clone()).or_insert([0.; 3]);
            }
        }
        let presented = scene.presentation_leaf_offsets(&offsets, &BTreeMap::new())?;
        if self.connection_exploded_axis != Some(axis.id) {
            self.connection_exploded_axis = Some(axis.id);
            self.fit_requested = true;
        }
        Ok(presented)
    }
}

/// View-only offsets retain actual role order on each side of the receiving stack.
fn exploded_role_stations(
    roles: &[(EntityId, multicad_core::model::ScrewConnectionHardwareRole)],
) -> BTreeMap<EntityId, f64> {
    use multicad_core::model::ScrewConnectionHardwareRole as Role;
    let head_count = roles
        .iter()
        .filter(|(_, role)| *role == Role::HeadWasher)
        .count();
    let nut_count = roles
        .iter()
        .filter(|(_, role)| *role == Role::NutWasher)
        .count();
    let insert_count = usize::from(roles.iter().any(|(_, role)| *role == Role::Insert));
    let hanger = roles.iter().any(|(_, role)| *role == Role::HangerBolt);
    let mut head = 0;
    let mut nut = 0;
    roles
        .iter()
        .map(|(id, role)| {
            let station = match role {
                Role::HangerBolt => -15. * (nut_count + 2) as f64,
                Role::Screw => -15. * (head_count + insert_count + 1) as f64,
                Role::HeadWasher => {
                    let value = -15. * (head_count + insert_count - head) as f64;
                    head += 1;
                    value
                }
                Role::NutWasher => {
                    let offset = if hanger {
                        -15. * (nut_count - nut) as f64
                    } else {
                        15. * (nut + 1) as f64
                    };
                    nut += 1;
                    offset
                }
                Role::Nut => (if hanger { -15. } else { 15. }) * (nut_count + 1) as f64,
                Role::Insert => -15.,
            };
            (*id, station)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exploded_stack_separates_head_and_nut_sides_without_changing_authored_roles() {
        use multicad_core::model::ScrewConnectionHardwareRole as Role;
        let roles = [
            Role::NutWasher,
            Role::HeadWasher,
            Role::Screw,
            Role::HeadWasher,
            Role::NutWasher,
            Role::Nut,
        ]
        .map(|role| (EntityId::new(), role));
        let before = roles;
        let moved = exploded_role_stations(&roles);
        assert!(moved[&roles[2].0] < moved[&roles[1].0] && moved[&roles[1].0] < moved[&roles[3].0]);
        assert!(moved[&roles[3].0] < 0. && moved[&roles[0].0] > 0.);
        assert!(moved[&roles[0].0] < moved[&roles[4].0] && moved[&roles[4].0] < moved[&roles[5].0]);
        assert_eq!(roles, before);
        let hanger_roles = [
            Role::HangerBolt,
            Role::NutWasher,
            Role::NutWasher,
            Role::Nut,
        ]
        .map(|role| (EntityId::new(), role));
        let hanger = exploded_role_stations(&hanger_roles);
        assert!(hanger.values().all(|offset| *offset < 0.));
        assert!(hanger[&hanger_roles[0].0] < hanger[&hanger_roles[3].0]);
        assert!(hanger[&hanger_roles[3].0] < hanger[&hanger_roles[1].0]);
        assert!(hanger[&hanger_roles[1].0] < hanger[&hanger_roles[2].0]);
        let insert_roles =
            [Role::HeadWasher, Role::Screw, Role::Insert].map(|role| (EntityId::new(), role));
        let installed = exploded_role_stations(&insert_roles);
        assert!(installed[&insert_roles[1].0] < installed[&insert_roles[0].0]);
        assert!(
            installed[&insert_roles[0].0] < installed[&insert_roles[2].0]
                && installed[&insert_roles[2].0] < 0.
        );
    }
}
