//! Cached instanced solid rendering in eframe's depth-enabled WGPU pass.
use super::{Geometry, Scene};
use crate::camera::Camera;
use eframe::{
    egui,
    egui_wgpu::{self, CallbackResources, CallbackTrait},
    wgpu::{self, util::DeviceExt},
};
use multicad_core::document::EntityId;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Part(EntityId),
    Face { instance: EntityId, ordinal: u32 },
}
struct Mesh {
    source: Arc<Geometry>,
    vertices: wgpu::Buffer,
    count: u32,
}
struct Draw {
    geometry: EntityId,
    instances: wgpu::Buffer,
    count: u32,
}
struct Renderer {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    meshes: BTreeMap<EntityId, Mesh>,
    draws: Vec<Draw>,
    scene: Option<Arc<Scene>>,
    selection: Option<Selection>,
}
fn floats(bytes: &mut Vec<u8>, values: impl IntoIterator<Item = f32>) {
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
}
fn words(bytes: &mut Vec<u8>, values: impl IntoIterator<Item = u32>) {
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
}
impl Renderer {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MultiCAD solids"),
            source: wgpu::ShaderSource::Wgsl(include_str!("solid.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAD camera"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CAD camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CAD camera"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CAD solids layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("CAD depth-tested instanced solids"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vertex"),compilation_options:Default::default(),buffers:&[
                Some(wgpu::VertexBufferLayout {array_stride:16,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Uint32]}),
                Some(wgpu::VertexBufferLayout {array_stride:80,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Uint32x4]})]},
            primitive:Default::default(),depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::LessEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fragment"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:None,
        });
        Self {
            pipeline,
            uniform,
            bind_group,
            meshes: BTreeMap::new(),
            draws: vec![],
            scene: None,
            selection: None,
        }
    }
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Arc<Scene>,
        camera: Camera,
        size: egui::Vec2,
        selection: Option<Selection>,
    ) {
        if !self.scene.as_ref().is_some_and(|s| Arc::ptr_eq(s, scene))
            || self.selection != selection
        {
            self.meshes
                .retain(|id, _| scene.geometries().contains_key(id));
            for (id, geometry) in scene.geometries() {
                if self
                    .meshes
                    .get(id)
                    .is_some_and(|m| Arc::ptr_eq(&m.source, geometry))
                {
                    continue;
                }
                let mut bytes = Vec::with_capacity(geometry.indices().len() * 16);
                for (triangle, face) in geometry
                    .indices()
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .zip(geometry.triangle_faces())
                {
                    for index in triangle {
                        floats(
                            &mut bytes,
                            geometry.vertices()[*index as usize].map(|v| v as f32),
                        );
                        words(&mut bytes, [*face]);
                    }
                }
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cached CAD mesh"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                self.meshes.insert(
                    *id,
                    Mesh {
                        source: geometry.clone(),
                        vertices: buffer,
                        count: geometry.indices().len() as u32,
                    },
                );
            }
            self.draws.clear();
            let mut groups: BTreeMap<EntityId, Vec<_>> = BTreeMap::new();
            for instance in scene.instances() {
                groups.entry(instance.geometry).or_default().push(instance);
            }
            for (id, instances) in groups {
                let mut bytes = Vec::with_capacity(instances.len() * 80);
                for instance in &instances {
                    for row in instance.transform.rows() {
                        floats(&mut bytes, row);
                    }
                    floats(&mut bytes, instance.color.map(|v| v as f32 / 255.));
                    let (face, part) = match selection {
                        Some(Selection::Part(id)) if id == instance.id => (u32::MAX, 1),
                        Some(Selection::Face {
                            instance: id,
                            ordinal,
                        }) if id == instance.id => (ordinal, 0),
                        _ => (u32::MAX, 0),
                    };
                    words(&mut bytes, [face, part, 0, 0]);
                }
                self.draws.push(Draw {
                    geometry: id,
                    count: instances.len() as u32,
                    instances: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("CAD instance placements"),
                        contents: &bytes,
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                });
            }
            self.scene = Some(scene.clone());
            self.selection = selection;
        }
        let (s, c) = camera.yaw.sin_cos();
        let (sp, cp) = camera.pitch.sin_cos();
        let forward = [cp * s, cp * c, sp];
        let distance = size.y.max(1.) / camera.scale * 2.;
        let mut far = distance + 1.;
        if let Some([low, high]) = scene.world_bounds() {
            for corner in 0..8 {
                let point: [f32; 3] = std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        low[i] as f32
                    } else {
                        high[i] as f32
                    }
                });
                far = far.max(
                    distance
                        - (0..3)
                            .map(|i| (point[i] - camera.target[i]) * forward[i])
                            .sum::<f32>()
                        + 1.,
                );
            }
        }
        let mut bytes = Vec::with_capacity(96);
        floats(&mut bytes, camera.target.into_iter().chain([0.]));
        floats(&mut bytes, [c, -s, 0., 0.]);
        floats(&mut bytes, [-sp * s, -sp * c, cp, 0.]);
        floats(&mut bytes, forward.into_iter().chain([0.]));
        floats(
            &mut bytes,
            [size.x.max(1.), size.y.max(1.), camera.scale, distance],
        );
        floats(&mut bytes, [0.1, far, f32::from(camera.perspective), 0.]);
        queue.write_buffer(&self.uniform, 0, &bytes);
    }
    fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        for draw in &self.draws {
            let mesh = &self.meshes[&draw.geometry];
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(1, draw.instances.slice(..));
            pass.draw(0..mesh.count, 0..draw.count);
        }
    }
}
pub(crate) fn install(state: &egui_wgpu::RenderState) {
    state
        .renderer
        .write()
        .callback_resources
        .insert(Renderer::new(&state.device, state.target_format));
}
struct Callback {
    scene: Arc<Scene>,
    camera: Camera,
    size: egui::Vec2,
    selection: Option<Selection>,
}
impl CallbackTrait for Callback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &egui_wgpu::ScreenDescriptor,
        _: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = resources.get_mut::<Renderer>() {
            renderer.prepare(
                device,
                queue,
                &self.scene,
                self.camera,
                self.size,
                self.selection,
            );
        }
        vec![]
    }
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<Renderer>() {
            renderer.paint(pass);
        }
    }
}
/// One depth-enabled solid viewport per native window. Grid/tool overlays can
/// follow this callback in painter order. Geometry buffers survive camera motion.
pub fn paint_scene(
    painter: &egui::Painter,
    rect: egui::Rect,
    scene: Arc<Scene>,
    camera: Camera,
    selection: Option<Selection>,
) {
    if !rect.is_finite()
        || rect.width() <= 0.
        || rect.height() <= 0.
        || !camera.scale.is_finite()
        || camera.scale <= 0.
        || camera
            .target
            .iter()
            .chain([&camera.yaw, &camera.pitch])
            .any(|v| !v.is_finite())
    {
        return;
    }
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        Callback {
            scene,
            camera,
            size: rect.size(),
            selection,
        },
    ));
}

#[cfg(test)]
mod tests;
