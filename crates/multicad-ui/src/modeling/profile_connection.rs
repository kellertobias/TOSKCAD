//! Typed profile connection draft. Viewport picks and the Inspector edit the same attachment.
use super::{
    FormStamp, ModelRequest, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent},
};
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityId},
    model::{
        self, BodyInput, MemberEnd, OccurrencePath, ProfileAttachment, ProfileConnectionKind,
        ProfileConnectionRequest, ProfileConnectorAnchor, ProfileGrooveSide, ProfileSection,
    },
    parameters::Unit,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileMemberTarget {
    pub member: BodyInput,
    pub occurrence: OccurrencePath,
    label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileConnectorPick {
    pub index: usize,
    pub target: ProfileMemberTarget,
    pub end: bool,
}

struct AttachmentDraft {
    target: Option<usize>,
    attachment: Option<ProfileAttachment>,
    station: DimensionField,
    length_mm: Option<f64>,
}
impl AttachmentDraft {
    fn new() -> Self {
        Self {
            target: None,
            attachment: None,
            station: DimensionField::from_dimension(&model::length(0.), "mm").expect("length unit"),
            length_mm: None,
        }
    }

    fn resolve(&self, document: &Document) -> Result<ProfileAttachment, String> {
        let mut attachment = self
            .attachment
            .clone()
            .ok_or("Pick a site on each profile member")?;
        if let ProfileConnectorAnchor::Groove { station, .. } = &mut attachment.anchor {
            *station = self.station.resolve(document)?;
        }
        Ok(attachment)
    }
}

pub(super) struct ProfileConnectionPanel {
    stamp: FormStamp,
    name: String,
    kind: ProfileConnectionKind,
    targets: Vec<ProfileMemberTarget>,
    sites: [AttachmentDraft; 2],
    seed: Option<ProfileConnectionRequest>,
    editing: bool,
    pick: Option<ProfileConnectorPick>,
}

fn kind_label(kind: ProfileConnectionKind) -> &'static str {
    match kind {
        ProfileConnectionKind::AngleBracket => "Angle bracket",
        ProfileConnectionKind::JoiningPlate => "Joining plate",
        ProfileConnectionKind::UniversalEnd => "Universal end connector",
    }
}

fn side_label(side: ProfileGrooveSide) -> &'static str {
    match side {
        ProfileGrooveSide::MinX => "Groove −X",
        ProfileGrooveSide::MaxX => "Groove +X",
        ProfileGrooveSide::MinY => "Groove −Y",
        ProfileGrooveSide::MaxY => "Groove +Y",
    }
}

fn end_label(end: MemberEnd) -> &'static str {
    match end {
        MemberEnd::Start => "Member start",
        MemberEnd::End => "Member end",
    }
}

impl ProfileConnectionPanel {
    pub fn new(document: &Document, selected: &BTreeSet<EntityId>) -> Result<Self, String> {
        let targets = model::transfer_occurrence_catalog(document)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter_map(|choice| {
                let producer = model::member_feature(document, choice.component)?;
                let source = model::read_member(document, producer).ok()?;
                let stock = model::read_profile_stock(document, source.stock).ok()?;
                if !ProfileSection::from_stock(&stock)
                    .ok()?
                    .is_item_line_8_40x40()
                {
                    return None;
                }
                let label = std::iter::once(choice.path.root)
                    .chain(choice.path.members.iter().copied())
                    .map(|id| document.get(id).map(|entity| entity.name.as_str()))
                    .collect::<Option<Vec<_>>>()?
                    .join(" / ");
                Some(ProfileMemberTarget {
                    member: BodyInput {
                        body: choice.component,
                        producer,
                    },
                    occurrence: choice.path,
                    label,
                })
            })
            .collect::<Vec<_>>();
        let mut sites = [AttachmentDraft::new(), AttachmentDraft::new()];
        let matching = targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                selected.contains(&target.occurrence.root) || selected.contains(&target.member.body)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.len() == selected.len() && matching.len() <= sites.len() {
            for (site, index) in sites.iter_mut().zip(matching) {
                site.target = Some(index);
            }
        }
        Ok(Self {
            stamp: FormStamp::new(),
            name: "Profile connection".into(),
            kind: ProfileConnectionKind::AngleBracket,
            targets,
            sites,
            seed: None,
            editing: false,
            pick: None,
        })
    }

    pub fn existing(
        document: &Document,
        request: ProfileConnectionRequest,
    ) -> Result<Self, String> {
        let mut panel = Self::new(document, &BTreeSet::new())?;
        panel.name = request.name.clone();
        panel.kind = request.kind;
        for (site, attachment) in panel
            .sites
            .iter_mut()
            .zip([&request.first, &request.second])
        {
            site.target = panel.targets.iter().position(|target| {
                target.member == attachment.member && target.occurrence == attachment.occurrence
            });
            if let ProfileConnectorAnchor::Groove { station, .. } = &attachment.anchor {
                site.station = DimensionField::from_dimension(station, "mm")?;
            }
            site.attachment = Some(attachment.clone());
        }
        panel.seed = Some(request);
        panel.editing = true;
        Ok(panel)
    }

    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Profile connection"
    }
    pub fn picker(&self) -> Option<&ProfileConnectorPick> {
        self.pick.as_ref()
    }

    pub fn capture(
        &mut self,
        pick: &ProfileConnectorPick,
        anchor: ProfileConnectorAnchor,
        length_mm: f64,
    ) -> Result<(), String> {
        if self.pick.as_ref() != Some(pick)
            || pick.end != matches!(anchor, ProfileConnectorAnchor::End { .. })
        {
            return Err("This pick no longer matches the active profile attachment".into());
        }
        let site = &mut self.sites[pick.index];
        if let ProfileConnectorAnchor::Groove { station, .. } = &anchor {
            site.station = DimensionField::from_dimension(station, "mm")?;
        }
        site.attachment = Some(ProfileAttachment {
            member: pick.target.member.clone(),
            occurrence: pick.target.occurrence.clone(),
            anchor,
        });
        site.length_mm = Some(length_mm);
        self.pick = None;
        self.stamp = FormStamp::new();
        self.ensure_seed();
        Ok(())
    }

    fn ensure_seed(&mut self) {
        if self.seed.is_none()
            && let [Some(first), Some(second)] =
                self.sites.each_ref().map(|site| site.attachment.clone())
        {
            self.seed = Some(ProfileConnectionRequest::new(
                self.name.clone(),
                self.kind,
                first,
                second,
            ));
        }
    }

    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<ProfileConnectionRequest, String> {
        if !enabled {
            return Err("Profile connection needs an available native model".into());
        }
        if self.pick.is_some() {
            return Err("Finish the active viewport pick first".into());
        }
        if self.name.trim().is_empty() {
            return Err("Enter a connection name".into());
        }
        let first = self.sites[0].resolve(document)?;
        let second = self.sites[1].resolve(document)?;
        if first.occurrence == second.occurrence {
            return Err("Choose two distinct physical profile members".into());
        }
        let mut request = self
            .seed
            .clone()
            .ok_or("Pick a site on each profile member")?;
        request.name = self.name.trim().into();
        request.kind = self.kind;
        request.first = first;
        request.second = second;
        Ok(request)
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong(if self.editing {
            "Edit profile connection"
        } else {
            "Connect aluminum profiles"
        });
        form::preview_status(ui, self.stamp, preview);
        ui.small("Supported section: item Line 8, 40 × 40 mm. Preview checks the chosen connector, mating profile sites and required hardware.");
        ui.label("Connection name");
        let mut changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.name)
                    .id(ui.make_persistent_id("profile-connection-name"))
                    .desired_width(controls::inspector_width(ui)),
            )
            .changed();
        ui.label("Connector family");
        let previous_kind = self.kind;
        ui.add_enabled_ui(!self.editing, |ui| {
            controls::inspector_combo(ui, "profile-connection-kind")
                .selected_text(kind_label(self.kind))
                .show_ui(ui, |ui| {
                    for kind in [
                        ProfileConnectionKind::AngleBracket,
                        ProfileConnectionKind::JoiningPlate,
                        ProfileConnectionKind::UniversalEnd,
                    ] {
                        ui.selectable_value(&mut self.kind, kind, kind_label(kind));
                    }
                });
        });
        ui.small(format!(
            "item article {} · {} modeled components",
            self.kind.article(),
            self.kind.roles().len()
        ));
        if self.kind != ProfileConnectionKind::UniversalEnd {
            ui.small("Fasteners: item 0.0.680.96 countersunk screw and T-slot nut sets.");
        }
        if self.editing {
            ui.small("A saved connection keeps its connector family.");
        }
        if previous_kind != self.kind {
            let was_end = previous_kind == ProfileConnectionKind::UniversalEnd;
            let needs_end = self.kind == ProfileConnectionKind::UniversalEnd;
            if was_end != needs_end {
                self.sites[0].attachment = None;
            }
            self.pick = None;
            changed = true;
        }
        ui.small("The selected groove sides and member ends set the connector orientation. Fasteners follow the selected connector family.");
        if self.targets.is_empty() {
            ui.label("Create two supported profile members before connecting them.");
        }
        for index in 0..self.sites.len() {
            let site = &mut self.sites[index];
            let end = index == 0 && self.kind == ProfileConnectionKind::UniversalEnd;
            ui.separator();
            ui.strong(if index == 0 {
                "First member"
            } else {
                "Second member"
            });
            let previous = site.target;
            let label = site.target.map_or("Choose a profile member", |i| {
                self.targets[i].label.as_str()
            });
            ui.add_enabled_ui(!(self.editing && end), |ui| {
                controls::inspector_combo(ui, ("profile-member", index))
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (i, target) in self.targets.iter().enumerate() {
                            ui.selectable_value(&mut site.target, Some(i), &target.label);
                        }
                    });
            });
            if site.target != previous {
                site.attachment = None;
                site.length_mm = None;
                self.pick = None;
                changed = true;
            }
            if site.target.is_none() && site.attachment.is_some() {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "The saved member reference needs repair. Choose the current profile member.",
                );
            }
            let bound = site
                .station
                .resolve(document)
                .is_ok_and(|station| station.expression().is_some());
            let pick_label = if end {
                "Pick member end in viewport"
            } else if bound {
                "Replace bound station with a viewport pick"
            } else {
                "Pick groove and station in viewport"
            };
            if ui
                .add_enabled(
                    enabled && site.target.is_some(),
                    egui::Button::new(pick_label),
                )
                .clicked()
                && let Some(target) = site.target
            {
                self.pick = Some(ProfileConnectorPick {
                    index,
                    target: self.targets[target].clone(),
                    end,
                });
                changed = true;
            }
            if self.pick.as_ref().is_some_and(|pick| pick.index == index) {
                ui.small(if end {
                    "Click the actual profile end face."
                } else {
                    "Click inside the actual T-slot at the connector station."
                });
                if ui.button("Stop picking").clicked() {
                    self.pick = None;
                    changed = true;
                }
            }
            if let Some(attachment) = &mut site.attachment {
                match &mut attachment.anchor {
                    ProfileConnectorAnchor::End { end } => {
                        controls::inspector_combo(ui, ("profile-end", index))
                            .selected_text(end_label(*end))
                            .show_ui(ui, |ui| {
                                for choice in [MemberEnd::Start, MemberEnd::End] {
                                    changed |= ui
                                        .selectable_value(end, choice, end_label(choice))
                                        .changed();
                                }
                            });
                    }
                    ProfileConnectorAnchor::Groove { side, .. } => {
                        controls::inspector_combo(ui, ("profile-side", index))
                            .selected_text(side_label(*side))
                            .show_ui(ui, |ui| {
                                for choice in [
                                    ProfileGrooveSide::MinX,
                                    ProfileGrooveSide::MaxX,
                                    ProfileGrooveSide::MinY,
                                    ProfileGrooveSide::MaxY,
                                ] {
                                    changed |= ui
                                        .selectable_value(side, choice, side_label(choice))
                                        .changed();
                                }
                            });
                        changed |= site.station.show_named(ui, ("profile-station", index), "Station from member start", "Distance along the selected groove. A parameter expression stays bound until explicitly replaced.");
                        if !bound
                            && let Some(length) = site.length_mm
                            && let Ok(mut station) = site
                                .station
                                .resolve(document)
                                .and_then(|value| value.value_in(Unit::Millimeter, None))
                            && ui
                                .add(
                                    egui::Slider::new(&mut station, 0. ..=length).show_value(false),
                                )
                                .on_hover_text("Slide the connector along the selected groove")
                                .changed()
                        {
                            site.station.set_exact_literal(station, "mm");
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.stamp = FormStamp::new();
        }
        let request = self.request(document, enabled);
        if let Err(error) = &request {
            ui.small(error);
        }
        let pending = matches!(preview, PreviewState::Pending { stamp } if stamp == self.stamp);
        match form::actions(
            ui,
            request.is_ok() && !pending,
            changed,
            self.stamp,
            preview,
        ) {
            Some(FormAction::Preview) => request.ok().map(|request| ModelEvent::Preview {
                stamp: self.stamp,
                request: ModelRequest::ProfileConnection(Box::new(request)),
            }),
            Some(FormAction::Apply) => Some(ModelEvent::Apply),
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            None => changed.then_some(ModelEvent::Changed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_and_inspector_station_edit_keep_one_connection_identity() {
        let document = Document::new("Profile connector draft").unwrap();
        let mut panel = ProfileConnectionPanel::new(&document, &BTreeSet::new()).unwrap();
        let targets = std::array::from_fn::<_, 2, _>(|index| {
            let body = EntityId::new();
            ProfileMemberTarget {
                member: BodyInput {
                    body,
                    producer: EntityId::new(),
                },
                occurrence: OccurrencePath {
                    root: body,
                    members: vec![],
                },
                label: format!("Rail {index}"),
            }
        });
        for (index, target) in targets.into_iter().enumerate() {
            let pick = ProfileConnectorPick {
                index,
                target,
                end: false,
            };
            panel.pick = Some(pick.clone());
            let wrong = ProfileConnectorPick {
                end: true,
                ..pick.clone()
            };
            let anchor = ProfileConnectorAnchor::Groove {
                side: ProfileGrooveSide::MaxY,
                station: model::length(15.),
            };
            assert!(panel.capture(&wrong, anchor.clone(), 100.).is_err());
            panel.capture(&pick, anchor, 100.).unwrap();
        }
        let initial = panel.request(&document, true).unwrap();
        assert_eq!(panel.request(&document, true).unwrap(), initial);
        panel.sites[0].station.set_exact_literal(22.5, "mm");
        let edited = panel.request(&document, true).unwrap();
        assert_eq!(edited.connection, initial.connection);
        assert_eq!(edited.first.occurrence, initial.first.occurrence);
        assert_eq!(edited.second, initial.second);
        assert_eq!(
            edited.first.anchor,
            ProfileConnectorAnchor::Groove {
                side: ProfileGrooveSide::MaxY,
                station: model::length(22.5),
            }
        );
        assert_eq!(document.entities().count(), 0);
    }
}
