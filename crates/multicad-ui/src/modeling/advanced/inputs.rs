//! Current solved choices are supplied by the host; authored lists retain order.
use super::*;
use crate::icons::{Icon, mini_button};
#[derive(Default)]
pub(super) struct Inputs {
    pub profiles: Vec<ProfileSource>,
    pub path: Option<SketchPathSource>,
    choices: Vec<ProfileSource>,
    paths: Vec<SketchPathSource>,
}
pub(super) fn profile_label(
    document: &Document,
    p: &ProfileSource,
    choices: &[ProfileSource],
) -> String {
    let ordinal = choices
        .iter()
        .filter(|choice| choice.sketch == p.sketch)
        .position(|choice| choice == p)
        .map_or_else(
            || "saved profile".into(),
            |index| format!("profile {}", index + 1),
        );
    format!(
        "{} · {} ({} boundary curves)",
        document
            .get(p.sketch)
            .map_or("Missing sketch", |e| e.name.as_str()),
        ordinal,
        p.region.outer.curves.len()
    )
}
fn curve_label(document: &Document, sketch: EntityId, id: EntityId) -> String {
    let source = document.get(sketch).and_then(|e| {
        if let multicad_core::document::EntityData::Sketch { definition, .. } = &e.data {
            multicad_core::constraint_sketch::source::Source::from_definition(definition).ok()
        } else {
            None
        }
    });
    source
        .and_then(|source| source.sketch.curves.iter().position(|curve| curve.id == id))
        .map_or_else(
            || format!("Saved curve {}", &id.to_string()[..8]),
            |index| format!("Curve {}", index + 1),
        )
}
pub(super) fn initial_seam(document: &Document, p: &ProfileSource) -> Option<EntityId> {
    // Read identity/creation order only. Geometric choices still come solely
    // from the host's current solved presentations, never authored coordinates.
    let authored = document.get(p.sketch).and_then(|e| {
        if let multicad_core::document::EntityData::Sketch { definition, .. } = &e.data {
            multicad_core::constraint_sketch::source::Source::from_definition(definition).ok()
        } else {
            None
        }
    });
    authored
        .and_then(|s| {
            s.sketch
                .curves
                .iter()
                .find(|c| p.region.outer.curves.contains(&c.id))
                .map(|c| c.id)
        })
        .or_else(|| p.region.outer.curves.first().copied())
}
impl Inputs {
    pub fn new(profiles: Vec<ProfileSource>, path: Option<SketchPathSource>) -> Self {
        Self {
            profiles,
            path,
            ..Default::default()
        }
    }
    pub fn set_context(&mut self, profiles: Vec<ProfileSource>, paths: Vec<SketchPathSource>) {
        self.choices = profiles;
        self.paths = paths;
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        kind: AdvancedKind,
        fields: &mut fields::ParameterFields,
    ) -> bool {
        let mut changed = false;
        if self.choices.is_empty() {
            ui.small("Waiting for current solved closed-profile choices. Saved selections remain visible and editable; Native Preview validates them.");
        }
        let title = if kind == AdvancedKind::Sweep {
            "Section profile"
        } else {
            "Add section (ordered)"
        };
        let mut add = None;
        controls::inspector_combo(ui, "advanced-profile-choice")
            .selected_text(title)
            .show_ui(ui, |ui| {
                for (index, p) in self.choices.iter().enumerate() {
                    let selected = self.profiles.contains(p);
                    if ui
                        .add_enabled(
                            !selected || kind == AdvancedKind::Sweep,
                            egui::Button::new(profile_label(document, p, &self.choices)),
                        )
                        .clicked()
                    {
                        add = Some(index);
                        ui.close();
                    }
                }
            });
        if let Some(index) = add {
            let p = self.choices[index].clone();
            if kind == AdvancedKind::Sweep {
                self.profiles = vec![p];
            } else if !self.profiles.contains(&p) {
                if let fields::ParameterFields::Loft { seams, .. } = fields
                    && let Some(seam) = initial_seam(document, &p)
                {
                    seams.push(seam);
                }
                self.profiles.push(p);
            }
            changed = true;
        }
        let mut remove = None;
        let mut reorder = None;
        for (index, p) in self.profiles.iter().enumerate() {
            ui.push_id(("advanced-section",index),|ui|{
                ui.label(format!("{}. {}",index+1,profile_label(document,p,&self.choices)));
                ui.horizontal(|ui|{if ui.add_enabled_ui(index>0,|ui|mini_button(ui,Icon::ArrowUp,"Move section earlier")).inner.on_hover_text("Move this section earlier; its seam follows").clicked(){reorder=Some((index,index-1));}
if ui.add_enabled_ui(index+1<self.profiles.len(),|ui|mini_button(ui,Icon::ArrowDown,"Move section later")).inner.on_hover_text("Move this section later; its seam follows").clicked(){reorder=Some((index,index+1));}
if ui.button("Remove section").clicked(){remove=Some(index);}});
                if let fields::ParameterFields::Loft{seams,..}=fields
                    && let Some(seam)=seams.get_mut(index){controls::inspector_combo(ui,"section-seam").selected_text(format!("Start: {}",curve_label(document,p.sketch,*seam))).show_ui(ui,|ui|{for curve in &p.region.outer.curves{changed|=ui.selectable_value(seam,*curve,curve_label(document,p.sketch,*curve)).changed();}});ui.small("Start curves are explicit correspondence choices; native Preview rejects incompatible sections.");}

            });
        }
        if let Some(index) = remove {
            self.profiles.remove(index);
            if let fields::ParameterFields::Loft { seams, .. } = fields
                && index < seams.len()
            {
                seams.remove(index);
            }
            changed = true;
        } else if let Some((a, b)) = reorder {
            self.profiles.swap(a, b);
            if let fields::ParameterFields::Loft { seams, .. } = fields
                && a < seams.len()
                && b < seams.len()
            {
                seams.swap(a, b);
            }
            changed = true;
        }
        if kind == AdvancedKind::Sweep {
            changed |= self.show_path(ui, document);
        }
        changed
    }
    fn show_path(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        ui.separator();
        ui.strong("Ordered path segments");
        ui.small("Add connected straight segments from one solved sketch in traversal order. Arcs, splines and closed paths are unsupported.");
        let mut add = None;
        controls::inspector_combo(ui, "advanced-path-choice")
            .selected_text("Add path segment")
            .show_ui(ui, |ui| {
                for (index, p) in self.paths.iter().enumerate() {
                    let valid = p.curves.len() == 1
                        && self.path.as_ref().is_none_or(|selected| {
                            selected.sketch == p.sketch && !selected.curves.contains(&p.curves[0])
                        });
                    let label = format!(
                        "{} · {}",
                        document
                            .get(p.sketch)
                            .map_or("Missing path sketch", |e| e.name.as_str()),
                        p.curves
                            .first()
                            .map_or("No curve".into(), |id| curve_label(document, p.sketch, *id))
                    );
                    if ui.add_enabled(valid, egui::Button::new(label)).clicked() {
                        add = Some(index);
                        ui.close();
                    }
                }
            });
        let mut changed = false;
        if let Some(index) = add {
            let candidate = &self.paths[index];
            if let Some(path) = &mut self.path {
                path.curves.push(candidate.curves[0]);
            } else {
                self.path = Some(candidate.clone());
            }
            changed = true;
        }
        if let Some(path) = &mut self.path {
            let mut remove = None;
            let mut reorder = None;
            for (index, curve) in path.curves.iter().enumerate() {
                ui.push_id(("path-segment", index), |ui| {
                    ui.label(format!(
                        "{}. {}",
                        index + 1,
                        curve_label(document, path.sketch, *curve)
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled_ui(index > 0, |ui| {
                                mini_button(ui, Icon::ArrowUp, "Move segment earlier")
                            })
                            .inner
                            .on_hover_text("Move this segment earlier")
                            .clicked()
                        {
                            reorder = Some((index, index - 1));
                        }
                        if ui
                            .add_enabled_ui(index + 1 < path.curves.len(), |ui| {
                                mini_button(ui, Icon::ArrowDown, "Move segment later")
                            })
                            .inner
                            .on_hover_text("Move this segment later")
                            .clicked()
                        {
                            reorder = Some((index, index + 1));
                        }
                        if ui.button("Remove segment").clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                path.curves.remove(index);
                changed = true;
            } else if let Some((a, b)) = reorder {
                path.curves.swap(a, b);
                changed = true;
            }
            if path.curves.is_empty() {
                self.path = None;
            }
        }
        changed
    }
}
