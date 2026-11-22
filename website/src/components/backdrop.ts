// Runs Dupin's backdrop shaders, the very WGSL files the app ships, behind the demo panel with
// WebGPU. The wrapper mirrors the app's `prelude.wgsl` and `entry.wgsl`: the same `Backdrop`
// inputs and the same `backdrop(p, b)` call, with a full-canvas triangle for the panel's rect.
import flow from '../../../crates/dupin/src/shaders/flow.wgsl?raw';

const examples = import.meta.glob<string>('../../../examples/shaders/{ripple,matrix,embers}.wgsl', {
	query: '?raw',
	import: 'default',
	eager: true,
});

const exampleSource = (name: string) => examples[`../../../examples/shaders/${name}.wgsl`];

/** The backdrops the demo offers: the built-in one and some of the examples. */
export const backdrops = [
	{ id: 'flow', title: 'Flow', source: flow },
	...['ripple', 'matrix', 'embers'].flatMap((id) => {
		const source = exampleSource(id);
		return source ? [{ id, title: id[0]!.toUpperCase() + id.slice(1), source }] : [];
	}),
];

export type Inputs = {
	time: number;
	keystroke: number;
	query: number;
	searching: number;
	mode: number;
	field: number;
};

const head = /* wgsl */ `
struct Backdrop {
    size: vec2f,
    time: f32,
    dark: f32,
    accent: vec3f,
    keystroke: f32,
    query: f32,
    searching: f32,
    mode: f32,
    field: f32,
}

struct DupinWeb { a: vec4f, b: vec4f, c: vec4f, d: vec4f }
@group(0) @binding(0) var<uniform> dupin_u: DupinWeb;
`;

const tail = /* wgsl */ `
struct DupinVertex { @builtin(position) pos: vec4f }

@vertex fn dupin_vs(@builtin(vertex_index) vi: u32) -> DupinVertex {
    let c = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
    return DupinVertex(vec4f(c * 2.0 - 1.0, 0.0, 1.0));
}

@fragment fn dupin_fs(v: DupinVertex) -> @location(0) vec4f {
    let b = Backdrop(
        dupin_u.a.xy,
        dupin_u.a.z,
        dupin_u.a.w,
        dupin_u.b.rgb,
        dupin_u.b.w,
        dupin_u.c.x,
        dupin_u.c.y,
        dupin_u.c.z,
        dupin_u.c.w,
    );
    let c = backdrop(v.pos.xy / dupin_u.d.x, b);
    let alpha = clamp(c.a, 0.0, 1.0);
    return vec4f(clamp(c.rgb, vec3f(0.0), vec3f(1.0)) * alpha, alpha);
}
`;

export type Renderer = {
	/** Compile `source`; false if this browser's WebGPU rejects it. */
	use(source: string): Promise<boolean>;
	draw(inputs: Inputs): void;
	clear(): void;
	destroy(): void;
};

/** A renderer drawing into `canvas`, or undefined where WebGPU isn't available. */
export async function createRenderer(
	canvas: HTMLCanvasElement,
	accent: [number, number, number],
): Promise<Renderer | undefined> {
	const adapter = await navigator.gpu?.requestAdapter();
	const device = await adapter?.requestDevice();
	const context = canvas.getContext('webgpu');
	if (!device || !context) return undefined;
	const format = navigator.gpu.getPreferredCanvasFormat();
	context.configure({ device, format, alphaMode: 'premultiplied' });

	const uniforms = device.createBuffer({
		size: 64,
		usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
	});
	const layout = device.createBindGroupLayout({
		entries: [{ binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: {} }],
	});
	const group = device.createBindGroup({
		layout,
		entries: [{ binding: 0, resource: { buffer: uniforms } }],
	});
	let pipeline: GPURenderPipeline | undefined;

	const pass = (load: (pass: GPURenderPassEncoder) => void) => {
		const encoder = device.createCommandEncoder();
		const p = encoder.beginRenderPass({
			colorAttachments: [
				{
					view: context.getCurrentTexture().createView(),
					loadOp: 'clear',
					clearValue: [0, 0, 0, 0],
					storeOp: 'store',
				},
			],
		});
		load(p);
		p.end();
		device.queue.submit([encoder.finish()]);
	};

	return {
		async use(source) {
			const module = device.createShaderModule({ code: `${head}\n${source}\n${tail}` });
			const info = await module.getCompilationInfo();
			if (info.messages.some((m) => m.type === 'error')) return false;
			try {
				pipeline = await device.createRenderPipelineAsync({
					layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
					vertex: { module, entryPoint: 'dupin_vs' },
					fragment: {
						module,
						entryPoint: 'dupin_fs',
						targets: [
							{
								format,
								blend: {
									color: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
									alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
								},
							},
						],
					},
				});
				return true;
			} catch {
				pipeline = undefined;
				return false;
			}
		},
		draw(i) {
			const scale = window.devicePixelRatio || 1;
			const width = Math.max(1, Math.round(canvas.clientWidth * scale));
			const height = Math.max(1, Math.round(canvas.clientHeight * scale));
			if (canvas.width !== width || canvas.height !== height) {
				canvas.width = width;
				canvas.height = height;
			}
			if (!pipeline) return;
			const values = new Float32Array([
				width / scale,
				height / scale,
				i.time,
				1,
				...accent,
				i.keystroke,
				i.query,
				i.searching,
				i.mode,
				i.field,
				scale,
				0,
				0,
				0,
			]);
			device.queue.writeBuffer(uniforms, 0, values);
			const current = pipeline;
			pass((p) => {
				p.setPipeline(current);
				p.setBindGroup(0, group);
				p.draw(3);
			});
		},
		clear() {
			pass(() => {});
		},
		destroy() {
			uniforms.destroy();
			device.destroy();
		},
	};
}

/** `color`, any CSS color, as sRGB in 0–1, the way the canvas paints it. */
export function srgb(color: string): [number, number, number] {
	const canvas = document.createElement('canvas');
	canvas.width = canvas.height = 1;
	const ctx = canvas.getContext('2d');
	if (!ctx) return [0.35, 0.45, 1];
	ctx.fillStyle = color;
	ctx.fillRect(0, 0, 1, 1);
	const [r = 0, g = 0, b = 0] = ctx.getImageData(0, 0, 1, 1).data;
	return [r / 255, g / 255, b / 255];
}
