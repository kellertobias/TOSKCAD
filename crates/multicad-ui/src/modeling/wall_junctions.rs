//! Explicit joined-end controls over canonical wall junction records.
use crate::{controls, egui};
use multicad_core::{
    commands::{Session, SessionStamp},
    document::{EntityData, EntityId},
    model::{self, WallEnd, WallJunction, WallJunctionStyle},
};
struct Host {
    id: EntityId,
    name: String,
    points: usize,
    closed: bool,
}
#[derive(Default)]
pub(super) struct JunctionEditor {
    hosts: Vec<Host>,
    source: Option<SessionStamp>,
    pub notice: Option<String>,
}
impl JunctionEditor {
    pub fn sync_source(&mut self, session: &Session, branch: EntityId) {
        if self.source == Some(session.stamp()) {
            return;
        }
        self.source = Some(session.stamp());
        self.hosts=model::walls(session.document()).filter(|e|e.id!=branch).filter_map(|entity|{
            let EntityData::Feature {definition,suppressed:false,..}=&entity.data else{return None;};
            let wall=model::WallDefinition::from_definition(definition).ok()?;
            let part=session.document().get(wall.output.body)?;
            if !matches!(&part.data,EntityData::Part {features,..} if features.as_slice()==[entity.id]) {return None;}
            Some(Host {id:entity.id,name:part.name.clone(),points:wall.path.len(),closed:wall.closed})
        }).collect();
        self.hosts
            .sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    }
    /// Align is an explicit local-draft action, returned to the wall path editor.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        junctions: &mut Vec<WallJunction>,
        layered: bool,
        closed: bool,
    ) -> (bool, Option<usize>) {
        if !layered && junctions.is_empty() {
            return (false, None);
        }
        ui.separator();
        ui.strong("Joined ends");
        ui.small("Perpendicular flush-butt T: the host stays continuous and branch layers end at its physical face. No wrapping.");
        ui.small("Align replaces only that path endpoint. The physical end may trim or extend from the reference path, including across an exterior air gap.");
        let mut changed = false;
        let mut align = None;
        let mut remove = None;
        if !layered || closed {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Joined ends need an open layered branch. Remove joins before changing that scope.",
            );
        }
        for (index, junction) in junctions.iter_mut().enumerate() {
            ui.push_id(("wall-junction", index), |ui| {
                controls::inspector_combo(ui, "joined-end")
                    .selected_text(format!("{} end", junction.end.label()))
                    .show_ui(ui, |ui| {
                        for end in [WallEnd::Start, WallEnd::End] {
                            changed |= ui
                                .selectable_value(
                                    &mut junction.end,
                                    end,
                                    format!("{} end", end.label()),
                                )
                                .changed();
                        }
                    });
                let previous = junction.host_wall;
                controls::inspector_combo(ui, "join-host")
                    .selected_text(
                        self.hosts
                            .iter()
                            .find(|h| h.id == junction.host_wall)
                            .map_or("Missing or modified host — repair required", |h| {
                                h.name.as_str()
                            }),
                    )
                    .show_ui(ui, |ui| {
                        for host in &self.hosts {
                            changed |= ui
                                .selectable_value(&mut junction.host_wall, host.id, &host.name)
                                .changed();
                        }
                    });
                if let Some(host) = self.hosts.iter().find(|h| h.id == junction.host_wall) {
                    if previous != junction.host_wall {
                        junction.host_segment = 0;
                        junction.host_point_count = host.points;
                        junction.host_closed = host.closed;
                    }
                    let count = if host.closed {
                        host.points
                    } else {
                        host.points.saturating_sub(1)
                    };
                    let mut shown = junction.host_segment + 1;
                    ui.horizontal(|ui| {
                        ui.label("Host segment");
                        if ui
                            .add(egui::DragValue::new(&mut shown).range(1..=count.max(1)))
                            .changed()
                        {
                            junction.host_segment = shown - 1;
                            junction.host_point_count = host.points;
                            junction.host_closed = host.closed;
                            changed = true;
                        }
                    });
                    if junction.host_point_count != host.points
                        || junction.host_closed != host.closed
                    {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            "Host path changed. Align explicitly to reattach this end.",
                        );
                    }
                    if ui
                        .add_enabled(
                            layered && !closed,
                            egui::Button::new("Align endpoint perpendicular to host"),
                        )
                        .clicked()
                    {
                        junction.host_point_count = host.points;
                        junction.host_closed = host.closed;
                        align = Some(index);
                        changed = true;
                    }
                }
                if ui.button("Remove join").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            junctions.remove(index);
            align = None;
            changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            for end in [WallEnd::Start, WallEnd::End] {
                let enabled = layered
                    && !closed
                    && junctions.len() < 2
                    && !self.hosts.is_empty()
                    && !junctions.iter().any(|j| j.end == end);
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(format!("Add T at {}", end.label())),
                    )
                    .clicked()
                {
                    let host = &self.hosts[0];
                    junctions.push(WallJunction {
                        end,
                        host_wall: host.id,
                        host_segment: 0,
                        host_point_count: host.points,
                        host_closed: host.closed,
                        style: WallJunctionStyle::FlushButt,
                    });
                    changed = true;
                }
            }
        });
        if layered && self.hosts.is_empty() {
            ui.small("Create another direct architectural wall to use as a join host.");
        }
        if changed {
            self.notice = None;
        }
        if let Some(notice) = &self.notice {
            ui.colored_label(egui::Color32::LIGHT_RED, notice);
        }
        (changed, align)
    }
}
