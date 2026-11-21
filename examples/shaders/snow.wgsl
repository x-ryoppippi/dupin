// Snow: soft flakes drifting down in three parallax layers.
//
// Ported to WGSL for Dupin from shaders/weather/snow.glsl in an0sunshy/ghostty-shaders
// (https://github.com/an0sunshy/ghostty-shaders/blob/8a8484ef7b1c8c2cd600d58e86d574a3ac7750ee/shaders/weather/snow.glsl).
// The terminal compositing and the day/night dimming are dropped, the flakes are denser for
// a short field, and they darken instead of lighten on a light theme.
//
// Copyright (c) 2026 an0sunshy
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

fn snow_hash(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453);
}

fn snowflake(lp: vec2f, jitter: f32) -> f32 {
    let r = length(lp);
    let core = smoothstep(0.04 * jitter, 0.01 * jitter, r);
    let halo = smoothstep(0.08 * jitter, 0.04 * jitter, r) * 0.22;
    return core + halo;
}

fn snow_layer(uv: vec2f, t: f32, speed: f32, density: f32, scale: f32, seed: f32) -> f32 {
    var q = uv * scale + seed;
    // Flakes fall down the screen, which runs down in y here.
    q.y -= (t % 100.0) * speed;
    let cell = floor(q);
    let h = snow_hash(cell);
    if h < 1.0 - density {
        return 0.0;
    }
    var f = fract(q);
    f.x += sin(q.y * 0.4 + h * 6.28318) * 0.25;
    let center = vec2f(fract(h * 17.3), fract(h * 31.7));
    return snowflake(f - center, mix(0.7, 1.3, fract(h * 53.1)));
}

fn backdrop(p: vec2f, b: Backdrop) -> vec4f {
    let uv = p / 400.0;
    var s = snow_layer(uv, b.time, 0.4, 0.10, 12.0, 0.0) * 0.4;
    s += snow_layer(uv, b.time, 0.6, 0.12, 18.0, 3.7) * 0.6;
    s += snow_layer(uv, b.time, 0.9, 0.14, 24.0, 7.3) * 0.8;
    let color = select(vec3f(0.35, 0.42, 0.52), vec3f(0.80, 0.86, 0.95), b.dark > 0.5);
    return vec4f(color, clamp(s, 0.0, 1.0) * 0.8 * smoothstep(0.0, 0.4, b.time));
}
