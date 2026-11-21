// Embers: sparks rising from a flickering fire below the panel.
//
// Ported to WGSL for Dupin from campfire-embers.glsl in snedea/ghostty-themes
// (https://github.com/snedea/ghostty-themes/blob/219390c9402d8d1d21d19913557c8d59b863c796/shaders/campfire-embers.glsl).
// The terminal-only passes (dark mask, text bloom, vignette, grain) and the starfield are
// dropped; the embers rise through the panel's height and spread across its width.
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

// Additive light over the glass: premultiplied, the color is `c` itself.
fn glow(c: vec3f) -> vec4f {
    let a = clamp(max(max(c.r, c.g), c.b), 0.0, 1.0);
    return vec4f(c / max(a, 1e-4), a);
}

fn backdrop(p: vec2f, b: Backdrop) -> vec4f {
    let t = b.time;
    // y runs up from the panel's bottom edge, in units of the field's height.
    let uv = vec2f(p.x / b.field, (b.size.y - p.y) / b.field);
    let width = b.size.x / b.field;
    let height = b.size.y / b.field;

    var flicker = sin(t * 3.7) * 0.3 + sin(t * 7.1 + 1.0) * 0.15 + sin(t * 11.3 + 3.0) * 0.08;
    flicker = 0.92 + flicker * 0.08;

    let fire = vec2f(width * 0.5, -0.6);
    let fire_dist = distance(uv / vec2f(width * 0.5, 1.0), fire / vec2f(width * 0.5, 1.0));
    let fire_glow = smoothstep(1.6, 0.2, fire_dist);
    let fire_col = mix(vec3f(0.80, 0.25, 0.05), vec3f(0.50, 0.15, 0.02), smoothstep(0.1, 1.2, fire_dist));
    var col = fire_col * fire_glow * 0.35 * flicker;

    for (var i = 0; i < 40; i++) {
        let fi = f32(i);
        let seed1 = hash(vec2f(fi, 10.0));
        let seed2 = hash(vec2f(fi, 20.0));
        let seed3 = hash(vec2f(fi, 30.0));
        let speed = 0.15 + seed1 * 0.25;
        let life = fract(seed2 * 13.7 + t * speed);
        let start_x = (0.05 + 0.9 * hash(vec2f(fi, 40.0))) * width;
        let drift = sin(life * 6.28 + fi * 2.0) * (0.2 + seed3 * 0.3);
        let pos = vec2f(start_x + drift * life, life * height * 1.05 - 0.05);
        let d = distance(uv, pos);
        let size = mix(0.07, 0.025, life);
        let ember = smoothstep(size, size * 0.3, d);
        let ember_col = mix(vec3f(1.0, 0.6, 0.1), vec3f(0.6, 0.1, 0.0), life);
        let fade = 1.0 - smoothstep(0.7, 1.0, life);
        let sparkle = 0.5 + 0.5 * sin(t * (8.0 + seed3 * 12.0) + fi * 3.0);
        col += ember_col * ember * fade * sparkle * 0.9;
    }

    return glow(col * flicker * smoothstep(0.0, 0.4, t));
}
