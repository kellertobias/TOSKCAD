struct Camera {
    focus: vec4<f32>, right: vec4<f32>, up: vec4<f32>, forward: vec4<f32>,
    viewport: vec4<f32>, clip: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
struct Vertex {
    @location(0) position: vec3<f32>, @location(1) face: u32,
    @location(2) row0: vec4<f32>, @location(3) row1: vec4<f32>, @location(4) row2: vec4<f32>,
    @location(5) color: vec4<f32>, @location(6) selection: vec4<u32>,
}
struct Fragment {
    @builtin(position) position: vec4<f32>, @location(0) world: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@vertex fn vertex(v: Vertex) -> Fragment {
    let local = vec4(v.position, 1.0);
    let world = vec3(dot(v.row0, local), dot(v.row1, local), dot(v.row2, local));
    let relative = world - camera.focus.xyz;
    let x = dot(relative, camera.right.xyz) * camera.viewport.z * 2.0 / camera.viewport.x;
    let y = dot(relative, camera.up.xyz) * camera.viewport.z * 2.0 / camera.viewport.y;
    let z = camera.viewport.w - dot(relative, camera.forward.xyz);
    var w = 1.0;
    var depth = (z - camera.clip.x) / (camera.clip.y - camera.clip.x);
    if camera.clip.z > 0.5 {
        w = z / camera.viewport.w;
        depth = camera.clip.y / (camera.clip.y - camera.clip.x) * (z - camera.clip.x) / camera.viewport.w;
    }
    var out: Fragment;
    out.position = vec4(x, y, depth, w);
    out.world = world;
    out.color = v.color;
    if v.selection.y == 1u || v.selection.x == v.face {out.color = vec4(0.95,0.65,0.18,1.0);}
    return out;
}
@fragment fn fragment(v: Fragment) -> @location(0) vec4<f32> {
    let normal = normalize(cross(dpdx(v.world),dpdy(v.world)));
    let light = 0.28 + 0.72 * abs(dot(normal,normalize(vec3(0.4,0.6,1.0))));
    return vec4(v.color.rgb * light,1.0);
}
