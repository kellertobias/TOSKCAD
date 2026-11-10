use super::*;
use crate::modeling::{AdvancedKind, AdvancedSelection};
use multicad_core::model::{ProfileSource, SketchPathSource};
pub(super) struct AdvancedContext {
    guard: SessionStamp,
    profiles: Vec<ProfileSource>,
    paths: Vec<SketchPathSource>,
}
impl ModelToolController {
    pub fn begin_advanced(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Scene>,
        kind: AdvancedKind,
        selection: AdvancedSelection,
        channel: Channel,
    ) -> Result<(), String> {
        let mut panel = super::super::advanced::AdvancedPanel::new(
            session.document(),
            session.revision(),
            scene,
            kind,
            selection,
        )?;
        if let Some(context) = self
            .advanced_context
            .as_ref()
            .filter(|c| session.validate_stamp(&c.guard).is_ok())
        {
            panel.set_sketch_context(context.profiles.clone(), context.paths.clone());
        }
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Advanced(Box::new(panel)),
        )?;
        if let Some(scene) = scene {
            let result = self.verify_context(session, resources, scene);
            self.context_result(scene.stamp(), &result);
            if let Err(error) = result {
                self.cancel();
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn advanced_active(&self) -> bool {
        matches!(self.form, Some(ModelForm::Advanced(_)))
    }
    pub fn advanced_input(&self) -> Option<&multicad_core::model::BodyInput> {
        match &self.form {
            Some(ModelForm::Advanced(p)) => p.input(),
            _ => None,
        }
    }
    pub fn advanced_captures(&self) -> Option<&[multicad_core::references::GeometryIntent]> {
        match &self.form {
            Some(ModelForm::Advanced(p)) => Some(p.captures()),
            _ => None,
        }
    }
    pub fn advanced_picking(&self) -> bool {
        matches!(&self.form,Some(ModelForm::Advanced(p))if p.picking())
    }
    pub fn set_advanced_captures(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        captures: Vec<multicad_core::references::GeometryIntent>,
    ) -> Result<(), String> {
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err(
                "Face selection requires the complete current canonical native model".into(),
            );
        }
        let Some(ModelForm::Advanced(panel)) = &mut self.form else {
            return Err("Open an advanced modeling feature first".into());
        };
        if panel.set_captures(scene, captures)? {
            self.invalidate();
        }
        Ok(())
    }
    pub fn set_advanced_sketch_context(
        &mut self,
        session: &Session,
        profiles: Vec<ProfileSource>,
        paths: Vec<SketchPathSource>,
    ) -> Result<(), String> {
        if self.advanced_active() {
            let scope = self.scope.as_ref().ok_or("Open advanced controls first")?;
            session
                .validate_stamp(&scope.session)
                .map_err(|_| "Sketch choices belong to a different source session")?;
        }
        if profiles.iter().any(|p| {
            !matches!(
                session.document().get(p.sketch).map(|e| &e.data),
                Some(EntityData::Sketch { .. })
            )
        }) || paths.iter().any(|p| {
            p.curves.is_empty()
                || p.curves.len() > SketchPathSource::MAX_SEGMENTS
                || !matches!(
                    session.document().get(p.sketch).map(|e| &e.data),
                    Some(EntityData::Sketch { .. })
                )
        }) {
            return Err("Sketch choices must belong to this source and paths must contain 1–64 authored lines".into());
        }
        let guard = session.stamp();
        if let Some(ModelForm::Advanced(panel)) = &mut self.form {
            panel.set_sketch_context(profiles.clone(), paths.clone());
        }
        self.advanced_context = Some(AdvancedContext {
            guard,
            profiles,
            paths,
        });
        Ok(())
    }
    pub(super) fn seed_advanced_context(&mut self, session: &Session) {
        if let (Some(ModelForm::Advanced(panel)), Some(context)) =
            (&mut self.form, &self.advanced_context)
            && session.validate_stamp(&context.guard).is_ok()
        {
            panel.set_sketch_context(context.profiles.clone(), context.paths.clone());
        }
    }
    pub(super) fn stop_advanced_picking(&mut self) {
        if let Some(ModelForm::Advanced(panel)) = &mut self.form {
            panel.stop_picking();
        }
    }
}
