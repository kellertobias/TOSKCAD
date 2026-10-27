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
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        wait(instance.request_adapter(&Default::default())).expect("GPU adapter required");
    println!("Physical GPU check: {:?}", adapter.get_info());
    let (device, queue) = wait(adapter.request_device(&Default::default())).unwrap();
    let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let id = EntityId::new();
    let solid = Solid::box_named([100., 60., 18.], id, Default::default()).unwrap();
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
    let size = wgpu::Extent3d {
        width: 256,
        height: 256,
        depth_or_array_layers: 1,
    };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CAD physical render check"),
            size,
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
    let mut first_buffer = None;
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
    for (perspective, reverse, selection, oblique) in [
        (false, false, None, false),
        (true, true, None, false),
        (false, true, Some(Selection::Part(near.id)), false),
        (
            true,
            false,
            Some(Selection::Face {
                instance: near.id,
                ordinal: cap,
            }),
            false,
        ),
        (true, false, None, true),
    ] {
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
            camera,
            egui::vec2(256., 256.),
            selection,
        );
        assert_eq!(renderer.meshes.len(), 1);
        assert_eq!(renderer.draws.len(), 1);
        assert_eq!(renderer.draws[0].count, 2);
        if let Some(buffer) = &first_buffer {
            assert_eq!(
                buffer, &renderer.meshes[&id].vertices,
                "Camera/instance edits reuploaded a cached mesh"
            );
        }
        first_buffer = Some(renderer.meshes[&id].vertices.clone());
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
            renderer.paint(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256 * 4),
                    rows_per_image: Some(256),
                },
            },
            size,
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
        let pixels = readback.slice(..).get_mapped_range().unwrap();
        let center = &pixels[(128 * 256 + 128) * 4..(128 * 256 + 128) * 4 + 4];
        assert!(
            u16::from(center[0]) > u16::from(center[2]) * 2,
            "Front red solid disappeared: {center:?}"
        );
        if selection.is_some() {
            assert!(center[1] > 80, "Selection highlight missing: {center:?}");
        }
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] > 40)
                .count()
                > 5000,
            "Viewport is mostly empty"
        );
        println!(
            "projection perspective={perspective}, reverse={reverse}, selection={selection:?}, oblique={oblique}, center RGBA={center:?}"
        );
        drop(pixels);
        readback.unmap();
    }
}
