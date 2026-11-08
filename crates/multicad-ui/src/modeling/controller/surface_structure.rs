use super::*;
use crate::{
    modeling::surface_structure::SurfaceStructurePanel,
    viewport::{Hit, SceneReply, SceneScope},
};
use multicad_core::{
    model::SurfaceStructurePattern,
    references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind},
};

pub(super) struct SurfaceInspection {
    stamp: SceneStamp,
    producer: EntityId,
    result: Option<Result<Arc<Scene>, String>>,
}

impl SurfaceInspection {
    pub(super) fn pending(&self) -> bool {
        self.result.is_none()
    }

    pub(super) fn expects(&self, stamp: SceneStamp) -> bool {
        self.pending() && self.stamp == stamp
    }
}

impl ModelToolController {
    pub(crate) fn inspected_surface_scene(&self) -> Option<&Arc<Scene>> {
        self.surface_inspection
            .as_ref()?
            .result
            .as_ref()?
            .as_ref()
            .ok()
    }

    pub(crate) fn inspect_surface_source(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<Option<Arc<Scene>>, String> {
        self.current(session, resources, true)?;
        let producer = self
            .surface_structure_input()
            .ok_or("Wait for the current complete native model before choosing a Part")?
            .producer;
        if let Some(inspection) = &self.surface_inspection
            && inspection.producer == producer
        {
            return inspection.result.clone().transpose();
        }
        self.invalidate();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Surface Structure inspection generation limit reached")?;
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        };
        let submission = self
            .backend
            .as_mut()
            .ok_or("Native source inspection is unavailable")?
            .request_sources(session.document(), resources, stamp, &[producer]);
        self.surface_inspection = Some(SurfaceInspection {
            stamp,
            producer,
            result: submission.err().map(Err),
        });
        self.surface_inspection
            .as_ref()
            .expect("recorded inspection")
            .result
            .clone()
            .transpose()
    }

    pub(super) fn accept_surface_inspection(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        reply: SceneReply,
    ) {
        let inspection = self
            .surface_inspection
            .as_ref()
            .expect("matching inspection");
        let result = (|| {
            let scene = reply.result?;
            self.verify_context(session, resources, &scene)?;
            if scene.stamp() != inspection.stamp
                || scene.scope() != &SceneScope::Sources(vec![inspection.producer])
                || scene.geometry_snapshot(inspection.producer).is_none()
            {
                return Err(
                    "Native inspection did not return the retained Surface Structure producer"
                        .into(),
                );
            }
            Ok(scene)
        })();
        self.surface_inspection
            .as_mut()
            .expect("matching inspection")
            .result = Some(result);
    }

    pub(crate) fn begin_surface_structure(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        hits: &[Hit],
        channel: Channel,
    ) -> Result<(), String> {
        let mut panel = SurfaceStructurePanel::new()?;
        if let Some(first) = hits.first() {
            let input = current_part(session, scene, first.instance)?;
            let faces = hits
                .iter()
                .map(|hit| {
                    if hit.instance != input.body {
                        return Err("Choose up to four faces on the same Part".into());
                    }
                    let hit = scene
                        .refresh_hit(hit)
                        .ok_or("Select current native faces again")?;
                    capture(scene, &input, &hit)
                })
                .collect::<Result<Vec<_>, String>>()?;
            panel.set_faces(scene, input, faces)?;
        }
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::SurfaceStructure(Box::new(panel)),
        )?;
        let result = self.verify_context(session, resources, scene);
        self.context_result(scene.stamp(), &result);
        if result.is_err() {
            self.cancel();
        }
        result
    }

    pub(crate) fn surface_structure_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::SurfaceStructure(panel)) if panel.picking())
    }

    pub(crate) fn surface_structure_input(&self) -> Option<&model::BodyInput> {
        let Some(ModelForm::SurfaceStructure(panel)) = &self.form else {
            return None;
        };
        panel.input()
    }

    pub(crate) fn pick_surface_structure_face(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        canonical: &Scene,
        hit: &Hit,
        additive: bool,
    ) -> Result<(), String> {
        self.verify_context(session, resources, canonical)?;
        let approved_scene = canonical.is_full_model()
            || self
                .inspected_surface_scene()
                .is_some_and(|source| std::ptr::eq(source.as_ref(), canonical));
        let Some(ModelForm::SurfaceStructure(panel)) = &mut self.form else {
            return Err("Open Surface Structure before picking faces".into());
        };
        if !approved_scene || !panel.picking() || hit.stamp != canonical.stamp() {
            return Err("Select the retained source faces on the current model".into());
        }
        let input = match panel.input() {
            Some(input) if hit.instance == input.producer => input.clone(),
            Some(_) => return Err("Choose faces on the retained upstream Part".into()),
            None => current_part(session, canonical, hit.instance)?,
        };
        let face = capture(canonical, &input, hit)?;
        let snapshot = canonical
            .geometry_snapshot(input.producer)
            .ok_or("The retained face source is unavailable")?;
        let existing = panel.faces().iter().position(|intent| {
            matches!(references::resolve(intent, &snapshot, session.revision()),
                Resolution::Resolved { key, .. } if key == hit.key)
        });
        let faces = if additive {
            let mut faces = panel.faces().to_vec();
            if let Some(index) = existing {
                faces.remove(index);
            } else {
                faces.push(face);
            }
            faces
        } else {
            vec![existing.map_or(face, |index| panel.faces()[index].clone())]
        };
        if panel.set_faces(canonical, input, faces)? {
            self.invalidate();
        }
        Ok(())
    }

    pub(super) fn stop_surface_structure_picking(&mut self) {
        if let Some(ModelForm::SurfaceStructure(panel)) = &mut self.form {
            panel.stop_picking();
        }
    }

    pub(crate) fn take_surface_height_map_request(&mut self) -> Option<FormStamp> {
        let Some(ModelForm::SurfaceStructure(panel)) = &mut self.form else {
            return None;
        };
        panel.take_image_request()
    }

    pub(crate) fn accept_surface_height_map(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        stamp: FormStamp,
        pattern: SurfaceStructurePattern,
    ) -> Result<(), String> {
        self.current(session, resources, false)?;
        let Some(ModelForm::SurfaceStructure(panel)) = &mut self.form else {
            return Err("The Surface Structure form closed while the file was read".into());
        };
        if panel.stamp() != stamp {
            return Err(
                "Surface Structure changed while the file was read. Import it again.".into(),
            );
        }
        panel.set_image(pattern)?;
        self.invalidate();
        Ok(())
    }
}

fn current_part(
    session: &Session,
    scene: &Scene,
    body: EntityId,
) -> Result<model::BodyInput, String> {
    super::super::body_choices::choices(session.document(), session.revision(), scene)?
        .into_iter()
        .find(|choice| choice.input.body == body)
        .map(|choice| choice.input)
        .ok_or_else(|| {
            "Choose a visible current Part face; assembly occurrences are unsupported".into()
        })
}

fn capture(scene: &Scene, input: &model::BodyInput, hit: &Hit) -> Result<GeometryIntent, String> {
    if hit.geometry != input.producer || hit.key.kind != TopologyKind::Face || hit.intent.is_none()
    {
        return Err("Choose a named face on the retained source Part".into());
    }
    let snapshot = scene
        .geometry_snapshot(input.producer)
        .ok_or("The retained Part topology is unavailable")?;
    if !snapshot.elements.iter().any(|element| {
        element.key == hit.key
            && element.frames.len() == 1
            && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
    }) {
        return Err("Surface Structure currently supports named planar faces only".into());
    }
    model::capture_profile_face(&snapshot, hit.key, None).map(|face| face.intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{modeling::native_fixture, viewport::Ray};

    #[test]
    fn surface_structure_captures_current_faces_and_repairs_from_frozen_native_source() {
        let mut session = Session::new(Document::new("Surface Structure picks").unwrap()).unwrap();
        let resources = ProjectResources::default();
        let actor = || Actor::local(Channel::Ui);
        let stocks = [0., 60.].map(|x| {
            PrimitiveRequest::new(
                "Stock",
                PrimitiveParameters::Box {
                    size: [30., 24., 18.].map(length),
                    frame: PrimitiveFrame {
                        origin: [x, 0., 0.].map(length),
                        ..Default::default()
                    },
                },
            )
        });
        for stock in &stocks {
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor(), stock).unwrap();
            session.commit(prepared.prepared).unwrap();
        }
        let scene = super::super::chamfer_tests::canonical(&session);
        let top = |x| Ray::new([x, 12., 30.], [0., 0., -1.]).unwrap();
        let first = scene.pick_face(top(15.)).unwrap();
        let other = scene.pick_face(top(75.)).unwrap();
        let side = Ray::new([-10., 12., 9.], [1., 0., 0.]).unwrap();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
        assert!(
            controller
                .begin_surface_structure(
                    &session,
                    &resources,
                    &scene,
                    &[first.clone(), other],
                    Channel::Ui,
                )
                .is_err()
        );
        assert!(!controller.active());
        let second = scene.pick_face(side).unwrap();
        controller
            .begin_surface_structure(
                &session,
                &resources,
                &scene,
                &[first.clone(), second],
                Channel::Ui,
            )
            .unwrap();
        let read_request = |controller: &ModelToolController, session: &Session| {
            let Some(ModelForm::SurfaceStructure(panel)) = &controller.form else {
                panic!("Surface Structure form");
            };
            panel.request(session.document(), true).unwrap()
        };
        let initial = read_request(&controller, &session);
        assert_eq!(initial.input.body, stocks[0].body);
        assert_eq!(initial.faces.len(), 2);
        let stale_image = controller.stamp().unwrap();
        controller.cancel();

        controller
            .begin_surface_structure(&session, &resources, &scene, &[], Channel::Ui)
            .unwrap();
        assert!(controller.surface_structure_input().is_none());
        controller
            .pick_surface_structure_face(&session, &resources, &scene, &first, false)
            .unwrap();
        let mut request = read_request(&controller, &session);
        assert_eq!(request.input, initial.input);
        assert_eq!(request.faces.len(), 1);
        let pattern = SurfaceStructurePattern::HeightMap {
            width: 2,
            height: 1,
            samples: vec![0, 255],
            repeat: [1, 1],
        };
        assert!(
            controller
                .accept_surface_height_map(&session, &resources, stale_image, pattern.clone(),)
                .is_err()
        );
        let stamp = controller.stamp().unwrap();
        controller
            .accept_surface_height_map(&session, &resources, stamp, pattern.clone())
            .unwrap();
        request = read_request(&controller, &session);
        assert_eq!(request.parameters.pattern, pattern);
        assert_eq!(session.revision(), 2);
        assert!(session.document().get(request.feature).is_none());
        controller.cancel();

        // A saved intent can be structurally readable even after its old
        // native provenance disappears. Evaluate only the upstream producer.
        model::prepare_surface_structure(&session, session.revision(), actor(), &request).unwrap();
        let retired = references::Origin {
            source: request.input.producer,
            role: "retired.surface.face".into(),
        };
        request.faces[0].attachment.as_mut().unwrap().anchor = retired.clone();
        request.faces[0].origins = [retired].into();
        let prepared =
            model::prepare_surface_structure(&session, session.revision(), actor(), &request)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        let saved = session.document().clone();
        controller
            .begin_existing(&session, &resources, request.feature, Channel::Ui)
            .unwrap();
        controller.source_only_context("The saved face needs repair");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        super::super::chamfer_tests::click(
            &ctx,
            &mut controller,
            &mut session,
            "Pick or repair faces",
        );
        assert!(controller.surface_structure_picking());
        assert!(
            controller
                .inspect_surface_source(&session, &resources)
                .unwrap()
                .is_none()
        );
        super::super::chamfer_tests::wait(&mut controller, &session);
        let source = controller
            .inspect_surface_source(&session, &resources)
            .unwrap()
            .unwrap();
        assert!(!source.is_full_model());
        let isolated = source.isolated_source(request.input.producer).unwrap();
        let hit = isolated.pick_face(top(15.)).unwrap();
        controller
            .pick_surface_structure_face(&session, &resources, &source, &hit, false)
            .unwrap();
        let repaired = read_request(&controller, &session);
        assert_eq!(repaired.feature, request.feature);
        assert_eq!(repaired.input, request.input);
        assert_eq!(repaired.parameters.pattern, pattern);
        assert_ne!(repaired.faces[0].origins, request.faces[0].origins);
        assert_eq!(session.document(), &saved);
        assert!(!controller.can_apply(&session, &resources));
    }
}
