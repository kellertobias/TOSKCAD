struct Camera {
    focus: vec4<f32>, right: vec4<f32>, up: vec4<f32>, forward: vec4<f32>,
    viewport: vec4<f32>, clip: vec4<f32>, section: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
struct Vertex {
    @location(0) position: vec3<f32>, @location(1) face: u32,
    @location(2) row0: vec4<f32>, @location(3) row1: vec4<f32>, @location(4) row2: vec4<f32>,
    @location(7) material: vec4<f32>,
    @location(5) color: vec4<f32>, @location(6) selection: vec4<u32>,
}
struct Fragment {
    @builtin(position) position: vec4<f32>, @location(0) world: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) emphasis: u32,
}
fn project(world: vec3<f32>, bias_edge: bool) -> vec4<f32> {
    let relative = world - camera.focus.xyz;
    let x = dot(relative, camera.right.xyz) * camera.viewport.z * 2.0 / camera.viewport.x;
    let y = dot(relative, camera.up.xyz) * camera.viewport.z * 2.0 / camera.viewport.y;
    let z = camera.viewport.w - dot(relative, camera.forward.xyz);
    // Give a coincident edge a small camera-space allowance. A fixed offset
    // after projection can pull edges through distant surfaces in perspective.
    let edge = bias_edge && z > camera.clip.x;
    let test_z = select(z, max(camera.clip.x, z - 0.05), edge);
    let span = camera.clip.y - camera.clip.x;
    var w = 1.0;
    var depth = (test_z - camera.clip.x) / span;
    if camera.clip.z > 0.5 {
        w = z / camera.viewport.w;
        depth *= camera.clip.y / camera.viewport.w;
        if edge { depth *= z / test_z; }
    }
    return vec4(x, y, depth, w);
}
@vertex fn vertex(v: Vertex) -> Fragment {
    let local = vec4(v.position, 1.0);
    let world = vec3(dot(v.row0, local), dot(v.row1, local), dot(v.row2, local));
    var out: Fragment;
    out.position = project(world, v.face >= 0x10000000u);
    out.world = world;
    out.color = select(v.color, v.material, v.material.a >= 0.0);
    out.emphasis = 0u;
    if v.selection.y == 1u || v.selection.x == v.face {out.color = vec4(0.95,0.65,0.18,1.0);out.emphasis=1u;}
    if v.selection.z == v.face {out.color=vec4(0.2,0.6,0.95,1.0);out.emphasis=2u;}
    if v.selection.w == 1u && v.selection.x != v.face {out.emphasis=3u;}
    return out;
}
struct AxisFragment {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}
@vertex fn axis_vertex(@location(0) position: vec3<f32>, @location(1) color: vec3<f32>) -> AxisFragment {
    return AxisFragment(project(position, false), color);
}
@fragment fn axis_fragment(v: AxisFragment) -> @location(0) vec4<f32> {
    return vec4(v.color, 1.0);
}
@fragment fn line_fragment(v:Fragment)->@location(0) vec4<f32> {
    if dot(camera.section.xyz, v.world) < camera.section.w { discard; }
    if v.emphasis==3u { discard; }
    if v.emphasis==1u {return vec4(0.95,0.65,0.18,1.0);}
    if v.emphasis==2u {return vec4(0.2,0.6,0.95,1.0); }
    if camera.clip.w>0.5 {return vec4(0.53,0.66,0.79,1.0);}
    return vec4(0.07,0.1,0.15,1.0);
}
@fragment fn fragment(v: Fragment) -> @location(0) vec4<f32> {
    let normal = normalize(cross(dpdx(v.world),dpdy(v.world)));
    if dot(camera.section.xyz, v.world) < camera.section.w { discard; }
    if v.emphasis==3u { discard; }
    let light = 0.28 + 0.72 * abs(dot(normal,normalize(vec3(0.4,0.6,1.0))));
    return vec4(v.color.rgb * light,1.0);
}
