//! Draws the backdrop shader behind the panel's content. The view places an effect slot tagged
//! [`MARK`] there; this compositor paints the panel state's module into it, and animates only
//! while the panel shows.

use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use dupin_core::Mode;
use dupin_core::session::Results;
use rune::compose::{Compositor, DeviceKey, Frame};
use rune::render::EffectSite;

use crate::ui::Ui;

pub const MARK: u32 = 1;
const UNIFORM_FLOATS: usize = 28;
/// What `Backdrop::keystroke` reads before the first keystroke: long ago.
const NO_KEYSTROKE: f32 = 1.0e4;

#[derive(Default)]
pub struct Backdrop {
    gpu: Option<Gpu>,
    live: bool,
    drew: bool,
}

/// What the shader sees besides the slot, read from the panel state each frame.
#[derive(Clone, Copy)]
struct Inputs {
    time: f32,
    keystroke: f32,
    query: f32,
    searching: f32,
    mode: f32,
}

struct Gpu {
    key: DeviceKey,
    layout: wgpu::PipelineLayout,
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    program: Option<Program>,
}

/// One module and its pipelines, one per target format: isolated layers draw into other formats
/// than the window's.
struct Program {
    source: Arc<str>,
    module: Option<wgpu::ShaderModule>,
    pipes: Vec<(wgpu::TextureFormat, Option<wgpu::RenderPipeline>)>,
}

impl Gpu {
    fn new(device: &wgpu::Device, key: DeviceKey) -> Gpu {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("backdrop"),
            size: (UNIFORM_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        Gpu {
            key,
            layout,
            uniforms,
            group,
            program: None,
        }
    }

    fn use_source(&mut self, device: &wgpu::Device, source: &Arc<str>) {
        if self
            .program
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.source, source))
        {
            return;
        }
        let module = checked(device, "backdrop shader", || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("backdrop"),
                source: wgpu::ShaderSource::Wgsl(source.as_ref().into()),
            })
        });
        self.program = Some(Program {
            source: source.clone(),
            module,
            pipes: Vec::new(),
        });
    }

    fn pipe(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> Option<&wgpu::RenderPipeline> {
        let program = self.program.as_mut()?;
        let at = match program.pipes.iter().position(|(f, _)| *f == format) {
            Some(at) => at,
            None => {
                let module = program.module.as_ref()?;
                let pipe = checked(device, "backdrop pipeline", || {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("backdrop"),
                        layout: Some(&self.layout),
                        vertex: wgpu::VertexState {
                            module,
                            entry_point: Some("dupin_vs"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleStrip,
                            ..Default::default()
                        },
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module,
                            entry_point: Some("dupin_fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                });
                program.pipes.push((format, pipe));
                program.pipes.len() - 1
            }
        };
        program.pipes[at].1.as_ref()
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        site: &EffectSite<'_>,
        inputs: Inputs,
    ) -> bool {
        if self.pipe(device, site.format).is_none() {
            return false;
        }
        let i = site.instance;
        let mut u = [0f32; UNIFORM_FLOATS];
        u[0..4].copy_from_slice(&i.rect);
        u[4..8].copy_from_slice(&i.clip);
        for (k, bits) in i.fade.iter().enumerate() {
            u[8 + k] = f32::from_bits(*bits);
        }
        u[12..14].copy_from_slice(&site.origin);
        u[14] = site.size[0] as f32;
        u[15] = site.size[1] as f32;
        u[16..20].copy_from_slice(&site.data);
        u[20] = inputs.time;
        u[21] = site.scale;
        u[22] = if site.srgb_blend { 1.0 } else { 0.0 };
        u[23] = inputs.keystroke;
        u[24] = inputs.query;
        u[25] = inputs.searching;
        u[26] = inputs.mode;
        u[27] = crate::view::HEADER_H;
        let bytes: Vec<u8> = u.iter().flat_map(|v| v.to_ne_bytes()).collect();
        queue.write_buffer(&self.uniforms, 0, &bytes);
        let pipe = self.pipe(device, site.format).expect("made above");
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..4, 0..1);
        true
    }
}

/// Runs `make` in a validation error scope, so a module the shader compiler rejects past
/// `shader::compile`'s checks is logged and skipped instead of panicking in wgpu's default
/// error handler.
fn checked<T>(device: &wgpu::Device, what: &str, make: impl FnOnce() -> T) -> Option<T> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = make();
    let mut popped = std::pin::pin!(scope.pop());
    match popped.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Some(e)) => {
            crate::log::warn(&format!("{what}: {e}"));
            None
        }
        _ => Some(made),
    }
}

impl Compositor<Ui> for Backdrop {
    fn render(&mut self, f: &mut Frame<'_, Ui>) {
        let key = f.device_key();
        if self.gpu.as_ref().is_none_or(|g| g.key != key) {
            self.gpu = Some(Gpu::new(f.device, key));
        }
        let gpu = self.gpu.as_mut().expect("made above");
        let still = f.rt.reduced_motion || f.rt.system_reduced_motion();
        let s = &f.rt.state;
        if let Some(source) = s.backdrop.module() {
            gpu.use_source(f.device, source);
        }
        self.live = s.shown && s.backdrop.module().is_some() && !still;
        let clock = f.rt.clock;
        let inputs = Inputs {
            // Seconds since the panel showed, so a shader can play an entrance and the value
            // stays small enough for f32. Under Reduce Motion, a frame well past any entrance.
            time: if still { 10.0 } else { (clock - s.shown_at) as f32 },
            keystroke: ((clock - s.typed_at) as f32).min(NO_KEYSTROKE),
            query: s.query.text.chars().count() as f32,
            searching: matches!(s.session.results(), Results::Searching { .. }) as u8 as f32,
            mode: match s.session.mode() {
                Mode::Apps => 0.0,
                Mode::Files => 1.0,
                Mode::Contents => 2.0,
                Mode::Dictionary => 3.0,
            },
        };
        let (device, queue, size) = (f.device, f.queue, f.size);
        let clear = f.rt.clear_color();
        f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, size);
        let mut drew = false;
        f.ui.draw_with(f.encoder, f.target, clear, &mut |pass, site| {
            if site.mark == MARK {
                drew |= gpu.draw(device, queue, pass, site, inputs);
            }
        });
        self.drew = drew;
    }

    fn animating(&self) -> bool {
        self.live
    }

    fn draws_own(&self) -> bool {
        self.drew
    }
}
