//! Extrude / Revolve opened on planar model faces (CAD-146). The same forms,
//! operation controls, gizmo and transactional Apply as sketch profiles; only
//! each output's profile is a persistent face reference.
use super::*;
use multicad_core::model::FaceInput;

impl ModelToolController {
    pub fn begin_face_extrude(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        faces: Vec<FaceInput>,
        channel: Channel,
    ) -> Result<(), String> {
        if faces.is_empty() {
            return Err("Select one or more planar model faces".into());
        }
        let mut request = ExtrudeRequest::from_faces(faces, Default::default());
        let mut names = GeneratedNames::new(session.document());
        for output in &mut request.outputs {
            output.name = names.numbered("Extruded part", " extrusion");
        }
        let mut panel = ExtrudePanel::default();
        panel.begin_request(request)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Extrude(Box::new(panel)),
        )?;
        self.operations = Some(OperationControls::new(session.document(), None));
        self.identities = BooleanIdentities::new(session.document());
        Ok(())
    }
    pub fn begin_face_revolve(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        faces: Vec<FaceInput>,
        channel: Channel,
    ) -> Result<(), String> {
        if faces.is_empty() {
            return Err("Select one or more planar model faces".into());
        }
        let mut request = RevolveRequest::from_faces(faces, Default::default());
        let mut names = GeneratedNames::new(session.document());
        for output in &mut request.outputs {
            output.name = names.numbered("Revolved part", " feature");
        }
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Revolve(Box::new(RevolvePanel::new(request)?.acquire_axis())),
        )
    }
}
