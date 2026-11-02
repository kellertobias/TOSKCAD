//! Render production Project or Architecture UI offscreen with a real native project.
//! This creates no window and provides no macOS interaction acceptance evidence.
#[allow(dead_code)]
#[path = "../../multicad-app/src/modeling.rs"]
mod modeling;
use eframe::{egui, egui_wgpu, wgpu};
use multicad_core::{
    document::{Document, EntityId},
    project_file::{self, ProjectCard, info::PreviewImage},
};
use multicad_ui::{
    application::{Application, DocumentStorage},
    library::{LibraryEntry, LibraryStore, RegisterReceipt, RegisterRequest},
};
use multicad_ui::{architecture_navigation::Category, registry::Mode, viewport::SceneStamp};
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
#[derive(Clone, Copy)]
enum Review {
    Project,
    Architecture(Option<Category>),
    ArchitecturePlan,
    CompoundCutout(bool),
}
impl Review {
    fn name(self) -> String {
        match self {
            Self::Project => "project".into(),
            Self::Architecture(None) => "architecture-root".into(),
            Self::ArchitecturePlan => "architecture-plan".into(),
            Self::CompoundCutout(plan) => if plan {
                "compound-cutout-plan"
            } else {
                "compound-cutout-3d"
            }
            .into(),
            Self::Architecture(Some(category)) => {
                format!("architecture-{}", category.label().to_ascii_lowercase())
            }
        }
    }
}
fn control(output: &egui::FullOutput, label: &str) -> Result<egui::Pos2, Error> {
    // Feed egui's logical node bounds directly back to RawInput; native DPI
    // belongs to the viewport and readback dimensions, not these positions.
    let node = output
        .platform_output
        .accesskit_update
        .as_ref()
        .ok_or("No accessible controls")?
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .max_by(|a, b| (a.y1 - a.y0).total_cmp(&(b.y1 - b.y0)))
        .ok_or_else(|| format!("Missing painted control: {label}"))?;
    Ok(egui::pos2(
        (node.x0 + node.x1) as f32 / 2.,
        (node.y0 + node.y1) as f32 / 2.,
    ))
}
fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
struct Capture<'a> {
    source: &'a Path,
    output: &'a Path,
    size: [u32; 2],
    density: f32,
    review: Review,
}
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    file: &project_file::ProjectFile,
    capture: Capture<'_>,
) -> Result<Option<SceneStamp>, Error> {
    let Capture {
        source,
        output,
        size,
        density,
        review,
    } = capture;
    let mut app = Application::with_document(Box::new(ReadOnly), file.document.clone())?;
    app.current_mut().resources = file.resources.clone();
    app.current_mut().path = Some(source.to_path_buf());
    app.set_library_store(Box::new(ReadOnly));
    let architecture = matches!(
        review,
        Review::Architecture(_) | Review::ArchitecturePlan | Review::CompoundCutout(_)
    );
    if architecture {
        app.mode = Mode::Model;
        app.set_scene_backends(Arc::new(|| Box::new(modeling::Backend::new())));
        app.set_model_backends(Arc::new(|| Box::new(modeling::Backend::new())));
    }
    let source_hash = project_file::ContentHash::document(app.current().session.document())?;
    let mut routes: std::collections::VecDeque<_> = match review {
        Review::Project => [].into_iter().collect(),
        Review::Architecture(None) => ["Architecture"].into_iter().collect(),
        Review::ArchitecturePlan | Review::CompoundCutout(true) => {
            ["Architecture", "2D plan"].into_iter().collect()
        }
        Review::CompoundCutout(false) => ["Architecture"].into_iter().collect(),
        Review::Architecture(Some(category)) => {
            ["Architecture", category.label()].into_iter().collect()
        }
    };
    let mut release = None;
    let mut events = vec![];
    let mut fitted = false;
    let mut fitted_at = None;
    let mut stable = 0;
    let mut initial_scene = None;
    let mut compound_opened = false;
    let mut compound_preview_requested = false;
    let started = Instant::now();
    let context = egui::Context::default();
    multicad_ui::theme::configure(&context);
    context.enable_accesskit();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = egui_wgpu::Renderer::new(
        device,
        format,
        egui_wgpu::RendererOptions {
            depth_stencil_format: Some(wgpu::TextureFormat::Depth32Float),
            ..Default::default()
        },
    );
    if architecture {
        multicad_ui::viewport::install_renderer(device, format, &mut renderer.callback_resources);
    }
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
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Native viewport depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth.create_view(&Default::default());
    let mut index = 0;
    loop {
        if started.elapsed() > Duration::from_secs(120) {
            return Err("Native offscreen view did not become ready".into());
        }
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0] as f32, size[1] as f32),
            )),
            focused: true,
            time: Some(started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut events),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("root viewport")
            .native_pixels_per_point = Some(density);
        let mut output = context.run_ui(input, |ui| app.show(ui));
        if compound_opened && stable >= 2 {
            let check = || -> Result<(), Error> {
                let (callback_index, viewport) = output
                    .shapes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, shape)| {
                        if let egui::epaint::Shape::Callback(callback) = &shape.shape {
                            Some((i, callback.rect))
                        } else {
                            None
                        }
                    })
                    .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
                    .ok_or("Native viewport callback is missing")?;
                let source = multicad_core::model::compound_cutouts(&file.document)
                    .next()
                    .ok_or("Missing compound source")?;
                let definition =
                    multicad_core::model::read_compound_cutout(&file.document, source.id)?;
                let mut occupied: Vec<egui::Rect> = vec![];
                for index in 0..definition.boxes.len() {
                    let label = format!("Box {}", index + 1);
                    let (paint_index, bounds) = output
                        .shapes
                        .iter()
                        .enumerate()
                        .find_map(|(i, shape)| {
                            if let egui::epaint::Shape::Text(text) = &shape.shape {
                                (text.galley.job.text == label && viewport.contains(text.pos)).then(
                                    || (i, egui::Rect::from_min_size(text.pos, text.galley.size())),
                                )
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| format!("{label} is missing from the native viewport"))?;
                    if paint_index <= callback_index {
                        return Err(format!("{label} is painted behind the native solids").into());
                    }
                    if occupied.iter().any(|other| other.intersects(bounds)) {
                        return Err("Compound box labels overlap".into());
                    }
                    occupied.push(bounds);
                }
                for label in ["Add box", "Move box", "Resize box", "Remove box"] {
                    let visible = output.shapes.iter().any(|shape| {
                        if let egui::epaint::Shape::Text(text) = &shape.shape {
                            let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                            text.galley.job.text == label
                                && shape.clip_rect.contains(bounds.min)
                                && shape.clip_rect.contains(bounds.max)
                        } else {
                            false
                        }
                    });
                    if !visible {
                        return Err(format!("{label} is clipped in the compound inspector").into());
                    }
                }
                Ok(())
            };
            if let Err(error) = check() {
                output.textures_delta.clear();
                return Err(error);
            }
        }
        let ready = !architecture
            || app.current().scene.is_current(
                app.current().session.document().id(),
                app.current().session.revision(),
            ) && app.current().scene.visible().is_some();
        if architecture && ready && !fitted {
            app.current_mut().sketch.request_fit();
            fitted = true;
            fitted_at = Some(Instant::now());
        }
        if ready
            && index >= 5
            && (!architecture
                || fitted_at
                    .is_some_and(|time: Instant| time.elapsed() >= Duration::from_millis(500)))
        {
            if let Some(pos) = release.take() {
                events = pointer(pos, false);
            } else if let Some(label) = routes.pop_front() {
                initial_scene = initial_scene
                    .or_else(|| app.current().scene.visible().map(|scene| scene.stamp()));
                let pos = match control(&output, label) {
                    Ok(pos) => pos,
                    Err(error) => {
                        output.textures_delta.clear();
                        return Err(error);
                    }
                };
                events = pointer(pos, true);
                release = Some(pos);
            } else if matches!(review, Review::CompoundCutout(_)) && !compound_opened {
                let source = multicad_core::model::compound_cutouts(&file.document)
                    .next()
                    .ok_or("Project has no compound Cutout")?
                    .id;
                let active = app.current_mut();
                active.sketch.begin_feature_edit(
                    &active.session,
                    &active.resources,
                    &active.scene,
                    source,
                    multicad_core::commands::Channel::Ui,
                )?;
                active.sketch.request_fit();
                compound_opened = true;
                fitted_at = Some(Instant::now());
            } else if matches!(review, Review::CompoundCutout(_)) && !compound_preview_requested {
                let pos = control(&output, "Preview")?;
                events = pointer(pos, true);
                release = Some(pos);
                compound_preview_requested = true;
            } else {
                if stable == 0 {
                    events = vec![egui::Event::PointerGone];
                }
                if !matches!(review, Review::CompoundCutout(_))
                    || app
                        .current()
                        .sketch
                        .model_can_apply(&app.current().session, &app.current().resources)
                {
                    stable += 1;
                } else {
                    stable = 0;
                }
            }
        }
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
            renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
        }
        queue.submit(commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.free.clear();
        index += 1;
        if ready && routes.is_empty() && release.is_none() && events.is_empty() && stable >= 4 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let current_scene = app.current().scene.visible().map(|scene| scene.stamp());
    if architecture && current_scene != initial_scene {
        return Err("Category navigation regenerated the native scene".into());
    }
    if project_file::ContentHash::document(app.current().session.document())? != source_hash
        || app.current().session.revision() != 0
    {
        return Err("Offscreen navigation mutated the source".into());
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
    Ok(current_scene)
}
fn main() -> Result<(), Error> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(
        args.next()
            .ok_or("Usage: project_shell_review PROJECT.tcd OUTPUT_DIRECTORY")?,
    )
    .canonicalize()?;
    let output = PathBuf::from(args.next().ok_or("Missing output directory")?);
    let review_mode = match args.next() {
        None => 0,
        Some(arg) if arg == "--architecture" => 1,
        Some(arg) if arg == "--architecture-views" => 2,
        Some(arg) if arg == "--compound-cutout" => 3,
        _ => {
            return Err(
                "Expected optional --architecture, --architecture-views or --compound-cutout"
                    .into(),
            );
        }
    };
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
    let reviews: Vec<_> = if review_mode == 1 {
        std::iter::once(Review::Architecture(None))
            .chain(Category::ALL.map(|c| Review::Architecture(Some(c))))
            .collect()
    } else if review_mode == 2 {
        vec![Review::Architecture(None), Review::ArchitecturePlan]
    } else if review_mode == 3 {
        vec![Review::CompoundCutout(false), Review::CompoundCutout(true)]
    } else {
        vec![Review::Project]
    };
    for (size, density) in [([1100, 700], 1.), ([1360, 860], 1.), ([1360, 860], 2.)] {
        for review in &reviews {
            let name = format!(
                "{}-{}x{}-{}x.png",
                review.name(),
                size[0],
                size[1],
                density as u32
            );
            let destination = output.join(&name);
            let stamp = render(
                &device,
                &queue,
                &project,
                Capture {
                    source: &source,
                    output: &destination,
                    size,
                    density,
                    review: *review,
                },
            )?;
            images.push(serde_json::json!({"file":name,"logical_size":size,"pixels_per_point":density,"native_scene":stamp.map(|stamp|serde_json::json!({"document":stamp.document,"revision":stamp.revision,"generation":stamp.generation}))}));
            println!("Rendered {name}");
        }
    }
    let report = serde_json::json!({"source":source,"document":project.document.id(),"gpu":format!("{:?}",adapter.get_info()),"images":images,"evidence":"Offscreen rendering of production Application/egui controls with real project data. No native window or macOS interaction."});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
