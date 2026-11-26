# Shaders

A backdrop shader is a WGSL file that animates behind the panel, the query field and its results while it shows, as Ghostty's `custom-shader` does behind a terminal.

## Open it

In **Settings → Appearance** (<kbd>⌘,</kbd> in the panel), pick one under **Backdrop**, or set `backdrop` in `config.toml`. The **Shaders folder** row's **Open** button shows the `shaders` folder beside `config.toml`.

## What you see

- **Backdrop** offers **Off**, **Flow**, the four example shaders and every `.wgsl` file in the `shaders` folder.
- A shader that can't be read or doesn't compile shows its error in the panel at every summon until it's fixed.

| Shader                                       | What it draws                                                   |
| -------------------------------------------- | --------------------------------------------------------------- |
| `flow` (built in)                            | The accent colour flowing as warped noise, swelling as you type |
| [Aurora](../../examples/shaders/aurora.wgsl) | Green and purple curtains of light with twinkling stars         |
| [Embers](../../examples/shaders/embers.wgsl) | Sparks rising from a flickering fire                            |
| [Snow](../../examples/shaders/snow.wgsl)     | Flakes drifting down in three parallax layers                   |
| [Matrix](../../examples/shaders/matrix.wgsl) | Columns of glyphs raining down, brightening as you type         |

## What you can do

| Key / action                               | What it does                                                                                           |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| Pick an example in **Backdrop**            | Copies it into the `shaders` folder, unless a copy is already there, and points `backdrop` at the copy |
| **Open** on the **Shaders folder** row     | Opens the `shaders` folder beside `config.toml`                                                        |
| Save a `.wgsl` file, then press the hotkey | Shows the new version                                                                                  |
| Pick **Off**                               | Draws nothing and leaves the panel plain glass                                                         |

## Notes

- Dupin reads the file each time the panel shows and compiles it when its text has changed. Until a broken file is fixed, Dupin keeps the last version that compiled, or draws `flow`.
- The backdrop holds still under System Settings → Accessibility → Display → Reduce motion.
- A file can be at most 32 KB.

## Configuration

```toml
backdrop = "shaders/aurora.wgsl" # "off", "flow", or the path to a WGSL shader
```

| Key        | Default | Values                                                                                                                      |
| ---------- | ------- | --------------------------------------------------------------------------------------------------------------------------- |
| `backdrop` | `"off"` | `"off"`, `"flow"`, or the path to a WGSL file; `~/` is the home directory, and a relative path is relative to `config.toml` |

## Writing your own shader

Put a `.wgsl` file in the `shaders` folder beside `config.toml`, so Settings lists it, and define one function:

```wgsl
fn backdrop(p: vec2f, b: Backdrop) -> vec4f
```

`p` is the pixel's position in points from the panel's top-left corner. Return a straight (not premultiplied) sRGB colour and its alpha. Dupin premultiplies it, clips it to the panel's corners and blends it over the glass, under the text.

`b` holds:

| Field       | Type    | Meaning                                                                                                  |
| ----------- | ------- | -------------------------------------------------------------------------------------------------------- |
| `size`      | `vec2f` | The panel's size in points; its height changes with the number of results                                |
| `time`      | `f32`   | Seconds since the panel showed, for an entrance; held still under Reduce Motion                          |
| `dark`      | `f32`   | 1 in a dark theme, 0 in a light one                                                                      |
| `accent`    | `vec3f` | The theme's accent colour, sRGB                                                                          |
| `keystroke` | `f32`   | Seconds since the query last changed, up to 10000; 10000 before the first change and under Reduce Motion |
| `query`     | `f32`   | Characters in the query                                                                                  |
| `searching` | `f32`   | 1 while a search runs, 0 under Reduce Motion                                                             |
| `mode`      | `f32`   | 0 apps, 1 file names, 2 file contents, 3 Dictionary, 4 clipboard history, 5 calendar events              |
| `field`     | `f32`   | The query field's height in points, at the panel's top; a steady unit to scale by                        |

> [!TIP]
> The shader runs for every pixel of the panel on every frame while the panel shows, so keep loops short. The examples and the built-in [`flow.wgsl`](../../crates/dupin/src/shaders/flow.wgsl) are good starting points.

<details>
<summary>Limits</summary>

- Dupin defines these names too, so a shader can't reuse them: `Backdrop`, anything starting with `dupin_`, and Rune's clip helpers `clip_alpha`, `ramp`, `sd_rounded`, `sd_continuous`, `unpack_clip_radii` and `CONTINUOUS_K`.
- A shader reads only `b`: it can't declare an `override` or bind resources of its own.
- Dupin checks a file as Metal will draw it, and refuses one Metal can't compile or draw, such as a function whose local arrays pass the GPU's stack.
- A file can be at most 32 KB.

</details>

<details>
<summary>Credits</summary>

Aurora and Embers are ported from [snedea/ghostty-themes](https://github.com/snedea/ghostty-themes) (MIT), Snow from [an0sunshy/ghostty-shaders](https://github.com/an0sunshy/ghostty-shaders) (MIT), and Matrix is Dupin's own. The ported shaders keep their MIT notices in their headers.

</details>

See [Appearance](./appearance.md) for themes, and [`examples/shaders`](../../examples/shaders) for the example files.
