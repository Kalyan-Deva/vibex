// Vibex: one instanced quad pipeline for rects and glyphs.

struct Uniforms {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) color: vec4<f32>,
    // 0 = solid rect, 1 = mono glyph (coverage in alpha), 2 = colour glyph
    @location(5) kind: u32,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Instance) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let p = inst.pos + c * inst.size;
    var o: VOut;
    o.pos = vec4<f32>(p.x / u.screen.x * 2.0 - 1.0, 1.0 - p.y / u.screen.y * 2.0, 0.0, 1.0);
    o.uv = mix(inst.uv0, inst.uv1, c);
    o.color = inst.color;
    o.kind = inst.kind;
    return o;
}

// Output is premultiplied alpha.
@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    if (i.kind == 0u) {
        return vec4<f32>(i.color.rgb * i.color.a, i.color.a);
    }
    let t = textureSampleLevel(atlas, samp, i.uv, 0.0);
    if (i.kind == 1u) {
        let a = t.a * i.color.a;
        return vec4<f32>(i.color.rgb * a, a);
    }
    return t * i.color.a;
}
