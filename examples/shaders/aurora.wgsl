// Aurora: curtains of green and purple light with twinkling stars.
//
// Ported to WGSL for Dupin from aurora-shimmer.glsl in snedea/ghostty-themes
// (https://github.com/snedea/ghostty-themes/blob/219390c9402d8d1d21d19913557c8d59b863c796/shaders/aurora-shimmer.glsl).
// The terminal-only passes (dark mask, text bloom, vignette, grain) are dropped, and the
// panel is treated as a band of a sky as wide as it is.
//
// Copyright (c) 2026 snedea
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

fn hash(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2f(12.9898, 78.233))) * 43758.5453);
}

fn noise(p: vec2f) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2f(1.0, 0.0));
    let c = hash(i + vec2f(0.0, 1.0));
    let d = hash(i + vec2f(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn fbm(p: vec2f) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * noise(q);
        q *= 2.1;
        a *= 0.48;
    }
    return v;
}

// Additive light over the glass: premultiplied, the color is `c` itself.
fn glow(c: vec3f) -> vec4f {
    let a = clamp(max(max(c.r, c.g), c.b), 0.0, 1.0);
    return vec4f(c / max(a, 1e-4), a);
}

fn backdrop(p: vec2f, b: Backdrop) -> vec4f {
    let uv = vec2f(p.x / b.size.x, 0.55 + p.y / b.size.x);

    let t = b.time * 0.06;
    let curtain1 = fbm(vec2f(uv.x * 3.0 + t, uv.y * 0.8 + t * 0.3));
    let curtain2 = fbm(vec2f(uv.x * 2.5 - t * 0.7, uv.y * 1.2 + t * 0.2 + 5.0));
    let curtain3 = fbm(vec2f(uv.x * 4.0 + t * 0.4, uv.y * 0.6 - t * 0.1 + 10.0));

    let green = vec3f(0.15, 0.85, 0.35);
    let purple = vec3f(0.45, 0.15, 0.65);
    let teal = vec3f(0.10, 0.60, 0.55);
    var aurora = green * curtain1 * 0.50 + purple * curtain2 * 0.35 + teal * curtain3 * 0.25;
    let shimmer = noise(vec2f(uv.x * 20.0, uv.y * 8.0 + b.time * 0.8));
    aurora *= 0.85 + 0.30 * shimmer;
    var col = aurora * 0.55;

    let star_grid = 80.0;
    let cell = floor(uv * star_grid);
    let star = hash(cell);
    let is_star = step(0.97, star);
    let twinkle = 0.5 + 0.5 * sin(b.time * (2.0 + star * 4.0) + star * 50.0);
    let dot_ = smoothstep(0.12, 0.0, length(fract(uv * star_grid) - 0.5));
    col += vec3f(0.7, 0.8, 1.0) * dot_ * is_star * twinkle * 0.6;

    return glow(col * smoothstep(0.0, 0.5, b.time));
}
