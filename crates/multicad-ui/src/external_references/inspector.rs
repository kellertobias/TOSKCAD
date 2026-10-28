use super::{
    controller::{Proposal, Purpose},
    *,
};
use crate::{controls, egui, viewport::PickFilter};
use multicad_core::{
    commands::Session,
    document::{EntityData, EntityId},
    references::{ReferenceAddress, ReferenceReport, Resolution},
    sketch_external::{self, Behavior, Method},
};
impl Controller {
    pub(crate) fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        sketch: EntityId,
        report: Option<&ReferenceReport>,
        enabled: bool,
        resources: &multicad_core::project_file::ProjectResources,
    ) -> Option<Proposal> {
        controls::inspector_layout(ui);
        ui.separator();
        ui.strong("External references");
        let mut action = None;
        if self.active.is_some() {
            let ready = self.ready();
            let mut preview = false;
            let mut cancel = false;
            let mut load_source = None;
            ui.add_enabled_ui(enabled,|ui|{let active=self.active.as_mut().expect("active");
    ui.label(match active.purpose{Purpose::Capture=>"Select source geometry",Purpose::Repair(_)=>"Select replacement geometry"});
    let old=active.method;
    controls::inspector_combo(ui,"external-method").selected_text(method_label(active.method)).show_ui(ui,|ui|{ui.add_enabled_ui(matches!(active.purpose,Purpose::Capture),|ui|{ui.selectable_value(&mut active.method,Method::Coplanar,"Coplanar projection");ui.selectable_value(&mut active.method,Method::Intersect,"Plane intersection");});}).response.on_hover_text("Projection supports coplanar points and finite lines/circular edges. Intersection supports finite straight edges crossing the plane");
    if old!=active.method{active.selected=None;active.request=None;self.error=None;}
    ui.add_enabled_ui(matches!(active.purpose,Purpose::Capture),|ui|{controls::inspector_combo(ui,"external-behavior").selected_text(behavior_label(active.behavior)).show_ui(ui,|ui|{ui.selectable_value(&mut active.behavior,Behavior::ReferenceOnly,"Reference only");ui.selectable_value(&mut active.behavior,Behavior::Derived,"Derived geometry");}).response.on_hover_text("Reference curves are construction aids. Derived curves can form solid profiles. Both remain linked to their source");});
    if active.method==Method::Coplanar{controls::inspector_combo(ui,"external-filter").selected_text(active.filter.label()).show_ui(ui,|ui|{for filter in [PickFilter::Edges,PickFilter::Vertices]{if ui.selectable_value(&mut active.filter,filter,filter.label()).changed(){active.selected=None;active.request=None;}}});}
    ui.checkbox(&mut active.include_hidden,"Pick occluded edges / vertices").on_hover_text("Allow exact geometry behind other visible faces. Explicit source inspection can display a retained source definition");
    let mut inspect=active.inspected_source;
    controls::inspector_combo(ui,"external-inspected-source").selected_text(inspect.and_then(|id|session.document().get(id)).map_or("All visible bodies",|entity|entity.name.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut inspect,None,"All visible bodies");for entity in session.document().entities(){if matches!(entity.data,EntityData::Feature{..})&&(matches!(active.purpose,Purpose::Repair(_))||self.context.as_ref().is_some_and(|context|context.snapshots.contains_key(&entity.id))){ui.selectable_value(&mut inspect,Some(entity.id),&entity.name);}}}).response.on_hover_text("Temporarily inspect a current native source, including retained upstream feature geometry");
    if inspect!=active.inspected_source{active.inspected_source=inspect;active.selected=None;active.request=None;}
    if matches!(active.purpose,Purpose::Repair(_))&&self.native.backend.is_some()&&ui.add_enabled(inspect.is_some()&&!self.native.pending(),egui::Button::new("Load selected source geometry")).on_hover_text("Evaluate only this source, the sketch plane and other required reference sources while the downstream model is broken").clicked(){load_source=inspect;}
    if self.native.pending(){ui.small("Evaluating current reference sources…");}
    if self.context.as_ref().and_then(|c|c.scene.as_ref()).is_some_and(|scene|!scene.is_full_model()){ui.colored_label(egui::Color32::LIGHT_YELLOW,"Reference source view · incomplete model");}
    if let Some(hit)=&active.selected{ui.label(format!("Selected: {}",session.document().get(hit.geometry).map_or("Source",|entity|entity.name.as_str())));ui.small(format!("{:?} · exact current native geometry",hit.key.kind));}else{ui.small("Click a native edge or vertex in the viewport. Source stays unchanged until Preview and Apply.");}
    if active.method==Method::Intersect{ui.small("Finite straight edges only. Face boundaries and ambiguous circular/spline intersections are unsupported.");}
    preview=ui.add_enabled(ready&&active.selected.is_some(),egui::Button::new("Preview reference")).on_hover_text("Resolve the persistent source intent and solve the complete consumer sketch without modifying source").clicked();
    cancel=ui.button("Cancel reference tool").on_hover_text("Drop this selection and leave the document unchanged").clicked();
   });
            if !ready {
                ui.colored_label(egui::Color32::LIGHT_YELLOW,"Current native definition geometry is unavailable. Placed sketch contexts need the occurrence-aware reference adapter.");
            }
            if cancel {
                self.cancel();
            } else if let Some(source) = load_source {
                if let Err(error) = self.request_source(session, source, resources) {
                    self.error = Some(error);
                }
            } else if preview {
                match self.prepare(session, multicad_core::commands::Channel::Ui) {
                    Ok(proposal) => action = Some(proposal),
                    Err(error) => self.error = Some(error),
                }
            }
        } else {
            ui.small("Use Project reference or Intersect plane in the Sketch ribbon to capture exact model geometry.");
            match sketch_external::bindings(session.document(), sketch) {
                Ok(bindings) => {
                    for (index, binding) in bindings.into_iter().enumerate() {
                        ui.separator();
                        ui.label(format!(
                            "{} · {}",
                            index + 1,
                            session
                                .document()
                                .get(binding.source)
                                .map_or("Missing source", |entity| entity.name.as_str())
                        ));
                        ui.small(format!(
                            "{} · {}",
                            method_label(binding.definition.method),
                            behavior_label(binding.definition.behavior)
                        ));
                        let diagnostic = report
                            .filter(|report| report.revision == session.revision())
                            .and_then(|report| {
                                report.get(ReferenceAddress {
                                    owner: binding.reference,
                                    reference: binding.intent.id,
                                })
                            });
                        match diagnostic.map(|d| &d.resolution) {
                            Some(Resolution::Resolved {
                                geometry: Some(_), ..
                            }) => {
                                ui.small("Native source geometry is available");
                            }
                            Some(Resolution::Resolved { geometry: None, .. }) => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_YELLOW,
                                    "Exact analytic source geometry is unsupported",
                                );
                            }
                            Some(other) => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_YELLOW,
                                    format!("Reference needs repair: {other:?}"),
                                );
                            }
                            None => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_YELLOW,
                                    "Awaiting current native reference evaluation",
                                );
                            }
                        }
                        ui.push_id(binding.reference,|ui|ui.add_enabled_ui(enabled,|ui|{
      let mut behavior=binding.definition.behavior;
      controls::inspector_combo(ui,"existing-external-behavior").selected_text(behavior_label(behavior)).show_ui(ui,|ui|{ui.selectable_value(&mut behavior,Behavior::ReferenceOnly,"Reference only");ui.selectable_value(&mut behavior,Behavior::Derived,"Derived geometry");});
      if behavior!=binding.definition.behavior{match self.behavior(session,binding.reference,behavior){Ok(proposal)=>action=Some(proposal),Err(error)=>self.error=Some(error)}}
      ui.horizontal_wrapped(|ui|{
       if ui.add_enabled(self.can_start_repair(session),egui::Button::new("Repair reference")).on_hover_text("Choose a current named source and solve the consumer before Apply").clicked()&&let Err(error)=self.begin_repair(session,binding.reference){self.error=Some(error);}
       if ui.button("Remove reference").on_hover_text("Remove the reference and its output atomically. Dependent geometry or constraints must be removed explicitly first").clicked(){match self.remove(session,binding.reference){Ok(proposal)=>action=Some(proposal),Err(error)=>self.error=Some(error)}}
      });
     }));
                    }
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, error.to_string());
                }
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        action
    }
}
pub(crate) fn method_label(method: Method) -> &'static str {
    match method {
        Method::Coplanar => "Coplanar projection",
        Method::Intersect => "Plane intersection",
    }
}
pub(crate) fn behavior_label(behavior: Behavior) -> &'static str {
    match behavior {
        Behavior::ReferenceOnly => "Reference only",
        Behavior::Derived => "Derived geometry",
    }
}
