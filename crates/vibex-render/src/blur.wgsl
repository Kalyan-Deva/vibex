// Vibex: separable Gaussian blur (one direction per pass), drawn as a
// full-screen triangle into a quarter-size target.

struct Params {
    // offset between taps, in uv units of the source texture
    step: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
    var w = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var c = textureSampleLevel(src, samp, v.uv, 0.0) * w[0];
    for (var k = 1; k < 5; k++) {
        let o = p.step * f32(k);
        c += (textureSampleLevel(src, samp, v.uv + o, 0.0) + textureSampleLevel(src, samp, v.uv - o, 0.0)) * w[k];
    }
    return c;
}
