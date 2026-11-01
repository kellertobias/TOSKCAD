use multicad_core::{
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters,
};

fn fixture(points: &[[f64; 2]], closed: bool) -> (Document, FenceConstruction, FencePath) {
    let material = Entity::new(
        "Timber",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [150, 110, 70, 255],
            section_hatch: None,
        },
    );
    let document = Document::new("Fence layout")
        .unwrap()
        .edited(&[Mutation::Insert(material.clone())])
        .unwrap();
    let construction = FenceConstruction {
        height: length(1800.),
        post: FencePostProfile::Rectangular {
            width: length(90.),
            depth: length(90.),
        },
        post_material: material.id,
        preferred_post_spacing: length(2000.),
        layout: FenceBayLayout::ShortLast,
        infills: vec![FenceInfill {
            id: EntityId::new(),
            name: "Panel".into(),
            material: material.id,
            bottom: length(100.),
            height: length(1600.),
            normal_offset: length(0.),
            construction: FenceInfillKind::Panel {
                thickness: length(18.),
                width: FencePanelWidth::FitBay,
            },
        }],
    };
    let path = FencePath {
        vertices: points
            .iter()
            .map(|point| FenceVertex {
                id: EntityId::new(),
                point: point.map(length),
            })
            .collect(),
        closed,
        base_z: length(0.),
    };
    (document, construction, path)
}
fn plan(
    document: &Document,
    construction: &FenceConstruction,
    path: &FencePath,
    positions: &[FencePostPosition],
) -> Result<FenceLayout, multicad_core::document::DocumentError> {
    plan_fence(
        document,
        &parameters::evaluate(document).unwrap(),
        construction,
        path,
        positions,
    )
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

#[test]
fn short_last_and_equal_policies_show_actual_center_and_clear_widths() {
    let (document, mut construction, path) = fixture(&[[0., 0.], [4500., 0.]], false);
    let before = path.clone();
    let short = plan(&document, &construction, &path, &[]).unwrap();
    assert_eq!(short.posts.len(), 4);
    assert_eq!(short.bays.len(), 3);
    assert_eq!(
        short
            .bays
            .iter()
            .map(|b| b.center_spacing_mm)
            .collect::<Vec<_>>(),
        [2000., 2000., 500.]
    );
    assert_eq!(
        short
            .bays
            .iter()
            .map(|b| b.clear_width_mm)
            .collect::<Vec<_>>(),
        [1910., 1910., 410.]
    );
    assert_eq!(
        short.infills.iter().map(|m| m.width_mm).collect::<Vec<_>>(),
        [1910., 1910., 410.]
    );
    assert_eq!(short.infills[2].position_mm, [4250., 0., 100.]);
    assert_eq!(short.infills[2].height_mm, 1600.);
    assert_eq!(short.infills[2].depth_mm, 18.);
    assert_eq!(path, before, "Planning never changes the authored path");
    construction.layout = FenceBayLayout::Equal;
    let equal = plan(&document, &construction, &path, &[]).unwrap();
    assert_eq!(
        equal
            .bays
            .iter()
            .map(|b| b.center_spacing_mm)
            .collect::<Vec<_>>(),
        [1500.; 3]
    );
    assert_eq!(
        equal
            .bays
            .iter()
            .map(|b| b.clear_width_mm)
            .collect::<Vec<_>>(),
        [1410.; 3]
    );
    assert_eq!(equal.infills[0].position_mm, [750., 0., 100.]);
    assert_eq!(
        short.posts.iter().map(|p| p.key).collect::<Vec<_>>(),
        equal.posts.iter().map(|p| p.key).collect::<Vec<_>>()
    );
}

#[test]
fn l_and_closed_paths_have_one_shared_corner_post_and_true_bay_directions() {
    let (document, construction, path) = fixture(&[[0., 0.], [4000., 0.], [4000., 3000.]], false);
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    assert_eq!(layout.posts.len(), 5);
    assert_eq!(layout.bays.len(), 4);
    assert_eq!(
        layout
            .posts
            .iter()
            .filter(|p| p.position_mm == [4000., 0., 0.])
            .count(),
        1
    );
    assert_eq!(layout.bays[2].direction, [0., 1.]);
    assert_eq!(layout.infills[2].position_mm, [4000., 1000., 100.]);
    let (document, construction, path) =
        fixture(&[[0., 0.], [4000., 0.], [4000., 3000.], [0., 3000.]], true);
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    assert_eq!(layout.posts.len(), 8);
    assert_eq!(layout.bays.len(), 8);
    let first = FencePostKey::Vertex {
        vertex: path.vertices[0].id,
    };
    assert_eq!(layout.posts.iter().filter(|p| p.key == first).count(), 1);
    assert_eq!(layout.bays.last().unwrap().key.end, first);
    assert_eq!(layout.bays.last().unwrap().direction, [0., -1.]);
    let keys: std::collections::BTreeSet<_> = layout.infills.iter().map(|m| m.key).collect();
    assert_eq!(keys.len(), layout.infills.len());
}

#[test]
fn rectangular_corner_support_uses_the_rotated_post_depth() {
    let (document, mut construction, path) =
        fixture(&[[0., 0.], [2000., 0.], [2000., 2000.]], false);
    construction.post = FencePostProfile::Rectangular {
        width: length(100.),
        depth: length(60.),
    };
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    near(layout.bays[0].clear_width_mm, 1920.);
    near(layout.bays[1].clear_width_mm, 1900.);
    assert_eq!(layout.posts[1].direction, [0., 1.]);
}

#[test]
fn explicit_post_positions_retain_keys_and_require_repair_for_removed_or_rewired_edges() {
    let (document, mut construction, mut path) = fixture(&[[0., 0.], [4500., 0.]], false);
    let key = |ordinal| FencePostKey::Interior {
        start: path.vertices[0].id,
        end: path.vertices[1].id,
        ordinal,
    };
    let first = key(1);
    let second = key(2);
    let positions = vec![
        FencePostPosition {
            post: first,
            offset: length(1500.),
        },
        FencePostPosition {
            post: second,
            offset: length(3500.),
        },
    ];
    let layout = plan(&document, &construction, &path, &positions).unwrap();
    assert_eq!(
        layout
            .posts
            .iter()
            .find(|p| p.key == first)
            .unwrap()
            .position_mm,
        [1500., 0., 0.]
    );
    assert!(
        layout
            .posts
            .iter()
            .find(|p| p.key == first)
            .unwrap()
            .overridden
    );
    assert_eq!(
        layout
            .bays
            .iter()
            .map(|b| b.center_spacing_mm)
            .collect::<Vec<_>>(),
        [1500., 2000., 1000.]
    );
    path.vertices[1].point[0] = length(5000.);
    let edited = plan(&document, &construction, &path, &positions).unwrap();
    assert_eq!(
        edited.posts.iter().map(|p| p.key).collect::<Vec<_>>(),
        layout.posts.iter().map(|p| p.key).collect::<Vec<_>>()
    );
    construction.preferred_post_spacing = length(2500.);
    assert!(
        plan(&document, &construction, &path, &positions)
            .unwrap_err()
            .to_string()
            .contains("no longer exists")
    );
    construction.preferred_post_spacing = length(2000.);
    path.vertices[1].id = EntityId::new();
    assert!(
        plan(&document, &construction, &path, &positions)
            .unwrap_err()
            .to_string()
            .contains("repair")
    );
}

#[test]
fn crossed_out_of_segment_and_duplicate_overrides_never_get_sorted_or_discarded() {
    let (document, construction, path) = fixture(&[[0., 0.], [4500., 0.]], false);
    let key = |ordinal| FencePostKey::Interior {
        start: path.vertices[0].id,
        end: path.vertices[1].id,
        ordinal,
    };
    for positions in [
        vec![FencePostPosition {
            post: key(1),
            offset: length(4100.),
        }],
        vec![FencePostPosition {
            post: key(1),
            offset: length(4600.),
        }],
        vec![
            FencePostPosition {
                post: key(1),
                offset: length(1500.),
            },
            FencePostPosition {
                post: key(1),
                offset: length(1600.),
            },
        ],
        vec![FencePostPosition {
            post: FencePostKey::Vertex {
                vertex: path.vertices[0].id,
            },
            offset: length(100.),
        }],
    ] {
        assert!(plan(&document, &construction, &path, &positions).is_err());
    }
}

#[test]
fn fixed_panels_cut_only_when_explicit_and_incompatible_spans_fail_completely() {
    let (document, mut construction, path) = fixture(&[[0., 0.], [4500., 0.]], false);
    construction.infills[0].construction = FenceInfillKind::Panel {
        thickness: length(18.),
        width: FencePanelWidth::Purchased {
            width: length(1910.),
            short_bay: FenceShortPanelPolicy::Reject,
        },
    };
    assert!(plan(&document, &construction, &path, &[]).is_err());
    if let FenceInfillKind::Panel {
        width: FencePanelWidth::Purchased { short_bay, .. },
        ..
    } = &mut construction.infills[0].construction
    {
        *short_bay = FenceShortPanelPolicy::Cut;
    }
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    assert_eq!(
        layout.infills[2].kind,
        FenceLayoutInfillKind::Panel {
            cut_from_width_mm: Some(1910.)
        }
    );
    assert_eq!(
        layout.infills[0].kind,
        FenceLayoutInfillKind::Panel {
            cut_from_width_mm: None
        }
    );
    assert_eq!(layout.infills[2].width_mm, 410.);
    construction.preferred_post_spacing = length(2100.);
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("Purchased panel")
    );
}

#[test]
fn pickets_keep_width_and_equal_bounded_clear_gaps_while_rails_span_each_bay() {
    let (document, mut construction, path) = fixture(&[[0., 0.], [2000., 0.]], false);
    construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(70.),
        depth: length(20.),
        maximum_gap: length(100.),
    };
    let mut rail = construction.infills[0].clone();
    rail.id = EntityId::new();
    rail.name = "Rail".into();
    rail.height = length(60.);
    rail.normal_offset = length(45.);
    rail.construction = FenceInfillKind::Rail { depth: length(40.) };
    construction.infills.push(rail);
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    let pickets: Vec<_> = layout
        .infills
        .iter()
        .filter(|m| m.kind == FenceLayoutInfillKind::Picket)
        .collect();
    assert_eq!(pickets.len(), 11);
    let mut end = 45.;
    for (ordinal, picket) in pickets.iter().enumerate() {
        near(picket.width_mm, 70.);
        near(picket.position_mm[0] - 35. - end, 95.);
        end = picket.position_mm[0] + 35.;
        assert_eq!(picket.key.ordinal, ordinal as u32);
    }
    near(1955. - end, 95.);
    let rail = layout.infills.last().unwrap();
    assert_eq!(rail.kind, FenceLayoutInfillKind::Rail);
    assert_eq!(rail.width_mm, 1910.);
    assert_eq!(rail.position_mm, [1000., 45., 100.]);
    assert_eq!(rail.height_mm, 60.);
}

#[test]
fn path_degeneracy_self_contact_and_unsupported_rectangular_corners_fail() {
    for points in [
        vec![[0., 0.], [0., 0.]],
        vec![[0., 0.], [2000., 0.], [1000., 0.]],
        vec![[0., 0.], [2000., 2000.], [0., 2000.], [2000., 0.]],
        vec![[0., 0.], [2000., 0.], [2000., 2000.], [1000., 0.]],
        vec![
            [0., 0.],
            [4000., 0.],
            [4000., 2000.],
            [1000., 2000.],
            [1000., 0.],
            [3000., 0.],
        ],
    ] {
        let (document, construction, path) = fixture(&points, false);
        assert!(plan(&document, &construction, &path, &[]).is_err());
    }
    let (document, mut construction, path) =
        fixture(&[[0., 0.], [2000., 0.], [3000., 1000.]], false);
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("right angles")
    );
    construction.post = FencePostProfile::Round {
        diameter: length(90.),
    };
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    near(layout.bays[1].clear_width_mm, 1000_f64.hypot(1000.) - 90.);
}

#[test]
fn impossible_clear_spans_and_resource_limits_fail_before_unbounded_generation() {
    let (document, mut construction, mut path) = fixture(&[[0., 0.], [4500., 0.]], false);
    construction.preferred_post_spacing = length(0.1);
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("bounded")
    );
    construction.preferred_post_spacing = length(2000.);
    construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(0.001),
        depth: length(20.),
        maximum_gap: length(0.001),
    };
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("bounded")
    );
    construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(500.),
        depth: length(20.),
        maximum_gap: length(100.),
    };
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("clear-gap")
    );
    construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(2000.),
        depth: length(20.),
        maximum_gap: length(100.),
    };
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("wider")
    );
    path.vertices[1].point[0] = length(80.);
    assert!(
        plan(&document, &construction, &path, &[])
            .unwrap_err()
            .to_string()
            .contains("collide")
    );
}

#[test]
fn integral_pitch_roundoff_does_not_create_a_microscopic_extra_bay() {
    let (document, construction, path) = fixture(&[[0., 0.], [6000.000000000001, 0.]], false);
    let layout = plan(&document, &construction, &path, &[]).unwrap();
    assert_eq!(layout.bays.len(), 3);
    assert_eq!(layout.posts.len(), 4);
    assert_eq!(layout.posts[1].position_mm[0], 6000.000000000001);
    near(layout.bays[2].center_spacing_mm, 2000.);
}

#[test]
fn post_queries_support_explicit_span_repair_without_claiming_a_complete_infill_layout() {
    let (document, mut construction, path) = fixture(&[[0., 0.], [4500., 0.]], false);
    construction.infills[0].construction = FenceInfillKind::Panel {
        thickness: length(18.),
        width: FencePanelWidth::Purchased {
            width: length(1910.),
            short_bay: FenceShortPanelPolicy::Reject,
        },
    };
    let parameters = multicad_core::parameters::evaluate(&document).unwrap();
    let posts = plan_fence_posts(&document, &parameters, &construction, &path, &[]).unwrap();
    assert_eq!(posts.posts.len(), 4);
    assert_eq!(posts.bays[2].clear_width_mm, 410.);
    assert!(plan_fence(&document, &parameters, &construction, &path, &[]).is_err());
    let mut duplicate = path;
    duplicate.vertices[1].id = duplicate.vertices[0].id;
    assert!(plan_fence_posts(&document, &parameters, &construction, &duplicate, &[]).is_err());
}
