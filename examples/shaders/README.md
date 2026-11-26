# Backdrop shaders

Example shaders for the animation behind Dupin's panel. Turning one on, and writing your own, is in [Shaders](../../docs/features/shaders.md).

| Shader                       | What it draws                                           | Source                                                                                      |
| ---------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [`aurora.wgsl`](aurora.wgsl) | Green and purple curtains of light with twinkling stars | Ported from [snedea/ghostty-themes](https://github.com/snedea/ghostty-themes) (MIT)         |
| [`embers.wgsl`](embers.wgsl) | Sparks rising from a flickering fire                    | Ported from [snedea/ghostty-themes](https://github.com/snedea/ghostty-themes) (MIT)         |
| [`snow.wgsl`](snow.wgsl)     | Flakes drifting down in three parallax layers           | Ported from [an0sunshy/ghostty-shaders](https://github.com/an0sunshy/ghostty-shaders) (MIT) |
| [`matrix.wgsl`](matrix.wgsl) | Columns of glyphs raining down, brightening as you type | Dupin                                                                                       |

The ported shaders keep their MIT notices in their headers. The built-in `flow` shader is [`crates/dupin/src/shaders/flow.wgsl`](../../crates/dupin/src/shaders/flow.wgsl).

A new shader here also goes in `EXAMPLES` in [`crates/dupin/src/shader.rs`](../../crates/dupin/src/shader.rs); a test checks the two agree.
