use super::*;
use crate::{
    modeling::TerminationViewport,
    viewport::{PickFilter, PickOptions},
};
use multicad_core::references::{AnalyticGeometry, GeometryIntent, TopologyKind};
impl AlignmentPanel {
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        source: ContentHash,
        view: TerminationViewport<'_>,
    ) -> bool {
        let Some(side) = self.picking else {
            return false;
        };
        let result = (|| -> Result<Option<(AlignmentInput, String, Selection)>, String> {
            let scene = view.scene.ok_or("Wait for the current canonical model")?;
            if scene.stamp().document != document.id()
                || scene.stamp().revision != revision
                || scene
                    .model_evaluation()
                    .is_none_or(|info| info.source_hash != source)
            {
                return Err("The visible model is stale or an unapplied candidate; wait for the canonical source".into());
            }
            let Some(pointer) = view.pointer.filter(|p| view.rect.contains(*p)) else {
                return Ok(None);
            };
            let filter = match self.mode {
                AlignmentMode::Frame => PickFilter::Faces,
                AlignmentMode::Axis if self.axis_faces => PickFilter::Faces,
                AlignmentMode::Axis => PickFilter::Edges,
                AlignmentMode::Position => PickFilter::Vertices,
            };
            let source_view = if side == Side::Source {
                if !scene
                    .instances()
                    .iter()
                    .any(|instance| instance.id == self.moving)
                {
                    return Err(
                        "The moving object is hidden; show it before picking its geometry".into(),
                    );
                }
                Some(scene.visible_subset(&BTreeSet::from([self.moving]))?)
            } else {
                None
            };
            let picking_scene = source_view.as_deref().unwrap_or(scene);
            let Some(hit) = picking_scene.pick_screen_with(
                view.camera,
                view.rect,
                pointer,
                PickOptions {
                    filter,
                    include_hidden: false,
                    section: view.section,
                },
            ) else {
                return Ok(None);
            };
            if side == Side::Source && hit.instance != self.moving {
                return Err("Pick geometry on the explicit moving object".into());
            }
            if side == Side::Target && hit.instance == self.moving {
                return Err("Choose a separate target object or explicit datum; self-referencing placement is not allowed".into());
            }
            let (target, snapshot) = if side == Side::Source {
                let mut snapshot = scene
                    .geometry_snapshot(self.producer)
                    .ok_or("Moving component topology is unavailable")?;
                let target = if self.moving == self.component {
                    self.producer
                } else {
                    self.component
                };
                snapshot.owner = target;
                (target, snapshot)
            } else {
                (
                    hit.instance,
                    scene
                        .topology_snapshot(hit.instance)
                        .ok_or("Target world topology is unavailable")?,
                )
            };
            // Reopening a Part alignment displays its later head, but the
            // authored source remains the frozen input. Map only a unique
            // preserved provenance fragment, never an ordinal or nearby face.
            let source_key = if side == Side::Source && hit.geometry != self.producer {
                let displayed = scene
                    .geometry_snapshot(hit.geometry)
                    .ok_or("Displayed source topology is unavailable")?;
                let selected = displayed
                    .elements
                    .iter()
                    .find(|element| element.key == hit.key)
                    .ok_or("Selected source topology is unavailable")?;
                let mut matching = snapshot.elements.iter().filter(|element| {
                    element.key.kind == hit.key.kind
                        && !element.origins.is_empty()
                        && element.origins.is_subset(&selected.origins)
                        && element.boundary.is_subset(&selected.boundary)
                });
                let first=matching.next().ok_or("This displayed element has no preserved frozen-input provenance; inspect or repair the upstream source")?;
                if matching.next().is_some() {
                    return Err("This displayed element maps to multiple frozen-input fragments; explicitly repair or choose another source".into());
                }
                first.key
            } else {
                hit.key
            };
            let element = snapshot
                .elements
                .iter()
                .find(|e| e.key == source_key)
                .ok_or("Selected topology is no longer present")?;
            match (self.mode,element.geometry) {
                (AlignmentMode::Frame,Some(AnalyticGeometry::Plane{..}))=>{},
                (AlignmentMode::Axis,Some(AnalyticGeometry::Line{..}|AnalyticGeometry::CircularArc{..}|AnalyticGeometry::Plane{..}))=>{},
                (AlignmentMode::Position,Some(AnalyticGeometry::Point{..}))=>{},
                _=>return Err("This topology has no supported exact alignment descriptor; choose another element or datum".into()),
            }
            let mut intent = GeometryIntent::capture(&snapshot, source_key)?;
            if self.mode == AlignmentMode::Frame {
                if element.frames.is_empty() {
                    return Err("This planar face has no authored semantic frame. Use normal/axis alignment or a construction datum".into());
                }
                if element.frames.len() == 1 {
                    intent.attachment = Some(Attachment {
                        anchor: element.frames[0].origin.clone(),
                        offset_mm: [0.; 3],
                        angle_rad: 0.,
                    });
                }
            }
            // Preserve the capture identity while hovering the same element.
            if let Some(previous) = match side {
                Side::Source => &self.source,
                Side::Target => &self.target,
            }
            .as_ref()
            .and_then(|a| a.input.as_ref())
                && previous.source == target
                && previous.intent.origins == intent.origins
                && previous.intent.boundary == intent.boundary
                && previous.intent.attachment == intent.attachment
            {
                intent.id = previous.intent.id;
            }
            let selection = match hit.key.kind {
                TopologyKind::Face => Selection::Face {
                    instance: hit.instance,
                    ordinal: hit.key.ordinal,
                },
                TopologyKind::Edge => Selection::Edge {
                    instance: hit.instance,
                    ordinal: hit.key.ordinal,
                },
                TopologyKind::Vertex => Selection::Vertex {
                    instance: hit.instance,
                    ordinal: hit.key.ordinal,
                },
            };
            Ok(Some((
                AlignmentInput {
                    source: target,
                    intent,
                },
                format!(
                    "{} · {:?} {}",
                    document
                        .get(hit.instance)
                        .map_or("Geometry", |e| e.name.as_str()),
                    hit.key.kind,
                    hit.key.ordinal
                ),
                selection,
            )))
        })();
        match result {
            Err(error) => {
                self.highlight = None;
                self.error = Some(error);
                false
            }
            Ok(None) => {
                self.highlight = None;
                false
            }
            Ok(Some((input, label, selection))) => {
                self.highlight = Some(selection);
                self.error = None;
                if !view.clicked {
                    return false;
                }
                if self.mode == AlignmentMode::Frame && input.intent.attachment.is_none() {
                    let scene = view.scene.expect("validated scene");
                    let snapshot = if side == Side::Source {
                        scene.geometry_snapshot(self.producer)
                    } else {
                        scene.topology_snapshot(input.source)
                    };
                    let Some(snapshot) = snapshot else {
                        return false;
                    };
                    let anchors = snapshot
                        .elements
                        .iter()
                        .filter(|e| {
                            e.origins == input.intent.origins
                                && input
                                    .intent
                                    .boundary
                                    .as_ref()
                                    .is_none_or(|b| b == &e.boundary)
                        })
                        .flat_map(|e| e.frames.iter().map(|f| f.origin.clone()))
                        .collect();
                    self.pending = Some(PendingFrame {
                        side,
                        input,
                        anchors,
                        label,
                    });
                    self.picking = None;
                } else {
                    let reference = input.intent.id;
                    self.set_anchor(
                        side,
                        AnchorChoice {
                            anchor: AlignmentAnchor::Geometry { reference },
                            input: Some(input),
                            label,
                        },
                    );
                }
                self.stamp.changed();
                true
            }
        }
    }
}
