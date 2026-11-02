//! Render the production Project UI offscreen with a real native project.
//! This creates no window and provides no macOS interaction acceptance evidence.
use eframe::{egui, egui_wgpu, wgpu};
use multicad_core::{
    document::{Document, EntityId},
    project_file::{self, ProjectCard, info::PreviewImage},
};
use multicad_ui::{
    application::{Application, DocumentStorage},
    library::{LibraryEntry, LibraryStore, RegisterReceipt, RegisterRequest},
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};
type Error = Box<dyn std::error::Error>;
struct ReadOnly;
impl DocumentStorage for ReadOnly {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        Err("Offscreen review does not write project files".into())
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        Err("Project is supplied directly".into())
    }
}
impl LibraryStore for ReadOnly {
    fn load(&mut self) -> Result<Vec<LibraryEntry>, String> {
        Ok(vec![])
    }
    fn register(
        &mut self,
        _: &RegisterRequest,
    ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String> {
        Err("Read-only offscreen review".into())
    }
    fn remove(&mut self, _: EntityId) -> Result<Vec<LibraryEntry>, String> {
        Err("Read-only offscreen review".into())
    }
    fn card(&mut self, _: &Path) -> Result<ProjectCard, String> {
        Err("No catalog cards requested by this review".into())
    }
    fn read_preview(&mut self, _: &Path) -> Result<PreviewImage, String> {
        Err("Uses the native project's existing preview".into())
    }
}
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        assert!(Instant::now() < deadline, "Offscreen GPU request timed out");
        std::thread::park_timeout(Duration::from_millis(5));
    }
}
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    file: &project_file::ProjectFile,
    source: &Path,
    output: &Path,
    size: [u32; 2],
    density: f32,
) -> Result<(), Error> {
    let mut app = Application::with_document(Box::new(ReadOnly), file.document.clone())?;
    app.current_mut().resources = file.resources.clone();
    app.current_mut().path = Some(source.to_path_buf());
    app.set_library_store(Box::new(ReadOnly));
    let context = egui::Context::default();
    multicad_ui::theme::configure(&context);
    context.set_pixels_per_point(density);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = egui_wgpu::Renderer::new(device, format, Default::default());
    let pixels = [
        (size[0] as f32 * density) as u32,
        (size[1] as f32 * density) as u32,
    ];
    let extent = wgpu::Extent3d {
        width: pixels[0],
        height: pixels[1],
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Production Project UI review"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    for index in 0..6 {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0] as f32, size[1] as f32),
                )),
                focused: true,
                time: Some(index as f64 / 60.),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(device, queue, *id, delta);
            }
        }
        output.textures_delta.set.clear();
        let descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: pixels,
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = context.tessellate(output.shapes, output.pixels_per_point);
        let mut encoder = device.create_command_encoder(&Default::default());
        let commands = renderer.update_buffers(device, queue, &mut encoder, &jobs, &descriptor);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Offscreen production egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
        }
        queue.submit(commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.free.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    let row_bytes = (pixels[0] * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("UI image readback"),
        size: u64::from(row_bytes) * u64::from(pixels[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(pixels[1]),
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(20)),
    })?;
    receive.recv_timeout(Duration::from_secs(5))??;
    let mapped = buffer.slice(..).get_mapped_range()?;
    let mut rgba = Vec::with_capacity((pixels[0] * pixels[1] * 4) as usize);
    for row in mapped.chunks_exact(row_bytes as usize) {
        rgba.extend_from_slice(&row[..pixels[0] as usize * 4]);
    }
    drop(mapped);
    buffer.unmap();
    let mut encoder = png::Encoder::new(std::fs::File::create(output)?, pixels[0], pixels[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    writer.finish()?;
    Ok(())
}
fn main() -> Result<(), Error> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(
        args.next()
            .ok_or("Usage: project_shell_review PROJECT.tcd OUTPUT_DIRECTORY")?,
    )
    .canonicalize()?;
    let output = PathBuf::from(args.next().ok_or("Missing output directory")?);
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    std::fs::create_dir_all(&output)?;
    let project = project_file::read(
        &mut std::fs::File::open(&source)?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = wait(instance.request_adapter(&Default::default()))?;
    let (device, queue) = wait(adapter.request_device(&Default::default()))?;
    let mut images = vec![];
    for (size, density) in [([1100, 700], 1.), ([1360, 860], 1.), ([1360, 860], 2.)] {
        let name = format!("project-{}x{}-{}x.png", size[0], size[1], density as u32);
        render(
            &device,
            &queue,
            &project,
            &source,
            &output.join(&name),
            size,
            density,
        )?;
        images
            .push(serde_json::json!({"file":name,"logical_size":size,"pixels_per_point":density}));
    }
    let report = serde_json::json!({"source":source,"document":project.document.id(),"gpu":format!("{:?}",adapter.get_info()),"images":images,"evidence":"Offscreen rendering of production Application/egui controls with real project data. No native window or macOS interaction."});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
