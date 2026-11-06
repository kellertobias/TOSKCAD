//! Independent head/drive drafts retain their original canonical dimensions.
use super::super::dimension::DimensionField;
use crate::{controls, egui};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::*,
};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum HeadKind {
    SocketCap,
    ExternalHex,
    Pan,
    Button,
    Countersunk,
}
impl HeadKind {
    const ALL: [Self; 5] = [
        Self::SocketCap,
        Self::ExternalHex,
        Self::Pan,
        Self::Button,
        Self::Countersunk,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::SocketCap => "Socket cap",
            Self::ExternalHex => "External hex head",
            Self::Pan => "Pan head",
            Self::Button => "Button head",
            Self::Countersunk => "Countersunk head",
        }
    }
    fn of(source: &ScrewHead) -> Self {
        match source {
            ScrewHead::SocketCap { .. } => Self::SocketCap,
            ScrewHead::ExternalHex { .. } => Self::ExternalHex,
            ScrewHead::Pan { .. } => Self::Pan,
            ScrewHead::Button { .. } => Self::Button,
            ScrewHead::Countersunk { .. } => Self::Countersunk,
        }
    }
    fn defaults(self) -> ScrewHead {
        match self {
            Self::SocketCap => ScrewHead::SocketCap {
                diameter: length(5.68),
                height: length(3.),
            },
            Self::ExternalHex => ScrewHead::ExternalHex {
                across_flats: length(5.5),
                height: length(2.),
            },
            Self::Pan => ScrewHead::Pan {
                profile: Box::new(CrownedScrewHeadParameters {
                    diameter: length(5.6),
                    height: length(2.4),
                    crown_radius: length(5.),
                    top_land_diameter: length(4.),
                    neck_radius: length(0.),
                }),
            },
            Self::Button => ScrewHead::Button {
                profile: Box::new(CrownedScrewHeadParameters {
                    diameter: length(5.7),
                    height: length(1.65),
                    crown_radius: length(3.5),
                    top_land_diameter: length(3.2),
                    neck_radius: length(0.),
                }),
            },
            Self::Countersunk => ScrewHead::Countersunk {
                profile: Box::new(CountersunkScrewHeadParameters {
                    diameter: length(5.5),
                    height: length(1.65),
                    included_angle: degrees(90.),
                    edge_chamfer: length(0.05),
                    neck_radius: length(0.),
                }),
            },
        }
    }
    fn fields(self) -> &'static [FieldSpec] {
        match self {
            Self::SocketCap => &[
                ("/diameter", "Head diameter", "mm"),
                ("/height", "Head height", "mm"),
            ],
            Self::ExternalHex => &[
                ("/across_flats", "Head across flats", "mm"),
                ("/height", "Head height", "mm"),
            ],
            Self::Pan | Self::Button => &[
                ("/diameter", "Head diameter", "mm"),
                ("/height", "Head height", "mm"),
                ("/crown_radius", "Crown radius", "mm"),
                ("/top_land_diameter", "Flat top land diameter", "mm"),
                ("/neck_radius", "Neck blend radius", "mm"),
            ],
            Self::Countersunk => &[
                ("/diameter", "Head diameter", "mm"),
                ("/height", "Head height", "mm"),
                ("/included_angle", "Head included angle", "deg"),
                ("/edge_chamfer", "Top edge chamfer", "mm"),
                ("/neck_radius", "Neck blend radius", "mm"),
            ],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum DriveKind {
    HexSocket,
    ExternalHex,
    Hexalobular,
    PhillipsH,
}
impl DriveKind {
    const ALL: [Self; 4] = [
        Self::HexSocket,
        Self::ExternalHex,
        Self::Hexalobular,
        Self::PhillipsH,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::HexSocket => "Hex socket",
            Self::ExternalHex => "External hex flats",
            Self::Hexalobular => "Hexalobular recess",
            Self::PhillipsH => "Phillips H recess",
        }
    }
    fn of(source: &ScrewDrive) -> Self {
        match source {
            ScrewDrive::HexSocket { .. } => Self::HexSocket,
            ScrewDrive::ExternalHex => Self::ExternalHex,
            ScrewDrive::Hexalobular { .. } => Self::Hexalobular,
            ScrewDrive::PhillipsH { .. } => Self::PhillipsH,
        }
    }
    fn defaults(self, size: u8) -> ScrewDrive {
        match self {
            Self::HexSocket => ScrewDrive::HexSocket {
                across_flats: length(2.55),
                depth: length(1.3),
            },
            Self::ExternalHex => ScrewDrive::ExternalHex,
            Self::Hexalobular => ScrewDrive::Hexalobular {
                profile: Box::new(HexalobularDriveParameters {
                    major_diameter: length(2.8),
                    minor_diameter: length(2.05),
                    outer_lobe_radius: length(0.28),
                    depth: length(1.14),
                    bottom_angle: degrees(120.),
                }),
            },
            Self::PhillipsH => {
                let (m, r, b, g, beta, inner) = match size {
                    0 => (
                        1.6,
                        0.3,
                        0.6,
                        0.81,
                        7.,
                        PhillipsInnerForm::Rounded {
                            radius: length(0.25),
                        },
                    ),
                    2 => (
                        4.3,
                        0.6,
                        1.46,
                        2.29,
                        5.75,
                        PhillipsInnerForm::ShallowV {
                            angle: degrees(140.),
                            e_width: length(0.815),
                        },
                    ),
                    3 => (
                        6.5,
                        0.8,
                        2.4,
                        3.81,
                        5.75,
                        PhillipsInnerForm::ShallowV {
                            angle: degrees(146.),
                            e_width: length(2.005),
                        },
                    ),
                    4 => (
                        8.5,
                        1.,
                        3.47,
                        5.08,
                        7.,
                        PhillipsInnerForm::ShallowV {
                            angle: degrees(153.),
                            e_width: length(2.415),
                        },
                    ),
                    _ => (
                        3.1,
                        0.5,
                        0.96,
                        1.27,
                        7.,
                        PhillipsInnerForm::ShallowV {
                            angle: degrees(138.),
                            e_width: length(0.435),
                        },
                    ),
                };
                ScrewDrive::PhillipsH {
                    profile: Box::new(PhillipsHParameters {
                        size: size_enum(size),
                        mouth_diameter: length(m),
                        mouth_radius: length(r),
                        b_width: length(b),
                        g_diameter: length(g),
                        tilt_angle: degrees(beta),
                        outer_flank_angle: degrees(26.5),
                        floor_angle: degrees(28.),
                        outer_corner_angle: degrees(92.),
                        inner,
                    }),
                }
            }
        }
    }
    fn fields(self, size: u8) -> Vec<FieldSpec> {
        match self {
            Self::HexSocket => vec![
                ("/across_flats", "Socket across flats", "mm"),
                ("/depth", "Socket depth", "mm"),
            ],
            Self::ExternalHex => vec![],
            Self::Hexalobular => vec![
                ("/major_diameter", "Recess major diameter", "mm"),
                ("/minor_diameter", "Recess minor diameter", "mm"),
                ("/outer_lobe_radius", "Outer lobe radius", "mm"),
                ("/depth", "Functional recess depth", "mm"),
                ("/bottom_angle", "Bottom included angle", "deg"),
            ],
            Self::PhillipsH => {
                let mut fields = vec![
                    (
                        "/profile/mouth_diameter",
                        "Theoretical mouth diameter",
                        "mm",
                    ),
                    ("/profile/mouth_radius", "Mouth roundover radius", "mm"),
                    ("/profile/b_width", "B width", "mm"),
                    ("/profile/g_diameter", "G diameter", "mm"),
                    ("/profile/tilt_angle", "Generator tilt angle", "deg"),
                    ("/profile/outer_flank_angle", "Outer flank angle", "deg"),
                    ("/profile/floor_angle", "Floor angle", "deg"),
                    (
                        "/profile/outer_corner_angle",
                        "Outer corner included angle",
                        "deg",
                    ),
                ];
                if size == 0 {
                    fields.push((
                        "/profile/inner/radius",
                        "H0 inner radius in normal C-C section",
                        "mm",
                    ));
                } else {
                    fields.push((
                        "/profile/inner/angle",
                        "Inner included angle in normal C-C section",
                        "deg",
                    ));
                    fields.push(("/profile/inner/e_width", "E width", "mm"));
                }
                fields
            }
        }
    }
}
fn degrees(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "deg".into(),
    }
}
fn size_enum(size: u8) -> PhillipsHSize {
    match size {
        0 => PhillipsHSize::H0,
        2 => PhillipsHSize::H2,
        3 => PhillipsHSize::H3,
        4 => PhillipsHSize::H4,
        _ => PhillipsHSize::H1,
    }
}
fn size_index(size: PhillipsHSize) -> u8 {
    match size {
        PhillipsHSize::H0 => 0,
        PhillipsHSize::H1 => 1,
        PhillipsHSize::H2 => 2,
        PhillipsHSize::H3 => 3,
        PhillipsHSize::H4 => 4,
    }
}
type FieldSpec = (&'static str, &'static str, &'static str);
trait CanonicalShape: Sized {
    fn encoded(&self) -> Result<serde_json::Value, String>;
    fn decoded(value: serde_json::Value) -> Result<Self, String>;
}
macro_rules! canonical_shape {
    ($($ty:ty),+)=>{$(impl CanonicalShape for $ty {
        fn encoded(&self)->Result<serde_json::Value,String> {serde_json::to_value(self).map_err(|e|e.to_string())}
        fn decoded(value:serde_json::Value)->Result<Self,String> {serde_json::from_value(value).map_err(|e|e.to_string())}
    })+};
}
canonical_shape!(ScrewHead, ScrewDrive);
struct ShapeDraft<T> {
    source: T,
    fields: Vec<(FieldSpec, DimensionField)>,
}
impl<T: CanonicalShape> ShapeDraft<T> {
    fn new(source: T, specs: &[FieldSpec]) -> Result<Self, String> {
        let value = source.encoded()?;
        let fields = specs
            .iter()
            .map(|&spec| {
                let dimension: Dimension = serde_json::from_value(
                    value
                        .pointer(spec.0)
                        .ok_or_else(|| format!("Missing profile input {}", spec.0))?
                        .clone(),
                )
                .map_err(|e| e.to_string())?;
                Ok((spec, DimensionField::from_dimension(&dimension, spec.2)?))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { source, fields })
    }
    fn resolve(&self, document: &Document) -> Result<T, String> {
        let mut value = self.source.encoded()?;
        for (spec, field) in &self.fields {
            *value
                .pointer_mut(spec.0)
                .ok_or("Missing canonical profile input")? =
                serde_json::to_value(field.resolve(document)?).map_err(|e| e.to_string())?;
        }
        T::decoded(value)
    }
    fn show(&mut self, ui: &mut egui::Ui, namespace: impl egui::AsIdSalt) -> bool {
        ui.push_id(namespace,|ui|{let mut changed=false;for (spec,field) in &mut self.fields {changed|=field.show_named(ui,spec.0,spec.1,"Retained value or bound expression. Millimeters/degrees are explicit; native Preview validates the complete profile.");}changed}).inner
    }
}
pub(super) struct ScrewShapeForm {
    pub(super) head: HeadKind,
    pub(super) drive: DriveKind,
    phillips_size: u8,
    heads: BTreeMap<HeadKind, ShapeDraft<ScrewHead>>,
    drives: BTreeMap<(DriveKind, u8), ShapeDraft<ScrewDrive>>,
}
impl ScrewShapeForm {
    pub(super) fn new(head: &ScrewHead, drive: &ScrewDrive) -> Result<Self, String> {
        let mut heads = BTreeMap::new();
        for kind in HeadKind::ALL {
            heads.insert(kind, ShapeDraft::new(kind.defaults(), kind.fields())?);
        }
        let mut drives = BTreeMap::new();
        for kind in DriveKind::ALL {
            let sizes: Vec<u8> = if kind == DriveKind::PhillipsH {
                (0..=4).collect()
            } else {
                vec![0]
            };
            for size in sizes {
                drives.insert(
                    (kind, size),
                    ShapeDraft::new(kind.defaults(size), &kind.fields(size))?,
                );
            }
        }
        let h = HeadKind::of(head);
        let d = DriveKind::of(drive);
        let size = if let ScrewDrive::PhillipsH { profile } = drive {
            size_index(profile.size)
        } else {
            1
        };
        heads.insert(h, ShapeDraft::new(head.clone(), h.fields())?);
        drives.insert(
            (d, if d == DriveKind::PhillipsH { size } else { 0 }),
            ShapeDraft::new(drive.clone(), &d.fields(size))?,
        );
        Ok(Self {
            head: h,
            drive: d,
            phillips_size: size,
            heads,
            drives,
        })
    }
    pub(super) fn resolve(&self, document: &Document) -> Result<(ScrewHead, ScrewDrive), String> {
        Ok((
            self.heads[&self.head].resolve(document)?,
            self.drives[&(
                self.drive,
                if self.drive == DriveKind::PhillipsH {
                    self.phillips_size
                } else {
                    0
                },
            )]
                .resolve(document)?,
        ))
    }
    pub(super) fn show(&mut self, ui: &mut egui::Ui, owner: EntityId) -> bool {
        ui.push_id(("screw_shape",owner),|ui|{
   let mut changed=false;ui.label("Head form");
   controls::inspector_combo(ui,"head_kind").selected_text(self.head.label()).show_ui(ui,|ui|for kind in HeadKind::ALL {changed|=ui.selectable_value(&mut self.head,kind,kind.label()).changed();});
   changed|=self.heads.get_mut(&self.head).expect("all head drafts").show(ui,("head",self.head));
   if self.head==HeadKind::Countersunk {ui.small("Neck/construction datum, not a flat bearing seat. Overall length includes the actual head height.");}
   ui.separator();ui.label("Drive form");
   controls::inspector_combo(ui,"drive_kind").selected_text(self.drive.label()).show_ui(ui,|ui|for kind in DriveKind::ALL {changed|=ui.selectable_value(&mut self.drive,kind,kind.label()).changed();});
   if self.drive==DriveKind::PhillipsH {
    controls::inspector_combo(ui,"phillips_size").selected_text(format!("H{}",self.phillips_size)).show_ui(ui,|ui|for size in 0..=4 {changed|=ui.selectable_value(&mut self.phillips_size,size,format!("H{size}")).changed();});
    if self.phillips_size==0 {ui.small("H0 uses a true rounded inner profile in the normal C-C section. E width is derived; no V angle is authored.");}
   }
   let key=(self.drive,if self.drive==DriveKind::PhillipsH {self.phillips_size}else{0});
   changed|=self.drives.get_mut(&key).expect("all drive drafts").show(ui,("drive",key));
   ui.small("Configured engineering geometry. Head and drive are independent; incompatible selections remain until explicitly repaired. Gauge penetration, removed depth and source certification are separate.");
   changed
  }).inner
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{commands::Session, parameters::BoundExpression};
    fn session() -> Session {
        Session::new(Document::new("Screw shape form").unwrap()).unwrap()
    }
    #[test]
    fn every_head_and_h_size_retains_bound_sources_across_kind_roundtrips() {
        let s = session();
        fn bind<T: CanonicalShape>(source: T, doc: &Document) -> T {
            fn walk(value: &mut serde_json::Value, doc: &Document) {
                if value.get("value").is_some_and(serde_json::Value::is_number)
                    && value.get("unit").is_some()
                {
                    let n = value["value"].as_f64().unwrap();
                    let unit = value["unit"].as_str().unwrap();
                    let expression =
                        BoundExpression::bind(doc, &format!("={n}"), unit, None).unwrap();
                    *value = serde_json::to_value(Dimension::Expression { expression }).unwrap();
                    return;
                }
                if let Some(map) = value.as_object_mut() {
                    for child in map.values_mut() {
                        walk(child, doc)
                    }
                }
            }
            let mut value = source.encoded().unwrap();
            walk(&mut value, doc);
            T::decoded(value).unwrap()
        }
        for kind in HeadKind::ALL {
            let head = bind(kind.defaults(), s.document());
            for size in 0..=4 {
                let drive = bind(DriveKind::PhillipsH.defaults(size), s.document());
                let mut form = ScrewShapeForm::new(&head, &drive).unwrap();
                assert_eq!(
                    form.resolve(s.document()).unwrap(),
                    (head.clone(), drive.clone())
                );
                form.head = HeadKind::ExternalHex;
                form.drive = DriveKind::ExternalHex;
                form.phillips_size = (size + 1) % 5;
                form.head = kind;
                form.drive = DriveKind::PhillipsH;
                form.phillips_size = size;
                assert_eq!(form.resolve(s.document()).unwrap(), (head.clone(), drive));
            }
        }
    }
    #[test]
    fn loaded_profiles_unrelated_edit_preserves_exact_fields_length_and_ids() {
        use super::super::MechanicalPartPanel;
        let s = session();
        let before = s.document().clone();
        for size in 0..=4 {
            let head = ScrewHead::Pan {
                profile: Box::new(CrownedScrewHeadParameters {
                    diameter: length(100.),
                    height: length(30.),
                    crown_radius: length(100.),
                    top_land_diameter: length(80.),
                    neck_radius: length(0.),
                }),
            };
            let mut drive = DriveKind::PhillipsH.defaults(size);
            if let ScrewDrive::PhillipsH { profile } = &mut drive {
                profile.mouth_diameter = Dimension::Expression {
                    expression: BoundExpression::bind(
                        s.document(),
                        &format!(
                            "={}",
                            match size {
                                0 => 1.6,
                                1 => 3.1,
                                2 => 4.3,
                                3 => 6.5,
                                _ => 8.5,
                            }
                        ),
                        "mm",
                        None,
                    )
                    .unwrap(),
                };
            }
            let mut parameters = default_machine_screw();
            let MechanicalPartParameters::MachineScrew {
                head: h,
                drive: d,
                underhead_length,
                length_input,
                ..
            } = &mut parameters
            else {
                unreachable!()
            };
            *h = head.clone();
            *d = drive.clone();
            *underhead_length = None;
            *length_input = Some(ScrewLengthInput::Overall {
                value: Dimension::Expression {
                    expression: BoundExpression::bind(s.document(), "=43", "mm", None).unwrap(),
                },
            });
            let request = MechanicalPartRequest::new("Retained H profile", parameters);
            let mut panel =
                MechanicalPartPanel::new(request.clone(), s.parameters().clone(), true).unwrap();
            assert_eq!(panel.request(s.document(), true).unwrap(), request);
            panel.origin[0].set_literal(5., "mm");
            let changed = panel.request(s.document(), true).unwrap();
            assert_eq!(changed.feature, request.feature);
            assert_eq!(changed.body, request.body);
            let MechanicalPartParameters::MachineScrew {
                head: actual_head,
                drive: actual_drive,
                length_input: actual_length,
                underhead_length,
                ..
            } = changed.parameters
            else {
                unreachable!()
            };
            assert_eq!(actual_head, head);
            assert_eq!(actual_drive, drive);
            assert_eq!(underhead_length, None);
            let MechanicalPartParameters::MachineScrew { length_input, .. } = &request.parameters
            else {
                unreachable!()
            };
            assert_eq!(&actual_length, length_input);
            assert_eq!(s.document(), &before);
            assert_eq!(s.revision(), 0);
        }
    }
    fn paint(
        ctx: &egui::Context,
        form: &mut ScrewShapeForm,
        s: &Session,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let (head, drive) = form.resolve(s.document()).unwrap();
        let mut p = default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            head: h, drive: d, ..
        } = &mut p
        else {
            unreachable!()
        };
        *h = head;
        *d = drive;
        let invalid = p.resolve(s.parameters()).err().map(|e| e.to_string());
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(350., 2300.),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                if let Some(error) = &invalid {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                form.show(ui, EntityId::nil());
            },
        )
    }
    fn click(ctx: &egui::Context, form: &mut ScrewShapeForm, s: &Session, label: &str) {
        paint(ctx, form, s, vec![]).drop_without_applying_deltas();
        let output = paint(ctx, form, s, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.job.text == label => {
                    Some(t.pos + t.galley.size() / 2.)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing painted {label}"));
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            paint(
                ctx,
                form,
                s,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }
    #[test]
    fn painted_independent_head_and_h_size_choices_restore_original_sources() {
        let s = session();
        let mut drive = DriveKind::PhillipsH.defaults(1);
        if let ScrewDrive::PhillipsH { profile } = &mut drive {
            profile.mouth_diameter = Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=3.1", "mm", None).unwrap(),
            };
        }
        let head = HeadKind::Pan.defaults();
        let mut form = ScrewShapeForm::new(&head, &drive).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        click(&ctx, &mut form, &s, "Pan head");
        click(&ctx, &mut form, &s, "Countersunk head");
        assert_eq!(form.resolve(s.document()).unwrap().1, drive);
        click(&ctx, &mut form, &s, "Countersunk head");
        click(&ctx, &mut form, &s, "Pan head");
        click(&ctx, &mut form, &s, "H1");
        click(&ctx, &mut form, &s, "H0");
        let output = paint(&ctx, &mut form, &s, vec![]);
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("H0 inner radius in normal C-C section"));
        assert!(!text.contains("E width (mm)"));
        assert!(!text.contains("Inner included angle in normal C-C section (deg)"));
        output.drop_without_applying_deltas();
        click(&ctx, &mut form, &s, "H0");
        click(&ctx, &mut form, &s, "H1");
        assert_eq!(form.resolve(s.document()).unwrap(), (head, drive));
        assert_eq!(s.revision(), 0);
    }
    #[test]
    fn painted_partial_typing_keeps_focus_when_validation_inserts_rows() {
        let s = session();
        let mut form =
            ScrewShapeForm::new(&HeadKind::Pan.defaults(), &DriveKind::HexSocket.defaults(0))
                .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        paint(&ctx, &mut form, &s, vec![]).drop_without_applying_deltas();
        let output = paint(&ctx, &mut form, &s, vec![]);
        let pos = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter(|(_, node)| node.value() == Some("5.6"))
            .find_map(|(_, node)| node.bounds())
            .map(|b| egui::pos2(((b.x0 + b.x1) / 2.) as f32, ((b.y0 + b.y1) / 2.) as f32))
            .expect("actual painted diameter input");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            paint(
                &ctx,
                &mut form,
                &s,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        let focus = ctx.memory(|m| m.focused()).expect("diameter focus");
        let select_all = || egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        paint(
            &ctx,
            &mut form,
            &s,
            vec![select_all(), egui::Event::Text("0".into())],
        )
        .drop_without_applying_deltas();
        assert_eq!(form.heads[&HeadKind::Pan].fields[0].1.text, "0");
        let (head, drive) = form.resolve(s.document()).unwrap();
        let mut invalid = default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            head: h, drive: d, ..
        } = &mut invalid
        else {
            unreachable!()
        };
        *h = head;
        *d = drive;
        let diagnostic = invalid.resolve(s.parameters()).unwrap_err().to_string();
        let out = paint(&ctx, &mut form, &s, vec![]);
        assert!(out.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t) if t.galley.job.text.contains(&diagnostic))),"Missing canonical diagnostic: {diagnostic}");
        out.drop_without_applying_deltas();
        for events in [
            vec![select_all(), egui::Event::Text("5".into())],
            vec![egui::Event::Text(".".into())],
            vec![egui::Event::Text("7".into())],
        ] {
            paint(&ctx, &mut form, &s, events).drop_without_applying_deltas();
            assert_eq!(ctx.memory(|m| m.focused()), Some(focus));
        }
        assert_eq!(form.heads[&HeadKind::Pan].fields[0].1.text, "5.7");
        let (head, _) = form.resolve(s.document()).unwrap();
        let ScrewHead::Pan { profile } = head else {
            unreachable!()
        };
        assert_eq!(profile.diameter, length(5.7));
        assert_eq!(s.revision(), 0);
    }
}
