@fragment fn dupin_fs(v: DupinVertex) -> @location(0) vec4f {
    let win = v.pos.xy + dupin_u.frame.xy;
    let scale = dupin_u.a.y;
    let b = Backdrop(
        dupin_u.rect.zw / scale,
        dupin_u.a.x,
        dupin_u.accent.w,
        dupin_u.accent.rgb,
        dupin_u.a.w,
        dupin_u.b.x,
        dupin_u.b.y,
        dupin_u.b.z,
        dupin_u.b.w,
    );
    let c = backdrop((win - dupin_u.rect.xy) / scale, b);
    let alpha = clamp(c.a, 0.0, 1.0) * clip_alpha(win, dupin_u.clip, dupin_u.fade);
    var rgb = clamp(c.rgb, vec3f(0.0), vec3f(1.0));
    if dupin_u.a.z < 0.5 {
        rgb = dupin_decode(rgb);
    }
    return vec4f(rgb * alpha, alpha);
}
