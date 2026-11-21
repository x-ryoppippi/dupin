// Ripple: each keystroke drops a ring where the text ends, on a slow wash of the accent.
// Written for Dupin; shows the inputs besides time (keystroke, query, mode).
//
// SPDX-License-Identifier: GPL-3.0-or-later

fn backdrop(p: vec2f, b: Backdrop) -> vec4f {
    // About where the caret is: the text starts some 46 pt in, glyphs are some 11 pt wide.
    let caret = vec2f(min(46.0 + b.query * 11.0, b.size.x - 20.0), b.field * 0.5);
    let d = distance(p, caret);

    let r = b.keystroke * 420.0;
    let off = (d - r) / (6.0 + b.keystroke * 30.0);
    let ring = exp(-off * off) * exp(-b.keystroke * 2.2);
    let wash = 0.5 + 0.5 * sin(p.x / 90.0 - b.time * 0.7 + sin(p.y / 20.0 + b.time) * 0.6);

    // Files and contents searches lean the hue, so the mode reads at a glance.
    let tint = mix(b.accent, b.accent.gbr, clamp(b.mode, 0.0, 1.0) * 0.5);
    let strength = select(0.18, 0.26, b.dark > 0.5);
    let alpha = (wash * 0.35 + ring * 1.4) * strength;
    return vec4f(tint, alpha * smoothstep(0.0, 0.3, b.time));
}
