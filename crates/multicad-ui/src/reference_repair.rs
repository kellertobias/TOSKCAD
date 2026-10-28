//! Explicit reference replacement uses the same prepared transaction as MCP.
//! Evaluation-local keys are only UI choices; canonical source stores provenance.
use crate::viewport::{ReferenceEvaluation, SceneStamp};
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    references::{
        self, Diagnostic, GeometryIntent, Origin, ReferenceAddress, RepairPreview, Resolution,
        TopologyElement, TopologyKey,
    },
};

#[derive(Default)]
pub struct RepairPanel {
    stamp: Option<SceneStamp>,
    address: Option<ReferenceAddress>,
    source: Option<EntityId>,
    selected: Option<TopologyKey>,
    anchor: Option<Origin>,
    inspecting: bool,
    preview: Option<RepairPreview>,
    selection_error: Option<String>,
    message: Option<(bool, String)>,
}

fn name(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| format!("Missing source {id}"))
}
fn status(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Resolved { .. } => "Resolved".into(),
        Resolution::Missing => "Referenced geometry was removed".into(),
        Resolution::MissingSource => "Source geometry is unavailable".into(),
        Resolution::Ambiguous { candidates } => {
            format!("Ambiguous · {} candidates", candidates.len())
        }
        Resolution::Stale { snapshot_revision } => {
            format!("Stale geometry · revision {snapshot_revision}")
        }
        Resolution::UnavailableFrame => "Planar attachment frame is unavailable".into(),
        Resolution::Invalid { message } => format!("Invalid reference · {message}"),
    }
}
fn friendly_role(role: &str) -> String {
    let tail = role
        .rsplit('/')
        .find(|part| !part.is_empty() && EntityId::parse(part).is_err())
        .unwrap_or("");
    match tail.to_ascii_lowercase().as_str() {
        "cap.start" => "Start face".into(),
        "cap.end" => "End face".into(),
        "x.min" => "X minimum".into(),
        "x.max" => "X maximum".into(),
        "y.min" => "Y minimum".into(),
        "y.max" => "Y maximum".into(),
        "z.min" => "Z minimum".into(),
        "z.max" => "Z maximum".into(),
        "side" => "Side face".into(),
        _ => {
            // Advanced provenance may contain UUIDs; the compact choice label
            // uses a human description and leaves those identities in details.
            if tail.is_empty()
                || tail.as_bytes().windows(36).any(|window| {
                    std::str::from_utf8(window).is_ok_and(|s| EntityId::parse(s).is_ok())
                })
            {
                return "Geometry".into();
            }
            let words = tail.replace(['.', '_', '-'], " ");
            let mut chars = words.chars();
            chars.next().map_or_else(
                || "Geometry".into(),
                |first| first.to_uppercase().collect::<String>() + chars.as_str(),
            )
        }
    }
}
fn candidate_priority(diagnostic: &Diagnostic, source: EntityId, element: &TopologyElement) -> u8 {
    if source == diagnostic.source
        && matches!(&diagnostic.resolution,Resolution::Ambiguous{candidates} if candidates.contains(&element.key))
    {
        0
    } else if diagnostic.intent.origins.is_subset(&element.origins) {
        1
    } else {
        2
    }
}
fn candidate_label(element: &TopologyElement) -> String {
    let roles = element
        .origins
        .iter()
        .map(|o| friendly_role(&o.role))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(3)
        .collect::<Vec<_>>()
        .join(" / ");
    format!(
        "{:?} {} · {}",
        element.key.kind,
        element.key.ordinal.saturating_add(1),
        roles
    )
}
impl RepairPanel {
    pub fn viewing_candidates(&self) -> bool {
        self.highlight().is_some()
    }
    /// Owner and key refer to the current evaluation only, never a saved index.
    pub fn highlight(&self) -> Option<(EntityId, TopologyKey)> {
        self.inspecting.then_some((self.source?, self.selected?))
    }
    pub fn has_pending(&self) -> bool {
        self.address.is_some()
    }
    pub fn cancel(&mut self) {
        self.address = None;
        self.source = None;
        self.selected = None;
        self.anchor = None;
        self.preview = None;
        self.inspecting = false;
        self.selection_error = None;
        self.message = None;
    }
    fn synchronize(&mut self, session: &Session, evaluation: &ReferenceEvaluation) -> bool {
        if evaluation.stamp.document != session.document().id()
            || evaluation.stamp.revision != session.revision()
            || evaluation.report.revision != session.revision()
        {
            self.cancel();
            self.message = Some((
                true,
                "Wait for current reference geometry before repairing".into(),
            ));
            return false;
        }
        if self.stamp.is_some_and(|old| old != evaluation.stamp) {
            let pending = self.has_pending();
            self.cancel();
            if pending {
                self.message = Some((
                    false,
                    "Geometry changed; the repair draft was discarded".into(),
                ));
            }
        }
        self.stamp = Some(evaluation.stamp);
        true
    }
    fn begin(&mut self, diagnostic: &Diagnostic) {
        self.cancel();
        self.address = Some(diagnostic.address);
        self.source = Some(diagnostic.source);
    }
    fn choose_source(&mut self, source: EntityId) {
        self.source = Some(source);
        self.selected = None;
        self.anchor = None;
        self.preview = None;
        self.selection_error = None;
        self.message = None;
    }
    fn choose_candidate(&mut self, evaluation: &ReferenceEvaluation, key: TopologyKey) {
        self.selected = Some(key);
        self.anchor = None;
        self.preview = None;
        self.selection_error = None;
        self.message = None;
        self.inspecting = true;
        let Some(snapshot) = self.source.and_then(|id| evaluation.snapshots.get(&id)) else {
            self.selection_error = Some("Replacement source geometry is unavailable".into());
            return;
        };
        if let Err(error) = GeometryIntent::capture(snapshot, key) {
            self.selection_error = Some(error);
            return;
        }
        let Some(diagnostic) = self.address.and_then(|a| evaluation.report.get(a)) else {
            self.selection_error = Some("Reference diagnostic is unavailable".into());
            return;
        };
        if let Some(attachment) = &diagnostic.intent.attachment {
            let candidate = snapshot
                .elements
                .iter()
                .find(|e| e.key == key)
                .expect("captured");
            self.anchor = candidate
                .frames
                .iter()
                .find(|f| f.origin == attachment.anchor)
                .map(|f| f.origin.clone())
                .or_else(|| {
                    (candidate.frames.len() == 1).then(|| candidate.frames[0].origin.clone())
                });
        }
    }
    fn prepare(
        &mut self,
        session: &Session,
        evaluation: &ReferenceEvaluation,
    ) -> Result<(), String> {
        if !self.synchronize(session, evaluation) {
            return Err("Reference geometry is stale".into());
        }
        let snapshot = self
            .source
            .and_then(|id| evaluation.snapshots.get(&id))
            .ok_or("Choose a replacement source")?;
        let address = self.address.ok_or("Choose a reference to repair")?;
        if external_owner(session.document(), address.owner) {
            return Err("Use the sketch external-reference tool to solve the consumer before accepting this repair".into());
        }
        let selected = self.selected.ok_or("Choose a replacement candidate")?;
        self.preview = Some(
            references::prepare_repair(
                session,
                session.revision(),
                Actor::local(Channel::Ui),
                address,
                snapshot,
                selected,
                self.anchor.clone(),
            )
            .map_err(|e| e.to_string())?,
        );
        self.message = None;
        Ok(())
    }
    fn commit(
        &mut self,
        session: &mut Session,
        evaluation: &ReferenceEvaluation,
    ) -> Result<(), String> {
        if !self.synchronize(session, evaluation) {
            return Err("Reference geometry is stale".into());
        }
        let preview = self.preview.take().ok_or("Preview the replacement first")?;
        session
            .commit(preview.prepared)
            .map_err(|e| e.to_string())?;
        self.cancel();
        self.message = Some((
            false,
            "Reference repaired · one undoable source change".into(),
        ));
        Ok(())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        evaluation: &ReferenceEvaluation,
    ) {
        let current = self.synchronize(session, evaluation);
        if self
            .address
            .is_some_and(|address| external_owner(session.document(), address.owner))
        {
            self.cancel();
        }
        let unresolved: Vec<_> = evaluation
            .report
            .entries
            .iter()
            .filter(|d| !matches!(d.resolution, Resolution::Resolved { .. }))
            .filter(|d| !external_owner(session.document(), d.address.owner))
            .collect();
        if unresolved.is_empty() && !self.has_pending() {
            return;
        }
        ui.separator();
        ui.strong("Geometry references");
        if let Some((error, message)) = &self.message {
            ui.colored_label(
                if *error {
                    Color32::LIGHT_RED
                } else {
                    Color32::LIGHT_BLUE
                },
                message,
            );
        }
        if !current {
            return;
        }
        if unresolved.is_empty() {
            return;
        }
        ui.colored_label(
            Color32::LIGHT_YELLOW,
            "References need repair. Previous geometry, when available, stays visible.",
        );
        egui::ScrollArea::vertical()
            .id_salt("unresolved-references")
            .max_height(180.)
            .show(ui, |ui| {
                for diagnostic in &unresolved {
                    let label = format!(
                        "{} · {}",
                        name(session.document(), diagnostic.address.owner),
                        status(&diagnostic.resolution)
                    );
                    if ui
                        .selectable_label(self.address == Some(diagnostic.address), label)
                        .on_hover_text(format!(
                            "Source: {}\nRole: {}\nReference: {}",
                            name(session.document(), diagnostic.source),
                            diagnostic.role,
                            diagnostic.address.reference
                        ))
                        .clicked()
                    {
                        self.begin(diagnostic);
                    }
                }
            });
        let Some(diagnostic) = self.address.and_then(|a| evaluation.report.get(a)) else {
            return;
        };
        ui.separator();
        ui.label(format!(
            "Used by: {}",
            name(session.document(), diagnostic.address.owner)
        ));
        ui.label(format!(
            "Original source: {}",
            name(session.document(), diagnostic.source)
        ));
        ui.small(format!(
            "Role: {} · {:?}",
            diagnostic.role, diagnostic.intent.kind
        ));
        ui.collapsing(
            format!("Affected objects · {}", diagnostic.affected.len()),
            |ui| {
                for id in &diagnostic.affected {
                    ui.label(name(session.document(), *id));
                }
            },
        );
        let mut sources: Vec<_> = evaluation
            .snapshots
            .values()
            .filter(|s| {
                s.revision == session.revision()
                    && session.document().get(s.owner).is_some()
                    && s.elements
                        .iter()
                        .any(|e| e.key.kind == diagnostic.intent.kind && !e.origins.is_empty())
            })
            .map(|s| (s.owner, name(session.document(), s.owner)))
            .collect();
        sources.sort_by(|a, b| a.1.cmp(&b.1));
        let mut source = self.source;
        egui::ComboBox::from_id_salt("repair-source")
            .selected_text(
                source
                    .map(|id| name(session.document(), id))
                    .unwrap_or_else(|| "Choose source…".into()),
            )
            .show_ui(ui, |ui| {
                for (id, label) in sources {
                    ui.selectable_value(&mut source, Some(id), label);
                }
            })
            .response
            .on_hover_text("Choose a current source with named replacement geometry");
        if source != self.source
            && let Some(source) = source
        {
            self.choose_source(source);
        }
        let snapshot = self.source.and_then(|id| evaluation.snapshots.get(&id));
        if let Some(snapshot) = snapshot {
            ui.label("Replacement candidates");
            ui.small("Choose the geometry this reference should use.");
            let mut candidates: Vec<_> = snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == diagnostic.intent.kind && !e.origins.is_empty())
                .collect();
            candidates.sort_by_key(|e| (candidate_priority(diagnostic, snapshot.owner, e), e.key));
            egui::ScrollArea::vertical().id_salt("repair-candidates").max_height(200.).show_rows(ui,24.,candidates.len(),|ui,range| {
                for index in range {
                    let element=candidates[index];
                    let label=candidate_label(element);
                    if ui.add(egui::Button::selectable(self.selected==Some(element.key),label).truncate())
                        .on_hover_text("Inspect this candidate; source stays unchanged until a prepared repair is accepted").clicked() {
                        self.choose_candidate(evaluation,element.key);
                    }
                }
            });
            if let Some(candidate) = self
                .selected
                .and_then(|key| snapshot.elements.iter().find(|e| e.key == key))
            {
                ui.checkbox(&mut self.inspecting,"Inspect replacement geometry")
                    .on_hover_text("Temporarily show the current replacement source; the last valid model remains retained");
                ui.collapsing("Candidate provenance", |ui| {
                    for origin in &candidate.origins {
                        ui.small(format!(
                            "{} · {}",
                            name(session.document(), origin.source),
                            origin.role
                        ));
                    }
                    if !candidate.boundary.is_empty() {
                        ui.label("Adjacent source boundaries");
                        for origin in &candidate.boundary {
                            ui.small(format!(
                                "{} · {}",
                                name(session.document(), origin.source),
                                origin.role
                            ));
                        }
                    }
                });
                if diagnostic.intent.attachment.is_some() {
                    let mut anchor = self.anchor.clone();
                    egui::ComboBox::from_id_salt("repair-anchor")
                        .selected_text(anchor.as_ref().map(|o|friendly_role(&o.role)).unwrap_or_else(||"Choose planar frame…".into()))
                        .show_ui(ui,|ui|{for frame in &candidate.frames {ui.selectable_value(&mut anchor,Some(frame.origin.clone()),format!("{} · {}",name(session.document(),frame.origin.source),friendly_role(&frame.origin.role)));}})
                        .response.on_hover_text("Choose the semantic planar frame; authored offsets and rotation are preserved");
                    if anchor != self.anchor {
                        self.anchor = anchor;
                        self.preview = None;
                    }
                }
            }
        } else {
            ui.small("No current topology for this source; choose a named replacement source.");
        }
        if let Some(error) = &self.selection_error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Some(preview) = &self.preview {
            ui.colored_label(
                Color32::LIGHT_BLUE,
                "Replacement prepared · source is unchanged",
            );
            ui.small(format!(
                "{} affected objects · accepting creates one undoable transaction",
                preview.affected.len()
            ));
        }
        use crate::inspector_actions::{self, Button, Owner};
        let clicked = inspector_actions::row(
            ui,
            Owner::Repair,
            self.preview.is_none().then_some(Button::new(
                "Preview replacement",
                self.selected.is_some() && self.selection_error.is_none(),
                "Validate provenance and attachment without modifying source",
            )),
            Button::new(
                "Accept replacement",
                self.preview.is_some(),
                "Commit this explicit replacement through the shared transactional API",
            ),
            Button::new(
                "Cancel repair",
                true,
                "Discard the repair draft and return to the retained model",
            ),
        );
        if clicked.apply
            && let Err(error) = self.commit(session, evaluation)
        {
            self.message = Some((true, error));
        }
        if clicked.preview
            && let Err(error) = self.prepare(session, evaluation)
        {
            self.message = Some((true, error));
        }
        if clicked.cancel {
            self.cancel();
        }
    }
}
fn external_owner(document: &Document, id: EntityId) -> bool {
    matches!(document.get(id).map(|entity|&entity.data),Some(multicad_core::document::EntityData::Extension{definition,..})if definition.type_id==multicad_core::sketch_external::TYPE_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_reference_owners_cannot_use_source_only_generic_repair() {
        let mut fixture = crate::external_references::tests::fixture();
        let scene = crate::external_references::tests::scene(&fixture, 1);
        let selected = crate::external_references::tests::hit(&fixture, &scene, false);
        let snapshots = scene.reference_snapshots();
        let request = multicad_core::sketch_external::CaptureRequest::new(
            fixture.session.document(),
            fixture.session.revision(),
            multicad_core::sketch_external::CaptureInput {
                sketch: fixture.sketch,
                source: fixture.source.feature,
                intent: selected.intent.unwrap(),
                method: multicad_core::sketch_external::Method::Coplanar,
                behavior: multicad_core::sketch_external::Behavior::ReferenceOnly,
            },
            &snapshots,
        )
        .unwrap();
        let prepared = multicad_solver::prepare_external_reference(
            &fixture.session,
            fixture.session.revision(),
            Actor::local(Channel::Ui),
            &request,
            &snapshots,
        )
        .unwrap();
        fixture
            .session
            .commit(prepared.edit.preview.unwrap())
            .unwrap();
        let current = crate::external_references::tests::scene(&fixture, 2);
        let snapshots = current.reference_snapshots();
        let report = references::inspect(
            fixture.session.document(),
            fixture.session.revision(),
            &snapshots,
        );
        let diagnostic = report
            .entries
            .iter()
            .find(|diagnostic| diagnostic.address.owner == request.reference)
            .unwrap()
            .clone();
        let evaluation = ReferenceEvaluation {
            stamp: current.stamp(),
            report,
            snapshots,
            candidate: current,
        };
        let mut panel = RepairPanel::default();
        assert!(panel.synchronize(&fixture.session, &evaluation));
        panel.begin(&diagnostic);
        panel.choose_candidate(&evaluation, selected.key);
        let before = fixture.session.document().clone();
        assert!(
            panel
                .prepare(&fixture.session, &evaluation)
                .unwrap_err()
                .contains("external-reference")
        );
        assert!(panel.preview.is_none());
        assert_eq!(fixture.session.document(), &before);
    }
    use crate::viewport::Scene;
    use multicad_core::document::*;
    use multicad_core::references::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };
    fn feature(name: &str, inputs: Vec<Link>) -> Entity {
        Entity::new(
            name,
            EntityData::Feature {
                definition: Definition {
                    type_id: "repair.test".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs,
                suppressed: false,
            },
        )
    }
    fn frame(z: f64) -> ReferenceFrame {
        ReferenceFrame {
            origin_mm: [0., 0., z],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        }
    }
    fn evaluation(
        session: &Session,
        snapshots: BTreeMap<EntityId, TopologySnapshot>,
        generation: u64,
    ) -> ReferenceEvaluation {
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation,
        };
        ReferenceEvaluation {
            stamp,
            report: references::inspect(session.document(), session.revision(), &snapshots),
            snapshots,
            candidate: Arc::new(Scene::new(stamp, vec![], vec![]).unwrap()),
        }
    }
    fn fixture() -> (Session, ReferenceEvaluation, EntityId) {
        let source = feature("Panel", vec![]);
        let cut = feature("Cut", vec![]);
        let origin = Origin {
            source: source.id,
            role: "cap.end".into(),
        };
        let mut face = TopologyElement {
            geometry: None,
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 0,
            },
            origins: BTreeSet::from([origin.clone()]),
            boundary: BTreeSet::new(),
            frames: vec![AnchoredFrame {
                origin: origin.clone(),
                frame: frame(18.),
            }],
        };
        let mut snapshot = TopologySnapshot {
            owner: source.id,
            revision: 0,
            elements: vec![face.clone()],
        };
        let mut intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: origin,
            offset_mm: [10., 20., 2.],
            angle_rad: 0.,
        });
        let mut link = Link::new(source.id, "face");
        link.geometry = Some(intent);
        let plane = Entity::new(
            "Hole plane",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(link),
            },
        );
        let hole = feature("Dependent hole", vec![Link::new(plane.id, "plane")]);
        let mut document = Document::new("Reference repairs").unwrap();
        for entity in [source, cut.clone(), plane, hole] {
            document.insert(entity).unwrap();
        }
        let session = Session::new(document).unwrap();
        face.boundary.insert(Origin {
            source: cut.id,
            role: "left.wall".into(),
        });
        let mut right = face.clone();
        right.key.ordinal = 1;
        right.boundary = BTreeSet::from([Origin {
            source: cut.id,
            role: "right.wall".into(),
        }]);
        snapshot.elements = vec![face, right];
        let evaluated = evaluation(&session, BTreeMap::from([(snapshot.owner, snapshot)]), 1);
        (session, evaluated, cut.id)
    }
    fn select(panel: &mut RepairPanel, session: &Session, evaluation: &ReferenceEvaluation) {
        assert!(panel.synchronize(session, evaluation));
        panel.begin(&evaluation.report.entries[0]);
        panel.choose_candidate(
            evaluation,
            evaluation.snapshots[&evaluation.report.entries[0].source].elements[1].key,
        );
    }
    #[test]
    fn explicit_preview_cancel_commit_and_undo_preserve_canonical_intent() {
        let (mut session, evaluation, _) = fixture();
        let mut panel = RepairPanel::default();
        let before = session.document().clone();
        select(&mut panel, &session, &evaluation);
        assert!(panel.has_pending());
        assert!(panel.viewing_candidates());
        assert_eq!(panel.highlight().unwrap().1.ordinal, 1);
        panel.prepare(&session, &evaluation).unwrap();
        assert_eq!(session.document(), &before);
        assert!(!session.can_undo());
        panel.cancel();
        assert!(!panel.has_pending());
        assert!(panel.highlight().is_none());
        select(&mut panel, &session, &evaluation);
        panel.prepare(&session, &evaluation).unwrap();
        assert!(
            panel
                .preview
                .as_ref()
                .unwrap()
                .affected
                .iter()
                .any(|id| session.document().get(*id).unwrap().name == "Dependent hole")
        );
        panel.commit(&mut session, &evaluation).unwrap();
        assert_eq!(session.revision(), 1);
        assert!(!panel.has_pending());
        let after = session.document().clone();
        let intent = after
            .get(evaluation.report.entries[0].address.owner)
            .unwrap()
            .references()
            .into_iter()
            .find_map(|l| l.geometry)
            .unwrap();
        assert_eq!(intent.id, evaluation.report.entries[0].intent.id);
        assert_eq!(
            intent.attachment.as_ref().unwrap().offset_mm,
            [10., 20., 2.]
        );
        assert!(
            intent
                .boundary
                .as_ref()
                .unwrap()
                .iter()
                .any(|o| o.role == "right.wall")
        );
        assert_eq!(
            session.activity().next().unwrap().label,
            "Repair geometry reference"
        );
        session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &before);
        session.redo(2, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &after);
    }
    #[test]
    fn a_new_evaluation_or_source_revision_discards_prepared_repairs() {
        let (mut session, evaluated, _) = fixture();
        let mut panel = RepairPanel::default();
        select(&mut panel, &session, &evaluated);
        panel.prepare(&session, &evaluated).unwrap();
        let next = evaluation(&session, evaluated.snapshots.clone(), 2);
        assert!(panel.commit(&mut session, &next).is_err());
        assert_eq!(session.revision(), 0);
        assert!(!panel.has_pending());
        select(&mut panel, &session, &evaluated);
        panel.prepare(&session, &evaluated).unwrap();
        session
            .apply(multicad_core::commands::Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Ui),
                label: "Unrelated source edit".into(),
                edits: vec![Mutation::RenameDocument("Updated".into())],
            })
            .unwrap();
        assert!(panel.commit(&mut session, &evaluated).is_err());
        assert_eq!(session.revision(), 1);
        assert!(!panel.has_pending());
    }
    #[test]
    fn unavailable_original_source_can_be_explicitly_replaced_by_a_named_source() {
        let (mut session, old, cut) = fixture();
        let origin = Origin {
            source: cut,
            role: "new.cap".into(),
        };
        let key = TopologyKey {
            kind: TopologyKind::Face,
            ordinal: 8,
        };
        let snapshot = TopologySnapshot {
            owner: cut,
            revision: 0,
            elements: vec![TopologyElement {
                geometry: None,
                key,
                origins: BTreeSet::from([origin.clone()]),
                boundary: BTreeSet::new(),
                frames: vec![AnchoredFrame {
                    origin: origin.clone(),
                    frame: frame(24.),
                }],
            }],
        };
        let evaluated = evaluation(&session, BTreeMap::from([(cut, snapshot)]), 2);
        assert_eq!(
            evaluated.report.entries[0].resolution,
            Resolution::MissingSource
        );
        let mut panel = RepairPanel::default();
        panel.synchronize(&session, &evaluated);
        panel.begin(&old.report.entries[0]);
        panel.choose_source(cut);
        panel.choose_candidate(&evaluated, key);
        panel.prepare(&session, &evaluated).unwrap();
        panel.commit(&mut session, &evaluated).unwrap();
        let owner = session
            .document()
            .get(evaluated.report.entries[0].address.owner)
            .unwrap();
        let link = owner
            .references()
            .into_iter()
            .find(|l| l.geometry.is_some())
            .unwrap();
        assert_eq!(link.target, cut);
        assert_eq!(
            link.geometry
                .as_ref()
                .unwrap()
                .attachment
                .as_ref()
                .unwrap()
                .anchor,
            origin
        );
    }
    #[test]
    fn wrong_kind_and_indistinguishable_candidates_fail_without_source_changes() {
        let (session, mut evaluated, _) = fixture();
        let source = evaluated.report.entries[0].source;
        let mut panel = RepairPanel::default();
        select(&mut panel, &session, &evaluated);
        panel.choose_candidate(
            &evaluated,
            TopologyKey {
                kind: TopologyKind::Vertex,
                ordinal: 0,
            },
        );
        assert!(panel.selection_error.is_some());
        assert!(panel.prepare(&session, &evaluated).is_err());
        let snapshot = evaluated.snapshots.get_mut(&source).unwrap();
        snapshot.elements[1].boundary = snapshot.elements[0].boundary.clone();
        let key = snapshot.elements[1].key;
        panel.choose_candidate(&evaluated, key);
        assert!(panel.selection_error.is_some());
        assert!(panel.prepare(&session, &evaluated).is_err());
        assert_eq!(session.revision(), 0);
        assert!(!session.can_undo());
    }
    fn ui_frame(
        ctx: &egui::Context,
        panel: &mut RepairPanel,
        session: &mut Session,
        evaluation: &ReferenceEvaluation,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 1200.),
                )),
                events,
                ..Default::default()
            },
            |ui| panel.show(ui, session, evaluation),
        )
    }
    fn text_position(output: &egui::FullOutput, needle: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing UI label: {needle}"))
    }
    fn click(
        ctx: &egui::Context,
        panel: &mut RepairPanel,
        session: &mut Session,
        evaluation: &ReferenceEvaluation,
        point: egui::Pos2,
    ) -> egui::FullOutput {
        ui_frame(
            ctx,
            panel,
            session,
            evaluation,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
        ui_frame(
            ctx,
            panel,
            session,
            evaluation,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
    }
    #[test]
    fn pointer_workflow_requires_explicit_preview_before_accepting_replacement() {
        let (mut session, evaluation, _) = fixture();
        let before = session.document().clone();
        let mut panel = RepairPanel::default();
        let ctx = egui::Context::default();
        let output = ui_frame(&ctx, &mut panel, &mut session, &evaluation, vec![]);
        let position = text_position(&output, "Hole plane · Ambiguous");
        output.drop_without_applying_deltas();
        let output = click(&ctx, &mut panel, &mut session, &evaluation, position);
        let position = text_position(&output, "Face 2 · End face");
        output.drop_without_applying_deltas();
        let output = click(&ctx, &mut panel, &mut session, &evaluation, position);
        assert!(panel.viewing_candidates());
        assert!(panel.preview.is_none());
        let position = text_position(&output, "Preview replacement");
        output.drop_without_applying_deltas();
        let output = click(&ctx, &mut panel, &mut session, &evaluation, position);
        assert!(panel.preview.is_some());
        assert_eq!(session.document(), &before);
        output.drop_without_applying_deltas();
        let output = ui_frame(&ctx, &mut panel, &mut session, &evaluation, vec![]);
        let position = text_position(&output, "Accept replacement");
        output.drop_without_applying_deltas();
        click(&ctx, &mut panel, &mut session, &evaluation, position).drop_without_applying_deltas();
        assert_eq!(session.revision(), 1);
        assert_ne!(session.document(), &before);
        assert_eq!(
            session.activity().next().unwrap().label,
            "Repair geometry reference"
        );
    }
    #[test]
    fn resolved_references_leave_the_inspector_empty() {
        let (mut session, evaluated, _) = fixture();
        let mut snapshots = evaluated.snapshots.clone();
        snapshots.values_mut().for_each(|s| s.elements.truncate(1));
        let resolved = evaluation(&session, snapshots, 2);
        assert!(resolved.report.all_resolved());
        let mut panel = RepairPanel::default();
        select(&mut panel, &session, &evaluated);
        let ctx = egui::Context::default();
        let output = ui_frame(&ctx, &mut panel, &mut session, &resolved, vec![]);
        assert!(!panel.has_pending());
        assert!(
            output
                .shapes
                .iter()
                .all(|s| !matches!(&s.shape, egui::Shape::Text(_))),
            "Resolved references added Inspector content"
        );
        output.drop_without_applying_deltas();
    }
    #[test]
    fn compact_candidates_use_friendly_roles_and_prioritize_original_intent() {
        let (_, evaluation, cut) = fixture();
        let diagnostic = &evaluation.report.entries[0];
        let snapshot = &evaluation.snapshots[&diagnostic.source];
        let ambiguous = snapshot.elements[1].clone();
        let mut matching = ambiguous.clone();
        matching.key.ordinal = 90;
        let mut other = ambiguous.clone();
        other.key.ordinal = 2;
        other.origins = std::collections::BTreeSet::from([Origin {
            source: cut,
            role: format!("side/{cut}/x.min"),
        }]);
        let mut choices = [&other, &matching, &ambiguous];
        choices.sort_by_key(|e| (candidate_priority(diagnostic, snapshot.owner, e), e.key));
        assert_eq!(choices.map(|e| e.key.ordinal), [1, 90, 2]);
        let label = candidate_label(&other);
        assert_eq!(label, "Face 3 · X minimum");
        assert!(!label.contains(&cut.to_string()));
        assert_eq!(friendly_role("cap.start"), "Start face");
        assert_eq!(friendly_role("cap.end"), "End face");
        assert_eq!(friendly_role(&format!("side/{cut}")), "Side face");
        assert_eq!(friendly_role(&format!("role.{cut}.face")), "Geometry");
    }
}
