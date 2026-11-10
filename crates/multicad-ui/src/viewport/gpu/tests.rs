use super::*;
use crate::viewport::{Instance, SceneStamp, Transform};
use multicad_core::document::Placement;
use multicad_kernel::{MeshOptions, Solid};
use std::{
    future::Future,
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};
fn wait<T>(future: impl Future<Output = T>) -> T {
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "GPU request timed out"
        );
        std::thread::park_timeout(Duration::from_millis(20));
    }
}
#[test]
fn solid_shader_validates_without_a_gpu() {
    let module =
        naga::front::wgsl::parse_str(include_str!("../solid.wgsl")).expect("WGSL must parse in CI");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("WGSL must validate with baseline GPU capabilities");
}
#[test]
#[ignore = "Requires an available GPU; run explicitly on physical hardware"]
fn gpu_renders_native_solids_with_depth_projection_highlight_and_cached_meshes() {
    render_native_gpu(false);
}
#[test]
#[ignore = "Requires an available GPU; run explicitly on physical hardware"]
fn gpu_renders_source_face_materials_with_selection_and_edges() {
    render_native_gpu(true);
}
fn render_native_gpu(material_palette: bool) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        wait(instance.request_adapter(&Default::default())).expect("GPU adapter required");
    println!("Physical GPU check: {:?}", adapter.get_info());
    let (device, queue) = wait(adapter.request_device(&Default::default())).unwrap();
    let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let id = EntityId::new();
    let solid = Solid::box_named([100., 60., 18.], id, Default::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let cap = solid
        .topology_snapshot(id, 0)
        .unwrap()
        .elements
        .into_iter()
        .find(|e| {
            e.key.kind == multicad_core::references::TopologyKind::Face
                && e.origins.iter().any(|o| o.role == "cap.end")
        })
        .unwrap()
        .key
        .ordinal;
    let geometry = Arc::new(
        Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid.topology_snapshot(id, 0).unwrap().elements,
        )
        .unwrap()
        .with_face_colors(if material_palette {
            BTreeMap::from([(cap, [20, 240, 20, 255])])
        } else {
            BTreeMap::new()
        })
        .unwrap()
        .with_topology_display(
            solid
                .display_edges(0.1)
                .unwrap()
                .into_iter()
                .map(|e| crate::viewport::DisplayEdge {
                    index: e.index,
                    points_mm: e.points_mm,
                })
                .collect(),
            solid
                .display_vertices()
                .unwrap()
                .into_iter()
                .map(|v| crate::viewport::DisplayPoint {
                    index: v.index,
                    position_mm: v.position_mm,
                })
                .collect(),
            0.1,
        )
        .unwrap(),
    );
    let far = Instance {
        id: EntityId::new(),
        geometry: id,
        transform: Transform::default(),
        color: [20, 20, 240, 255],
    };
    let near = Instance {
        id: EntityId::new(),
        geometry: id,
        transform: Transform::from_placement(Placement {
            translation_mm: [0., 0., 30.],
            ..Default::default()
        })
        .unwrap(),
        color: [240, 20, 20, 255],
    };
    let stamp = SceneStamp {
        document: EntityId::new(),
        revision: 0,
        generation: 0,
    };
    let (color, depth, readback) = targets(&device);
    let mut first_buffer = None;
    // CAD-152: a second face of the same instance comes from a highlight-only
    // overlay record; listing a side face first proves the overlay path.
    let side = solid
        .topology_snapshot(id, 0)
        .unwrap()
        .elements
        .into_iter()
        .find(|e| {
            e.key.kind == multicad_core::references::TopologyKind::Face && e.key.ordinal != cap
        })
        .unwrap()
        .key
        .ordinal;
    let mut images = vec![];
    for ((perspective, reverse, selection, oblique, style), section) in [
        (false, false, None, false, DisplayStyle::Filled),
        (true, true, None, false, DisplayStyle::Filled),
        (
            false,
            true,
            Some(Selection::Part(near.id)),
            false,
            DisplayStyle::Filled,
        ),
        (
            true,
            false,
            Some(Selection::Face {
                instance: near.id,
                ordinal: cap,
            }),
            false,
            DisplayStyle::Filled,
        ),
        (
            false,
            false,
            Some(Selection::Parts(Arc::new(BTreeSet::from([
                near.id, far.id,
            ])))),
            false,
            DisplayStyle::Filled,
        ),
        (
            true,
            false,
            Some(Selection::Faces(Arc::new(vec![
                (near.id, side),
                (near.id, cap),
            ]))),
            false,
            DisplayStyle::Filled,
        ),
        (true, false, None, true, DisplayStyle::Filled),
        (true, false, None, true, DisplayStyle::AllEdges),
        (true, false, None, true, DisplayStyle::VisibleEdges),
        (true, false, None, true, DisplayStyle::Wire),
    ]
    .into_iter()
    .map(|case| (case, None))
    .chain([(
        (false, false, None, false, DisplayStyle::Filled),
        Some(SectionPlane::new([0., 0., -1.], -20.).unwrap()),
    )])
    .chain([(
        (
            false,
            false,
            Some(Selection::Parts(Arc::new(BTreeSet::from([
                near.id, far.id,
            ])))),
            false,
            DisplayStyle::Filled,
        ),
        Some(SectionPlane::new([0., 0., -1.], -20.).unwrap()),
    )])
    .chain(DisplayStyle::ALL.into_iter().map(|style| {
        (
            (false, false, None, false, style),
            Some(SectionPlane::new([1., 0., 0.], 60.).unwrap()),
        )
    })) {
        let parts = if reverse {
            vec![near.clone(), far.clone()]
        } else {
            vec![far.clone(), near.clone()]
        };
        let scene = Arc::new(Scene::new(stamp, vec![geometry.clone()], parts).unwrap());
        let camera = Camera {
            target: [50., 30., 48.],
            yaw: if oblique { -0.6 } else { 0. },
            pitch: if oblique {
                0.6
            } else {
                std::f32::consts::FRAC_PI_2
            },
            scale: 2.,
            perspective,
        };
        renderer.prepare(
            &device,
            &queue,
            &scene,
            View {
                camera,
                size: egui::vec2(256., 256.),
                selection: selection.clone(),
                style,
                section,
                origin_axes: false,
            },
        );
        assert_eq!(renderer.meshes.len(), 1);
        assert_eq!(renderer.draws.len(), 1);
        // Two instances plus one overlay record per extra face of an instance.
        let overlays = match &selection {
            Some(Selection::Faces(faces)) => faces.len() as u32 - 1,
            _ => 0,
        };
        assert_eq!(renderer.draws[0].count, 2 + overlays);
        if let Some(buffer) = &first_buffer {
            assert_eq!(
                buffer, &renderer.meshes[&id].vertices,
                "Camera/instance edits reuploaded a cached mesh"
            );
        }
        first_buffer = Some(renderer.meshes[&id].vertices.clone());
        let pixels = capture(&device, &queue, &renderer, &color, &depth, &readback, style);
        let center = &pixels[(128 * 256 + 128) * 4..(128 * 256 + 128) * 4 + 4];
        let coverage = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[..3].iter().any(|v| *v > 40))
            .count();
        if section.is_some_and(|s| !s.contains([0.; 3])) {
            assert_eq!(
                &center[..3],
                &[0, 0, 0],
                "Section failed to remove the center"
            );
            for row in pixels.as_chunks::<1024>().0 {
                assert!(
                    row[..140 * 4]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| p[..3] == [0, 0, 0]),
                    "Clipped lines or faces remained visible"
                );
            }
            assert!(coverage > 50, "Section removed both sides");
        } else if section.is_some() {
            if selection.is_some() {
                assert!(
                    center[1] > 80,
                    "Rear selected instance lost its multi-selection highlight: {center:?}"
                );
            } else {
                assert!(
                    if material_palette {
                        u16::from(center[1]) > u16::from(center[0]) * 2
                    } else {
                        u16::from(center[2]) > u16::from(center[0]) * 2
                    },
                    "Section did not reveal the blue solid behind the red one: {center:?}"
                );
            }
        } else if style == DisplayStyle::Wire {
            assert!(
                (200..5000).contains(&coverage),
                "Wireframe coverage is invalid: {coverage}"
            );
        } else {
            assert!(
                if material_palette && selection.is_none() {
                    u16::from(center[1]) > u16::from(center[0]) * 2
                } else {
                    u16::from(center[0]) > u16::from(center[2]) * 2
                },
                "Front red solid disappeared: {center:?}"
            );
            if selection.is_some() {
                assert!(center[1] > 80, "Selection highlight missing: {center:?}");
            }
            assert!(coverage > 5000, "Viewport is mostly empty");
        }
        if oblique {
            images.push((style, pixels.to_vec()));
        }
        println!(
            "projection perspective={perspective}, reverse={reverse}, selection={selection:?}, oblique={oblique}, style={style:?}, center RGBA={center:?}"
        );
    }
    let image = |style| &images.iter().find(|(s, _)| *s == style).unwrap().1;
    let differences = |a: DisplayStyle, b: DisplayStyle| {
        image(a)
            .as_chunks::<4>()
            .0
            .iter()
            .zip(image(b).as_chunks::<4>().0)
            .filter(|(a, b)| a != b)
            .count()
    };
    assert!(
        differences(DisplayStyle::AllEdges, DisplayStyle::VisibleEdges) > 20,
        "Hidden edges did not change"
    );
    assert!(
        differences(DisplayStyle::Filled, DisplayStyle::VisibleEdges) > 20,
        "Visible edges did not render"
    );
}

const IMAGE_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 256,
    height: 256,
    depth_or_array_layers: 1,
};

fn targets(device: &wgpu::Device) -> (wgpu::Texture, wgpu::Texture, wgpu::Buffer) {
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CAD physical render check"),
            size: IMAGE_SIZE,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("CAD pixel readback"),
        size: 256 * 256 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (color, depth, readback)
}

fn capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Renderer,
    color: &wgpu::Texture,
    depth: &wgpu::Texture,
    readback: &wgpu::Buffer,
    style: DisplayStyle,
) -> Vec<u8> {
    let mut encoder = device.create_command_encoder(&Default::default());
    let view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("CAD depth and projection test"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.paint(&mut pass, style);
    }
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(256),
            },
        },
        IMAGE_SIZE,
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(10)),
        })
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
    readback.unmap();
    pixels
}

#[test]
#[ignore = "Requires an available GPU; run explicitly on physical hardware"]
fn distant_visible_edges_hide_the_rear_plate_in_both_projections() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        wait(instance.request_adapter(&Default::default())).expect("GPU adapter required");
    let (device, queue) = wait(adapter.request_device(&Default::default())).unwrap();
    let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let front = EntityId::new();
    let rear = EntityId::new();
    let plates = [(front, [180., 140., 4.]), (rear, [100., 60., 4.])];
    let geometry: Vec<_> = plates
        .into_iter()
        .map(|(id, dimensions)| {
            let solid = Solid::box_named(dimensions, id, Default::default()).unwrap();
            let mesh = solid.mesh(MeshOptions::default()).unwrap();
            Arc::new(
                Geometry::new(
                    id,
                    mesh.vertices.as_chunks::<3>().0.to_vec(),
                    mesh.indices,
                    mesh.face_indices,
                    solid.topology_snapshot(id, 0).unwrap().elements,
                )
                .unwrap()
                .with_topology_display(
                    solid
                        .display_edges(0.1)
                        .unwrap()
                        .into_iter()
                        .map(|edge| crate::viewport::DisplayEdge {
                            index: edge.index,
                            points_mm: edge.points_mm,
                        })
                        .collect(),
                    vec![],
                    0.1,
                )
                .unwrap(),
            )
        })
        .collect();
    let plate = |geometry, translation_mm| Instance {
        id: EntityId::new(),
        geometry,
        transform: Transform::from_placement(Placement {
            translation_mm,
            ..Default::default()
        })
        .unwrap(),
        color: [240, 40, 40, 255],
    };
    let front = plate(front, [0., 0., 8.]);
    let rear = plate(rear, [40., 40., 0.]);
    let stamp = SceneStamp {
        document: EntityId::new(),
        revision: 0,
        generation: 0,
    };
    let combined =
        Arc::new(Scene::new(stamp, geometry.clone(), vec![front.clone(), rear]).unwrap());
    let front_only = Arc::new(Scene::new(stamp, geometry, vec![front]).unwrap());
    let (color, depth, readback) = targets(&device);
    for perspective in [false, true] {
        for scale in [0.2, 0.8] {
            let camera = Camera {
                target: [90., 70., 12.],
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                scale,
                perspective,
            };
            let mut render = |scene: &Arc<Scene>, style| {
                renderer.prepare(
                    &device,
                    &queue,
                    scene,
                    View {
                        camera,
                        size: egui::vec2(256., 256.),
                        selection: None,
                        style,
                        section: None,
                        origin_axes: false,
                    },
                );
                capture(&device, &queue, &renderer, &color, &depth, &readback, style)
            };
            let visible = render(&combined, DisplayStyle::VisibleEdges);
            let control = render(&front_only, DisplayStyle::VisibleEdges);
            let filled = render(&combined, DisplayStyle::Filled);
            let all = render(&combined, DisplayStyle::AllEdges);
            let difference = |a: &[u8], b: &[u8]| {
                a.as_chunks::<4>()
                    .0
                    .iter()
                    .zip(b.as_chunks::<4>().0)
                    .filter(|(a, b)| a != b)
                    .count()
            };
            assert_eq!(
                difference(&visible, &control),
                0,
                "rear edges appeared in VisibleEdges, {camera:?}"
            );
            assert!(
                difference(&filled, &visible) > 20,
                "front edges disappeared, {camera:?}"
            );
            assert!(
                difference(&all, &visible) > 20,
                "rear plate must be detectable in AllEdges, {camera:?}"
            );
        }
    }
}

#[test]
#[ignore = "Requires an available GPU; run explicitly on physical hardware"]
fn world_origin_axes_respect_solid_depth_in_both_projections() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        wait(instance.request_adapter(&Default::default())).expect("GPU adapter required");
    let (device, queue) = wait(adapter.request_device(&Default::default())).unwrap();
    let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let (color, depth, readback) = targets(&device);
    let id = EntityId::new();
    let solid = Solid::box_named([30., 24., 4.], id, Default::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let geometry = Arc::new(
        Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid.topology_snapshot(id, 0).unwrap().elements,
        )
        .unwrap(),
    );
    let style = DisplayStyle::Filled;
    let stamp = SceneStamp {
        document: EntityId::new(),
        revision: 0,
        generation: 0,
    };
    let camera = Camera {
        target: [35., 0., 10.],
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 2.,
        perspective: false,
    };
    let axis_color = ORIGIN_AXES[0].1;
    let contains_x_axis = |pixels: &[u8], first_x: usize, last_x: usize| {
        (125..131).any(|y| {
            (first_x..last_x)
                .any(|x| pixels[(y * 256 + x) * 4..(y * 256 + x) * 4 + 3] == axis_color)
        })
    };
    for perspective in [false, true] {
        let camera = Camera {
            perspective,
            ..camera
        };
        for (plate_z, origin_axes) in [(-12., true), (8., true), (-12., false)] {
            let scene = Arc::new(
                Scene::new(
                    stamp,
                    vec![geometry.clone()],
                    vec![Instance {
                        id: EntityId::new(),
                        geometry: id,
                        transform: Transform::from_placement(Placement {
                            translation_mm: [25., -12., plate_z],
                            ..Default::default()
                        })
                        .unwrap(),
                        color: [200, 200, 200, 255],
                    }],
                )
                .unwrap(),
            );
            renderer.prepare(
                &device,
                &queue,
                &scene,
                View {
                    camera,
                    size: egui::vec2(256., 256.),
                    selection: None,
                    style,
                    section: None,
                    origin_axes,
                },
            );
            let pixels = capture(&device, &queue, &renderer, &color, &depth, &readback, style);
            let adjacent_to_axis = (122 * 256 + 128) * 4;
            assert!(
                pixels[adjacent_to_axis] > 70,
                "Plate must cover the axis region"
            );
            assert_eq!(
                contains_x_axis(&pixels, 124, 132),
                plate_z < 0. && origin_axes,
                "Axis at the plate must follow depth and visibility; perspective={perspective}, plate_z={plate_z}"
            );
            assert_eq!(
                contains_x_axis(&pixels, 84, 94),
                origin_axes,
                "Uncovered axis remains visible beyond the scene bounds; perspective={perspective}, plate_z={plate_z}"
            );
            assert!(
                !contains_x_axis(&pixels, 20, 40),
                "Axis must start at world origin"
            );
        }
    }
}

#[test]
#[ignore = "Requires an available GPU; run explicitly on physical hardware"]
fn gpu_uploads_only_drawn_geometry_and_reuses_buffers_through_visibility_and_resize() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        wait(instance.request_adapter(&Default::default())).expect("GPU adapter required");
    println!("Physical GPU check: {:?}", adapter.get_info());
    let (device, queue) = wait(adapter.request_device(&Default::default())).unwrap();
    let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let [shared, hidden, history] = std::array::from_fn(|_| EntityId::new());
    let triangle = |id, size: f64| {
        Arc::new(
            Geometry::new(
                id,
                vec![[-size, -size, 0.], [size, -size, 0.], [0., size, 0.]],
                vec![0, 1, 2],
                vec![0],
                vec![],
            )
            .unwrap(),
        )
    };
    let [shared_geometry, hidden_geometry, history_geometry] =
        [shared, hidden, history].map(|id| triangle(id, 16.0));
    let placed = |geometry, translation_mm, color| Instance {
        id: EntityId::new(),
        geometry,
        transform: Transform::from_placement(Placement {
            translation_mm,
            ..Default::default()
        })
        .unwrap(),
        color,
    };
    let instances = vec![
        placed(shared, [-45., 0., 0.], [240, 30, 30, 255]),
        placed(shared, [0., 0., 0.], [240, 30, 30, 255]),
        placed(hidden, [45., 0., 0.], [30, 240, 30, 255]),
    ];
    let visible = BTreeSet::from([instances[0].id, instances[1].id]);
    let stamp = SceneStamp {
        document: EntityId::new(),
        revision: 0,
        generation: 0,
    };
    let scene = |second: Arc<Geometry>| {
        Arc::new(
            Scene::new(
                stamp,
                vec![shared_geometry.clone(), second, history_geometry.clone()],
                instances.clone(),
            )
            .unwrap(),
        )
    };
    let full = scene(hidden_geometry.clone());
    let subset = full.visible_subset(&visible).unwrap();
    assert_eq!(subset.geometries().len(), 3);
    let view = View {
        camera: Camera {
            target: [0.; 3],
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale: 2.,
            perspective: false,
        },
        size: egui::vec2(256., 256.),
        selection: None,
        style: DisplayStyle::Filled,
        section: None,
        origin_axes: false,
    };
    let (color, depth, readback) = targets(&device);
    let prepare =
        |renderer: &mut Renderer, scene, view| renderer.prepare(&device, &queue, scene, view);
    let pixels = |renderer: &Renderer| {
        capture(
            &device,
            &queue,
            renderer,
            &color,
            &depth,
            &readback,
            DisplayStyle::Filled,
        )
    };

    prepare(&mut renderer, &subset, view.clone());
    assert_eq!(
        renderer.meshes.keys().copied().collect::<Vec<_>>(),
        [shared]
    );
    assert_eq!(renderer.draws.len(), 1);
    assert_eq!(renderer.draws[0].count, 2);
    let shared_buffer = renderer.meshes[&shared].vertices.clone();
    let hidden_pixels = pixels(&renderer);
    assert!(
        hidden_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 70)
    );

    prepare(&mut renderer, &full, view.clone());
    assert_eq!(renderer.meshes.len(), 2);
    assert!(!renderer.meshes.contains_key(&history));
    let hidden_buffer = renderer.meshes[&hidden].vertices.clone();
    let shown_pixels = pixels(&renderer);
    assert!(
        shown_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[1] > 70)
    );
    assert_ne!(shown_pixels, hidden_pixels);
    prepare(&mut renderer, &subset, view.clone());
    assert_eq!(renderer.meshes[&hidden].vertices, hidden_buffer);
    assert_eq!(pixels(&renderer), hidden_pixels);
    prepare(&mut renderer, &full, view.clone());
    assert_eq!(renderer.meshes[&hidden].vertices, hidden_buffer);
    assert_eq!(pixels(&renderer), shown_pixels);

    let selected = View {
        selection: Some(Selection::Part(instances[0].id)),
        ..view.clone()
    };
    prepare(&mut renderer, &full, selected.clone());
    let selected_pixels = pixels(&renderer);
    assert_ne!(selected_pixels, shown_pixels);
    let placements: Vec<_> = renderer
        .draws
        .iter()
        .map(|draw| draw.instances.clone())
        .collect();
    prepare(
        &mut renderer,
        &full,
        View {
            size: egui::vec2(192., 224.),
            ..selected
        },
    );
    assert_ne!(pixels(&renderer), selected_pixels);
    assert_eq!(renderer.meshes[&shared].vertices, shared_buffer);
    assert_eq!(renderer.meshes[&hidden].vertices, hidden_buffer);
    assert!(
        renderer
            .draws
            .iter()
            .zip(placements)
            .all(|(draw, previous)| draw.instances == previous)
    );

    let changed_geometry = triangle(hidden, 12.0);
    let changed = scene(changed_geometry.clone());
    let changed_subset = changed.visible_subset(&visible).unwrap();
    prepare(&mut renderer, &changed_subset, view.clone());
    assert!(!renderer.meshes.contains_key(&hidden));
    prepare(&mut renderer, &changed, view);
    assert!(Arc::ptr_eq(
        &renderer.meshes[&hidden].source,
        &changed_geometry
    ));
    assert_ne!(renderer.meshes[&hidden].vertices, hidden_buffer);
    assert_eq!(renderer.meshes[&shared].vertices, shared_buffer);
    assert!(!renderer.meshes.contains_key(&history));
}
