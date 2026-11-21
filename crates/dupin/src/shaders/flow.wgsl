// Dupin's built-in backdrop: the accent color flowing as warped noise. Each keystroke swells
// it for a moment, and a band sweeps across while a file search runs.

fn hash(p: vec2f) -> f32 {
    let q = fract(p * vec2f(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2f(1.0, 0.0));
    let c = hash(i + vec2f(0.0, 1.0));
    let d = hash(i + vec2f(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p: vec2f) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * noise(q);
        q = mat2x2f(1.6, 1.2, -1.2, 1.6) * q;
        a *= 0.5;
    }
    return v;
}

fn backdrop(p: vec2f, b: Backdrop) -> vec4f {
    let t = b.time;
    let pulse = exp(-b.keystroke * 3.0);

    let q = p / 110.0;
    let warp = vec2f(
        fbm(q + vec2f(0.0, t * 0.13)),
        fbm(q + vec2f(5.2, 1.3) - vec2f(t * 0.11, 0.0)),
    );
    let n = fbm(q + (2.2 + 0.8 * pulse) * warp + vec2f(t * 0.05, -t * 0.03));

    let rgb = mix(b.accent, b.accent.brg, smoothstep(0.3, 0.7, warp.x));
    let strength = select(0.30, 0.42, b.dark > 0.5) * (1.0 + 0.6 * pulse);
    var alpha = smoothstep(0.32 - 0.1 * pulse, 0.72, n) * strength;

    let sweep = fract(t * 0.6) * (b.size.x + 240.0) - 120.0;
    alpha += b.searching * 0.12 * exp(-pow((p.x - sweep) / 80.0, 2.0));

    // Fade in as the panel appears.
    return vec4f(rgb, alpha * smoothstep(0.0, 0.35, t));
}
