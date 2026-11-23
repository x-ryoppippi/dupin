# Backdrop shaders

Shaders for the animation behind Dupin's panel. Pick one in Settings (⌘, in the panel), which copies it to the `shaders` folder beside `config.toml` for you to edit. Settings also lists every other `.wgsl` file in that folder, so a shader of your own goes there too.

A new shader here also goes in `EXAMPLES` in [`crates/dupin/src/shader.rs`](../../crates/dupin/src/shader.rs); a test checks the two agree.

Dupin reads the file again when it changes, so saving it and pressing the hotkey shows the new version. If it doesn't compile, the panel says why and keeps the last version that did.

| Shader                       | What it draws                                           | Source                                                                                      |
| ---------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [`aurora.wgsl`](aurora.wgsl) | Green and purple curtains of light with twinkling stars | Ported from [snedea/ghostty-themes](https://github.com/snedea/ghostty-themes) (MIT)         |
| [`embers.wgsl`](embers.wgsl) | Sparks rising from a flickering fire                    | Ported from [snedea/ghostty-themes](https://github.com/snedea/ghostty-themes) (MIT)         |
| [`snow.wgsl`](snow.wgsl)     | Flakes drifting down in three parallax layers           | Ported from [an0sunshy/ghostty-shaders](https://github.com/an0sunshy/ghostty-shaders) (MIT) |
| [`matrix.wgsl`](matrix.wgsl) | Columns of glyphs raining down, brightening as you type | Dupin                                                                                       |

The built-in `flow` shader is [`crates/dupin/src/shaders/flow.wgsl`](../../crates/dupin/src/shaders/flow.wgsl). The ported shaders keep their MIT notices in their headers.

## Writing one

A backdrop shader is WGSL that defines one function:

```wgsl
fn backdrop(p: vec2f, b: Backdrop) -> vec4f
```

`p` is the pixel's position in points from the panel's top-left corner. Return a straight (not premultiplied) sRGB color and its alpha. Dupin premultiplies it, clips it to the panel's corners and blends it over the glass, under the text.

`b` holds:

| Field       | Type    | Meaning                                                                                                  |
| ----------- | ------- | -------------------------------------------------------------------------------------------------------- |
| `size`      | `vec2f` | The panel's size in points; its height changes with the number of results                                |
| `time`      | `f32`   | Seconds since the panel showed, for an entrance; held still under Reduce Motion                          |
| `dark`      | `f32`   | 1 in a dark theme, 0 in a light one                                                                      |
| `accent`    | `vec3f` | The theme's accent color, sRGB                                                                           |
| `keystroke` | `f32`   | Seconds since the query last changed, up to 10000; 10000 before the first change and under Reduce Motion |
| `query`     | `f32`   | Characters in the query                                                                                  |
| `searching` | `f32`   | 1 while a search runs, 0 under Reduce Motion                                                             |
| `mode`      | `f32`   | 0 searching apps, 1 file names, 2 file contents, 3 Dictionary, 5 calendar events                         |
| `field`     | `f32`   | The query field's height in points, at the panel's top; a steady unit to scale by                        |

Dupin defines these names too, so a shader can't reuse them: `Backdrop`, anything starting with `dupin_`, and Rune's clip helpers `clip_alpha`, `ramp`, `sd_rounded`, `sd_continuous`, `unpack_clip_radii` and `CONTINUOUS_K`.

A shader reads only `b`: it can't declare an `override` or bind resources of its own. Dupin checks a file as Metal will draw it, and refuses one Metal can't compile or draw, such as a function whose local arrays pass the GPU's stack. A file can be at most 32 KB.

The shader runs for every pixel of the panel on every frame while the panel shows, so keep loops short.
