use super::*;
impl EdgeTreatmentPanel {
    pub(super) fn prune_sides(&mut self) {
        self.sides
            .retain(|s| self.edges.iter().any(|e| e.id == s.edge));
        self.side_faces
            .retain(|f| self.sides.iter().any(|s| s.face == f.id));
        if self
            .face_for
            .is_some_and(|id| !self.edges.iter().any(|e| e.id == id))
        {
            self.face_for = None;
        }
    }
    pub(super) fn set_side_face(
        &mut self,
        scene: &Scene,
        captures: &[GeometryIntent],
    ) -> Result<bool, String> {
        let edge = self
            .face_for
            .ok_or("Choose the edge whose adjacent face should be set")?;
        if captures.len() > 1 {
            return Err("Choose exactly one adjacent face for this edge".into());
        }
        validate_captures(scene, &self.input, captures, TopologyKind::Face)?;
        let before = self.sides.clone();
        self.sides.retain(|s| s.edge != edge);
        if let Some(capture) = captures.first() {
            let snapshot = scene
                .geometry_snapshot(self.input.producer)
                .ok_or("The frozen source topology is unavailable")?;
            let selected = references::resolve(capture, &snapshot, scene.stamp().revision);
            let existing=self.side_faces.iter().find(|other|matches!((&selected,references::resolve(other,&snapshot,scene.stamp().revision)),(Resolution::Resolved{key:a,..},Resolution::Resolved{key:b,..})if *a==b)).map(|c|c.id);
            let face = existing.unwrap_or_else(|| {
                self.side_faces.push(capture.clone());
                capture.id
            });
            self.sides.push(EdgeSide { edge, face });
        }
        // Per-edge selection is complete; another edge needs another explicit choice.
        self.face_for = None;
        self.picking = false;
        self.prune_sides();
        if self.sides == before {
            return Ok(false);
        }
        self.stamp.changed();
        Ok(true)
    }
    pub(super) fn show_edge_sides(&mut self, ui: &mut egui::Ui) -> bool {
        let sided =
            self.profile == EdgeTreatmentProfile::Chamfer && self.mode != ChamferMode::Symmetric;
        if sided {
            ui.small("Choose an explicit adjacent reference face for every edge. Shared faces are allowed; Preview checks adjacency. No side is guessed from face order.");
        }
        let mut remove = None;
        for (index, edge) in self.edges.iter().enumerate() {
            ui.push_id(edge.id,|ui|{
                ui.label(format!("Edge capture {}",index+1)).on_hover_text(format!("Persistent capture {} · upstream producer {}",edge.id,self.input.producer));
                ui.horizontal_wrapped(|ui|{
                    if sided{
                        let bound=self.sides.iter().any(|s|s.edge==edge.id);
                        let label=if self.face_for==Some(edge.id){format!("Choosing face for edge {}…",index+1)}else if bound{format!("Change side face for edge {}",index+1)}else{format!("Choose side face for edge {}",index+1)};
                        if ui.add_enabled(self.picking_available,egui::Button::new(label)).on_hover_text("Explicitly pick this edge's adjacent native face on the frozen upstream Part").clicked(){self.face_for=Some(edge.id);self.picking=false;}
                    }
                    if ui.small_button("Remove").on_hover_text("Remove this edge and its local side-face binding").clicked(){remove=Some(index);}
                });
            });
        }
        if let Some(index) = remove {
            self.edges.remove(index);
            self.prune_sides();
            true
        } else {
            false
        }
    }
}
