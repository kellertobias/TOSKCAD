//! Cached instanced solid rendering in eframe's depth-enabled WGPU pass.
use super::{Geometry, Scene, SectionPlane};
use crate::{camera::Camera, navigation::DisplayStyle};
use eframe::{
    egui,
    egui_wgpu::{self, CallbackResources, CallbackTrait},
    wgpu::{self, util::DeviceExt},
};
use multicad_core::document::EntityId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceSelection {
    pub instance: EntityId,
    pub key: multicad_core::references::TopologyKey,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    References([Option<ReferenceSelection>; 2]),
    Part(EntityId),
    Parts(Arc<BTreeSet<EntityId>>),
    Face {
        instance: EntityId,
        ordinal: u32,
    },
    /// Several faces (CAD-152 Command/Control-click set), any instances.
    Faces(Arc<Vec<(EntityId, u32)>>),
    Edge {
        instance: EntityId,
        ordinal: u32,
    },
    Vertex {
        instance: EntityId,
        ordinal: u32,
    },
}
fn reference_code(reference: Option<ReferenceSelection>, instance: EntityId) -> u32 {
    reference
        .filter(|r| r.instance == instance)
        .map_or(u32::MAX, |r| {
            use multicad_core::references::TopologyKind;
            match r.key.kind {
                TopologyKind::Face => r.key.ordinal,
                TopologyKind::Edge => 0x1000_0000 + r.key.ordinal,
                TopologyKind::Vertex => 0x2000_0000 + r.key.ordinal,
            }
        })
}
struct Mesh {
    source: Arc<Geometry>,
    vertices: wgpu::Buffer,
    count: u32,
    edges: Option<wgpu::Buffer>,
    edge_count: u32,
}
struct Draw {
    geometry: EntityId,
    instances: wgpu::Buffer,
    count: u32,
}
struct Renderer {
    pipelines: [wgpu::RenderPipeline; 3],
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
            label: Some("ToskCAD solids"),
            source: wgpu::ShaderSource::Wgsl(include_str!("solid.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAD camera"),
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CAD camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
        let pipeline = |topology, entry, compare, write, bias| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("CAD instanced geometry"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vertex"),compilation_options:Default::default(),buffers:&[
                Some(wgpu::VertexBufferLayout {array_stride:32,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Uint32,7=>Float32x4]}),
                Some(wgpu::VertexBufferLayout {array_stride:80,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Uint32x4]})]},
            primitive:wgpu::PrimitiveState {topology,..Default::default()},depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(write),depth_compare:Some(compare),stencil:Default::default(),bias}),
            multisample:Default::default(),fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some(entry),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:None,
        })
        };
        let pipelines = [
            pipeline(
                wgpu::PrimitiveTopology::TriangleList,
                "fragment",
                wgpu::CompareFunction::LessEqual,
                true,
                Default::default(),
            ),
            pipeline(
                wgpu::PrimitiveTopology::LineList,
                "line_fragment",
                wgpu::CompareFunction::LessEqual,
                false,
                Default::default(),
            ),
            pipeline(
                wgpu::PrimitiveTopology::LineList,
                "line_fragment",
                wgpu::CompareFunction::Always,
                false,
                Default::default(),
            ),
        ];
        Self {
            pipelines,
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
        view: View,
    ) {
        let View {
            camera,
            size,
            selection,
            style,
            section,
        } = view;
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
                let mut bytes = Vec::with_capacity(geometry.indices().len() * 32);
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
                        floats(
                            &mut bytes,
                            geometry
                                .face_color(*face)
                                .map_or([0., 0., 0., -1.], |c| c.map(|v| v as f32 / 255.)),
                        );
                    }
                }
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cached CAD mesh"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let mut edge_bytes = vec![];
                for edge in geometry.edges() {
                    for segment in edge.points_mm.windows(2) {
                        for p in segment {
                            floats(&mut edge_bytes, p.map(|v| v as f32));
                            words(&mut edge_bytes, [0x1000_0000 + edge.index]);
                            floats(&mut edge_bytes, [0., 0., 0., -1.]);
                        }
                    }
                }
                let edge_count = (edge_bytes.len() / 32) as u32;
                let edges = (!edge_bytes.is_empty()).then(|| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Cached B-rep edges"),
                        contents: &edge_bytes,
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                });
                self.meshes.insert(
                    *id,
                    Mesh {
                        source: geometry.clone(),
                        vertices: buffer,
                        count: geometry.indices().len() as u32,
                        edges,
                        edge_count,
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
                let mut records = 0;
                for instance in &instances {
                    records += 1;
                    for row in instance.transform.rows() {
                        floats(&mut bytes, row);
                    }
                    floats(&mut bytes, instance.color.map(|v| v as f32 / 255.));
                    let (face, part) = match &selection {
                        Some(Selection::Part(id)) if *id == instance.id => (u32::MAX, 1),
                        Some(Selection::Parts(ids)) if ids.contains(&instance.id) => (u32::MAX, 1),
                        Some(Selection::Face {
                            instance: id,
                            ordinal,
                        }) if *id == instance.id => (*ordinal, 0),
                        Some(Selection::Edge {
                            instance: id,
                            ordinal,
                        }) if *id == instance.id => (0x1000_0000 + ordinal, 0),
                        Some(Selection::Vertex {
                            instance: id,
                            ordinal,
                        }) if *id == instance.id => (0x2000_0000 + ordinal, 0),
                        _ => (u32::MAX, 0),
                    };
                    let mut emphasis = [face, part, u32::MAX, 0];
                    if let Some(Selection::References(references)) = &selection {
                        emphasis = [
                            reference_code(references[0], instance.id),
                            0,
                            reference_code(references[1], instance.id),
                            0,
                        ];
                    }
                    let mut extra = vec![];
                    if let Some(Selection::Faces(faces)) = &selection {
                        let mut own = faces
                            .iter()
                            .filter(|(id, _)| *id == instance.id)
                            .map(|(_, ordinal)| *ordinal);
                        emphasis[0] = own.next().unwrap_or(u32::MAX);
                        extra.extend(own);
                    }
                    words(&mut bytes, emphasis);
                    // Further faces of one instance are highlight-only overlay
                    // records (w = 1): identical placement at LessEqual depth,
                    // every other triangle and edge is discarded in the shader.
                    for ordinal in extra {
                        for row in instance.transform.rows() {
                            floats(&mut bytes, row);
                        }
                        floats(&mut bytes, instance.color.map(|v| v as f32 / 255.));
                        words(&mut bytes, [ordinal, 0, u32::MAX, 1]);
                        records += 1;
                    }
                }
                self.draws.push(Draw {
                    geometry: id,
                    count: records,
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
        let mut near = 0.1;
        let mut far = distance + 1.;
        if let Some([low, high]) = scene.world_bounds() {
            let mut nearest = f32::INFINITY;
            let mut farthest = f32::NEG_INFINITY;
            for corner in 0..8 {
                let point: [f32; 3] = std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        low[i] as f32
                    } else {
                        high[i] as f32
                    }
                });
                let depth = distance
                    - (0..3)
                        .map(|i| (point[i] - camera.target[i]) * forward[i])
                        .sum::<f32>();
                nearest = nearest.min(depth);
                farthest = farthest.max(depth);
            }
            // Fit the perspective depth range to the scene. A near plane at
            // the eye otherwise gives distant surfaces almost the same depth.
            if camera.perspective {
                near = (nearest - 1.).max(near);
                far = (farthest + 1.).max(near + 1.);
            } else {
                far = far.max(farthest + 1.);
            }
        }
        let mut bytes = Vec::with_capacity(112);
        floats(&mut bytes, camera.target.into_iter().chain([0.]));
        floats(&mut bytes, [c, -s, 0., 0.]);
        floats(&mut bytes, [-sp * s, -sp * c, cp, 0.]);
        floats(&mut bytes, forward.into_iter().chain([0.]));
        floats(
            &mut bytes,
            [size.x.max(1.), size.y.max(1.), camera.scale, distance],
        );
        floats(
            &mut bytes,
            [
                near,
                far,
                f32::from(camera.perspective),
                f32::from(style == DisplayStyle::Wire),
            ],
        );
        floats(&mut bytes, section.map_or([0.; 4], SectionPlane::uniform));
        queue.write_buffer(&self.uniform, 0, &bytes);
    }
    fn paint(&self, pass: &mut wgpu::RenderPass<'_>, style: DisplayStyle) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        if style != DisplayStyle::Wire {
            pass.set_pipeline(&self.pipelines[0]);
            for draw in &self.draws {
                let mesh = &self.meshes[&draw.geometry];
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_vertex_buffer(1, draw.instances.slice(..));
                pass.draw(0..mesh.count, 0..draw.count);
            }
        }
        if style != DisplayStyle::Filled {
            pass.set_pipeline(
                &self.pipelines[if style == DisplayStyle::VisibleEdges {
                    1
                } else {
                    2
                }],
            );
            for draw in &self.draws {
                let mesh = &self.meshes[&draw.geometry];
                if let Some(edges) = &mesh.edges {
                    pass.set_vertex_buffer(0, edges.slice(..));
                    pass.set_vertex_buffer(1, draw.instances.slice(..));
                    pass.draw(0..mesh.edge_count, 0..draw.count);
                }
            }
        }
    }
}
/// Install the shared native viewport pipelines for a depth-enabled egui renderer.
/// Offscreen hosts use the same resources as native windows; the render pass
/// must provide a Depth32Float attachment.
pub fn install_renderer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    resources: &mut CallbackResources,
) {
    resources.insert(Renderer::new(device, format));
}
pub(crate) fn install(state: &egui_wgpu::RenderState) {
    install_renderer(
        &state.device,
        state.target_format,
        &mut state.renderer.write().callback_resources,
    );
}
#[derive(Clone)]
struct View {
    camera: Camera,
    size: egui::Vec2,
    selection: Option<Selection>,
    style: DisplayStyle,
    section: Option<SectionPlane>,
}
struct Callback {
    scene: Arc<Scene>,
    view: View,
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
            renderer.prepare(device, queue, &self.scene, self.view.clone());
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
            renderer.paint(pass, self.view.style);
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
    style: DisplayStyle,
) {
    paint_scene_section(painter, rect, scene, camera, selection, style, None);
}
pub fn paint_scene_section(
    painter: &egui::Painter,
    rect: egui::Rect,
    scene: Arc<Scene>,
    camera: Camera,
    selection: Option<Selection>,
    style: DisplayStyle,
    section: Option<SectionPlane>,
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
    crate::preview_capture::record_viewport(painter.ctx(), rect);
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        Callback {
            scene,
            view: View {
                camera,
                size: rect.size(),
                selection,
                style,
                section,
            },
        },
    ));
}

#[cfg(test)]
mod tests;
