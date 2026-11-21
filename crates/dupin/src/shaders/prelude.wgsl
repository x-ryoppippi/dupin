struct Backdrop {
    size: vec2f,
    time: f32,
    dark: f32,
    accent: vec3f,
    keystroke: f32,
    query: f32,
    searching: f32,
    mode: f32,
    field: f32,
}

struct DupinUniforms {
    rect: vec4f,
    clip: vec4f,
    fade: vec4<u32>,
    frame: vec4f,
    accent: vec4f,
    a: vec4f,
    b: vec4f,
}
@group(0) @binding(0) var<uniform> dupin_u: DupinUniforms;

struct DupinVertex { @builtin(position) pos: vec4f }

@vertex fn dupin_vs(@builtin(vertex_index) vi: u32) -> DupinVertex {
    let c = vec2f(f32(vi & 1u), f32(vi >> 1u));
    let l = (dupin_u.rect.xy + c * dupin_u.rect.zw - dupin_u.frame.xy) / dupin_u.frame.zw;
    return DupinVertex(vec4f(l.x * 2.0 - 1.0, 1.0 - l.y * 2.0, 0.0, 1.0));
}

fn dupin_decode(x: vec3f) -> vec3f {
    return select(pow((x + 0.055) / 1.055, vec3f(2.4)), x / 12.92, x <= vec3f(0.04045));
}
