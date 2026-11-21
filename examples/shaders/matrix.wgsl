// Matrix: columns of glyphs raining down the panel, each with a bright head and a fading trail.
// Written for Dupin. The glyphs are random 3×5 bit patterns, not a font, and flicker as the
// rain falls; each keystroke brightens the rain for a moment.
//
// SPDX-License-Identifier: GPL-3.0-or-later

const MATRIX_CELL = vec2f(11.0, 16.0);

fn matrix_hash(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453);
}

fn backdrop(p: vec2f, b: Backdrop) -> vec4f {
    let cell = floor(p / MATRIX_CELL);
    let local = fract(p / MATRIX_CELL);

    // Each column falls at its own speed, in rows per second, with its own trail length.
    let speed = 5.0 + matrix_hash(vec2f(cell.x, 1.0)) * 9.0;
    let trail = 6.0 + matrix_hash(vec2f(cell.x, 2.0)) * 14.0;
    let span = ceil(b.size.y / MATRIX_CELL.y) + trail + 4.0;
    let head = fract(matrix_hash(vec2f(cell.x, 3.0)) + b.time * speed / span) * span - 2.0;
    let behind = head - cell.y;
    if behind < 0.0 || behind > trail {
        return vec4f(0.0);
    }

    // The glyph: a 3×5 grid inside the cell, its bits changing a few times a second.
    let inner = (local - vec2f(0.18, 0.12)) / vec2f(0.64, 0.76);
    if any(inner < vec2f(0.0)) || any(inner >= vec2f(1.0)) {
        return vec4f(0.0);
    }
    let g = floor(inner * vec2f(3.0, 5.0));
    let bit = u32(g.y * 3.0 + g.x);
    let tick = floor(b.time * (1.5 + matrix_hash(cell) * 5.0));
    let bits = u32(matrix_hash(cell + vec2f(tick * 17.0, tick * 7.0)) * 32768.0);
    if ((bits >> bit) & 1u) == 0u {
        return vec4f(0.0);
    }

    let fade = pow(1.0 - behind / trail, 1.6);
    let lead = 1.0 - smoothstep(0.0, 1.0, behind);
    let pulse = exp(-b.keystroke * 4.0);
    let green = select(vec3f(0.0, 0.45, 0.18), vec3f(0.35, 1.0, 0.5), b.dark > 0.5);
    let ink = select(vec3f(0.0, 0.3, 0.1), vec3f(0.85, 1.0, 0.9), b.dark > 0.5);
    let color = mix(green, ink, lead);
    let alpha = (fade * 0.4 + lead * 0.35) * (1.0 + pulse * 0.6);
    return vec4f(color, clamp(alpha, 0.0, 1.0) * smoothstep(0.0, 0.4, b.time));
}
