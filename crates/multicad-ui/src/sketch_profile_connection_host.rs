//! Profile picks come from an isolated current physical member occurrence.
use super::*;
use crate::{
    modeling::ProfileConnectorPick,
    viewport::{PickFilter, PickOptions, Scene, SceneFeed, Selection},
};
use multicad_core::{
    model::{self, MemberEnd, ProfileConnectorAnchor, ProfileGrooveSide},
    project_file::ProjectResources,
    references::{
        AnalyticGeometry, ReferenceFrame, TopologyElement, TopologyKind, TopologySnapshot,
    },
};
use std::sync::Arc;

pub(super) struct ProfileConnectionPickView {
    canonical: Arc<Scene>,
    pick: ProfileConnectorPick,
    view: Arc<Scene>,
    camera: Camera,
}

fn cap_role(end: MemberEnd) -> &'static str {
    match end {
        MemberEnd::Start => "cap.start",
        MemberEnd::End => "cap.end",
    }
}

fn has_origin(element: &TopologyElement, owner: EntityId, role: &str) -> bool {
    element
        .origins
        .iter()
        .any(|origin| origin.source == owner && origin.role == role)
}

fn end_face(element: &TopologyElement, owner: EntityId) -> Option<MemberEnd> {
    let mut ends = [MemberEnd::Start, MemberEnd::End]
        .into_iter()
        .filter(|end| has_origin(element, owner, cap_role(*end)));
    let end = ends.next()?;
    ends.next().is_none().then_some(end)
}

fn groove_face(element: &TopologyElement, owner: EntityId) -> Option<ProfileGrooveSide> {
    let mut sides = [
        ("member.groove.min_x.floor", ProfileGrooveSide::MinX),
        ("member.groove.max_x.floor", ProfileGrooveSide::MaxX),
        ("member.groove.min_y.floor", ProfileGrooveSide::MinY),
        ("member.groove.max_y.floor", ProfileGrooveSide::MaxY),
    ]
    .into_iter()
    .filter_map(|(role, side)| has_origin(element, owner, role).then_some(side));
    let side = sides.next()?;
    sides.next().is_none().then_some(side)
}

fn cap_frame(
    snapshot: &TopologySnapshot,
    owner: EntityId,
    end: MemberEnd,
) -> Option<ReferenceFrame> {
    let role = cap_role(end);
    let mut frames = snapshot
        .elements
        .iter()
        .filter(|element| {
            element.key.kind == TopologyKind::Face
                && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
        })
        .flat_map(|element| &element.frames)
        .filter(|frame| frame.origin.source == owner && frame.origin.role == role);
    let frame = frames.next()?.frame;
    frames.next().is_none().then_some(frame)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn station(
    snapshot: &TopologySnapshot,
    owner: EntityId,
    point_mm: [f64; 3],
) -> Result<(f64, f64), String> {
    let start = cap_frame(snapshot, owner, MemberEnd::Start)
        .ok_or("The native member start face needs repair")?;
    let end = cap_frame(snapshot, owner, MemberEnd::End)
        .ok_or("The native member end face needs repair")?;
    let ends = std::array::from_fn(|i| end.origin_mm[i] - start.origin_mm[i]);
    let end_axis = end.normal();
    let signed_length = dot(ends, end_axis);
    let length = signed_length.abs();
    if !length.is_finite() || length <= 1e-6 || dot(start.normal(), end_axis).abs() < 1. - 1e-8 {
        return Err("The member no longer has two unambiguous parallel end faces".into());
    }
    let axis = end_axis.map(|v| v * signed_length.signum());
    let from_start = std::array::from_fn(|i| point_mm[i] - start.origin_mm[i]);
    let station = dot(from_start, axis);
    if !station.is_finite() || station < -1e-5 || station > length + 1e-5 {
        return Err("Pick a point inside the current member length".into());
    }
    Ok((station.clamp(0., length), length))
}

impl SketchEditor {
    pub(super) fn profile_connection_pick_scene(
        &mut self,
        session: &Session,
        feed: &SceneFeed,
    ) -> Result<Option<Arc<Scene>>, String> {
        let Some(pick) = self
            .model
            .profile_connection_picker()
            .filter(|_| !self.measurement.active())
        else {
            if let Some(previous) = self.profile_connection_pick_view.take() {
                self.camera = previous.camera;
                self.fit_requested = false;
                self.solid.highlight_reference(None);
            }
            return Ok(None);
        };
        let canonical = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
            .ok_or("Wait for the current native profile member before picking")?
            .clone();
        if let Some(previous) = &self.profile_connection_pick_view
            && previous.pick == pick
            && Arc::ptr_eq(&previous.canonical, &canonical)
        {
            return Ok(Some(previous.view.clone()));
        }
        if !canonical.assembly_evaluation().is_some_and(|info| {
            info.leaves.iter().any(|leaf| {
                leaf.path == pick.target.occurrence && leaf.component == pick.target.member.body
            })
        }) {
            return Err(
                "The selected profile occurrence no longer matches the native member".into(),
            );
        }
        let view = canonical.isolated_occurrence_source(&pick.target.occurrence)?;
        let camera = self
            .profile_connection_pick_view
            .as_ref()
            .map_or(self.camera, |previous| previous.camera);
        self.profile_connection_pick_view = Some(ProfileConnectionPickView {
            canonical,
            pick,
            view: view.clone(),
            camera,
        });
        self.fit_requested = true;
        Ok(Some(view))
    }

    pub(super) fn profile_connection_viewport(
        &mut self,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        presented: Option<&Scene>,
        rect: Rect,
    ) {
        let Some(cache) = &self.profile_connection_pick_view else {
            return;
        };
        if self.model.profile_connection_picker().as_ref() != Some(&cache.pick)
            || presented.is_none_or(|shown| !std::ptr::eq(shown, cache.view.as_ref()))
        {
            return;
        }
        let pick = &cache.pick;
        let owner = pick.target.member.producer;
        let Some(snapshot) = cache.view.topology_snapshot(pick.target.occurrence.root) else {
            return;
        };
        let valid_face = |element: &TopologyElement| {
            element.key.kind == TopologyKind::Face
                && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                && if pick.end {
                    end_face(element, owner).is_some()
                } else {
                    groove_face(element, owner).is_some()
                }
        };
        let hit = response.hover_pos().and_then(|pointer| {
            cache.view.pick_screen_matching(
                self.camera,
                rect,
                pointer,
                PickOptions {
                    filter: PickFilter::Faces,
                    include_hidden: false,
                    section: None,
                },
                |instance, key| {
                    instance == pick.target.occurrence.root
                        && snapshot
                            .elements
                            .iter()
                            .any(|element| element.key == key && valid_face(element))
                },
            )
        });
        self.solid
            .highlight_reference(hit.as_ref().map(|hit| Selection::Face {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            }));
        if !response.clicked() {
            return;
        }
        let Some(hit) = hit else {
            self.message = if pick.end {
                "Click a visible named end face of the selected member"
            } else {
                "Click the floor inside a visible T-slot on the selected member"
            }
            .into();
            return;
        };
        let Some(element) = snapshot
            .elements
            .iter()
            .find(|element| element.key == hit.key)
        else {
            return;
        };
        let result = station(&snapshot, owner, hit.point_mm).and_then(|(station_mm, length)| {
            let anchor = if pick.end {
                ProfileConnectorAnchor::End {
                    end: end_face(element, owner).ok_or("The selected end face needs repair")?,
                }
            } else {
                ProfileConnectorAnchor::Groove {
                    side: groove_face(element, owner).ok_or("The selected groove needs repair")?,
                    station: model::length(station_mm),
                }
            };
            self.model.set_profile_connection_capture(
                session,
                resources,
                &cache.canonical,
                pick,
                anchor,
                length,
            )
        });
        self.message = match result {
            Ok(()) => {
                "Profile attachment captured · Preview checks the connector and hardware".into()
            }
            Err(error) => error,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::references::{AnchoredFrame, Origin, TopologyKey};

    #[test]
    fn station_uses_rotated_native_end_planes_and_refuses_foreign_roles() {
        let owner = EntityId::new();
        let frame = |end, ordinal, origin_mm, v| {
            let origin = Origin {
                source: owner,
                role: cap_role(end).into(),
            };
            let frame = ReferenceFrame {
                origin_mm,
                u: [0., 1., 0.],
                v,
            };
            TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal,
                },
                origins: [origin.clone()].into(),
                boundary: Default::default(),
                frames: vec![AnchoredFrame { origin, frame }],
                geometry: Some(AnalyticGeometry::Plane { frame }),
            }
        };
        let mut snapshot = TopologySnapshot {
            owner: EntityId::new(),
            revision: 0,
            elements: vec![
                frame(MemberEnd::Start, 0, [10., 20., 30.], [0., 0., -1.]),
                frame(MemberEnd::End, 1, [110., 23., 48.], [0., 0., 1.]),
            ],
        };
        assert_eq!(
            station(&snapshot, owner, [37., 14., 55.]).unwrap(),
            (27., 100.)
        );
        assert!(station(&snapshot, owner, [111., 14., 55.]).is_err());
        assert!(station(&snapshot, EntityId::new(), [37., 14., 55.]).is_err());
        snapshot.elements.push(snapshot.elements[0].clone());
        assert!(station(&snapshot, owner, [37., 14., 55.]).is_err());
    }

    #[test]
    fn real_slotted_member_pick_names_the_floor_and_current_station() {
        use multicad_core::{
            commands::{Actor, Channel, Request},
            document::{Entity, Mutation},
            materials::{LibraryDefinition, ProfileShape, Stock, StockKind},
            model::{
                AxisAnchor, BodyOutput, MemberDefinition, MemberPath, MemberRequest, OccurrencePath,
            },
        };
        let mut session = Session::new(Document::new("Native groove pick").unwrap()).unwrap();
        let material = Entity::new(
            "Aluminum",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [160, 165, 170, 255],
                section_hatch: None,
            },
        );
        let stock = EntityId::new();
        let stock_entity = LibraryDefinition::Stock(Stock {
            material: material.id,
            specification: StockKind::Profile {
                designation: "item 0.0.026.03".into(),
                length_mm: 6000.,
                width_mm: 40.,
                height_mm: 40.,
                wall_mm: None,
                shape: ProfileShape::ItemLine8_40x40,
            },
        })
        .entity(stock, "Line 8")
        .unwrap();
        let actor = Actor::local(Channel::Ui);
        session
            .apply(Request {
                expected_revision: 0,
                actor: actor.clone(),
                label: "Add profile stock".into(),
                edits: vec![Mutation::Insert(material), Mutation::Insert(stock_entity)],
            })
            .unwrap();
        let member = MemberRequest::new(
            "Profile",
            MemberDefinition {
                output: BodyOutput {
                    body: EntityId::nil(),
                },
                stock,
                path: MemberPath::Points {
                    start: [0., 0., 0.].map(model::length),
                    end: [0., 0., 100.].map(model::length),
                },
                anchor: [AxisAnchor::Min; 2],
                rotation: None,
                extend_start: None,
                extend_end: None,
            },
        );
        let prepared = model::prepare_members(
            &session,
            session.revision(),
            actor,
            std::slice::from_ref(&member),
            None,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        let native = multicad_model::Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let scene =
            crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 1)
                .unwrap();
        let root = member.member.output.body;
        let view = scene
            .isolated_occurrence_source(&OccurrencePath {
                root,
                members: vec![],
            })
            .unwrap();
        let ray = crate::viewport::Ray::new([20., -10., 27.], [0., 1., 0.]).unwrap();
        let hit = view.pick_face(ray).expect("actual native groove floor");
        let snapshot = view.topology_snapshot(root).unwrap();
        let element = snapshot
            .elements
            .iter()
            .find(|element| element.key == hit.key)
            .unwrap();
        assert_eq!(
            groove_face(element, member.feature),
            Some(ProfileGrooveSide::MinY)
        );
        let (station_mm, length) = station(&snapshot, member.feature, hit.point_mm).unwrap();
        assert!((station_mm - 27.).abs() < 1e-8);
        assert!((length - 100.).abs() < 1e-8);
    }
}
