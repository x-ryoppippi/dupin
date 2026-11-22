//! A user's backdrop taken through every step Rune's Metal backend will take with it: naga's MSL
//! writer, Metal's compiler and the render pipeline it is drawn with. Rune ends the process on a
//! failure at any of them, and each refuses things the steps before it accept: names Metal's
//! headers reserve, nesting past its compiler's limits, a fragment function whose locals pass the
//! GPU's stack. Running them here turns each into a message.

use std::sync::OnceLock;

use naga::back::msl;
use naga::proc::{BoundsCheckPolicies, BoundsCheckPolicy};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSProcessInfo, NSString};
use objc2_metal::{
    MTLBlendFactor, MTLCompileOptions, MTLCreateSystemDefaultDevice, MTLDevice, MTLFunction,
    MTLLanguageVersion, MTLLibrary, MTLPixelFormat, MTLPrimitiveTopologyClass, MTLRenderPipelineDescriptor,
};

type Device = Retained<ProtocolObject<dyn MTLDevice>>;

/// The Mac's Metal device, made once: each check runs on a thread of its own.
struct Shared(Device);
// SAFETY: Metal devices are thread-safe; Apple documents creating resources from any thread.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

static DEVICE: OnceLock<Option<Shared>> = OnceLock::new();

fn device() -> Option<&'static Device> {
    DEVICE
        .get_or_init(|| MTLCreateSystemDefaultDevice().map(Shared))
        .as_ref()
        .map(|d| &d.0)
}

/// Whether this Mac has a Metal device to check with.
#[cfg(test)]
pub fn available() -> bool {
    device().is_some()
}

/// Check that Rune's Metal backend can draw `module`, with `dupin_u` in buffer 0. A Mac without a
/// Metal device (a CI runner, say) checks the translation alone.
pub fn check(module: &naga::Module, info: &naga::valid::ModuleInfo) -> Result<(), String> {
    let (lang, version) = msl_version();
    let vertex = translate(module, info, naga::ShaderStage::Vertex, "dupin_vs", lang)?;
    let fragment = translate(module, info, naga::ShaderStage::Fragment, "dupin_fs", lang)?;
    match device() {
        Some(device) => pipeline(device, version, &vertex, &fragment),
        None => Ok(()),
    }
}

/// MSL for one entry point, written as Rune's `translate` writes it, and the function's name.
fn translate(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: naga::ShaderStage,
    entry: &str,
    lang: (u8, u8),
) -> Result<(String, String), String> {
    let mut resources = msl::EntryPointResources::default();
    resources.resources.insert(
        naga::ResourceBinding { group: 0, binding: 0 },
        msl::BindTarget {
            buffer: Some(0),
            ..Default::default()
        },
    );
    let options = msl::Options {
        lang_version: lang,
        per_entry_point_map: [(entry.to_owned(), resources)].into(),
        fake_missing_bindings: false,
        bounds_check_policies: BoundsCheckPolicies {
            index: BoundsCheckPolicy::Restrict,
            buffer: BoundsCheckPolicy::Restrict,
            image_load: BoundsCheckPolicy::Restrict,
            binding_array: BoundsCheckPolicy::Unchecked,
        },
        zero_initialize_workgroup_memory: true,
        force_loop_bounding: true,
        emit_int_div_checks: true,
        ..Default::default()
    };
    let pipeline = msl::PipelineOptions {
        entry_point: Some((stage, entry.to_owned())),
        ..Default::default()
    };
    // Rune evaluates override expressions first; Dupin refuses overrides, but the pass also
    // rewrites what Rune hands the writer.
    let (module, info) = naga::back::pipeline_constants::process_overrides(
        module,
        info,
        Some((stage, entry)),
        &Default::default(),
    )
    .map_err(|e| format!(" can't be compiled for Metal: {e}"))?;
    let (source, translated) = msl::write_string(&module, &info, &options, &pipeline)
        .map_err(|e| format!(" can't be compiled for Metal: {e}"))?;
    let name = match translated.entry_point_names.first() {
        Some(Ok(name)) => name.clone(),
        Some(Err(e)) => return Err(format!(" can't be compiled for Metal: {e}")),
        None => return Err(format!(" can't be compiled for Metal: no {entry}")),
    };
    Ok((source, name))
}

/// Compile both stages as Rune does, one library each, and make the pipeline it draws with.
fn pipeline(
    device: &Device,
    version: MTLLanguageVersion,
    vertex: &(String, String),
    fragment: &(String, String),
) -> Result<(), String> {
    let vertex = function(device, version, vertex)?;
    let fragment = function(device, version, fragment)?;
    let d = MTLRenderPipelineDescriptor::new();
    d.setVertexFunction(Some(&vertex));
    d.setFragmentFunction(Some(&fragment));
    // SAFETY: attachment 0 of the four every descriptor has.
    let attachment = unsafe { d.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    attachment.setBlendingEnabled(true);
    attachment.setSourceRGBBlendFactor(MTLBlendFactor::One);
    attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
    attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    // SAFETY: the backdrop draws a triangle strip, whose class this is.
    unsafe { d.setInputPrimitiveTopology(MTLPrimitiveTopologyClass::Triangle) };
    device
        .newRenderPipelineStateWithDescriptor_error(&d)
        .map(drop)
        .map_err(|e| format!(" can't be drawn by Metal: {}", e.localizedDescription()))
}

fn function(
    device: &Device,
    version: MTLLanguageVersion,
    (source, name): &(String, String),
) -> Result<Retained<ProtocolObject<dyn MTLFunction>>, String> {
    let options = MTLCompileOptions::new();
    options.setLanguageVersion(version);
    options.setPreserveInvariance(true);
    let library = device
        .newLibraryWithSource_options_error(&NSString::from_str(source), Some(&options))
        .map_err(|e| format!(" can't be compiled for Metal: {}", e.localizedDescription()))?;
    library
        .newFunctionWithName(&NSString::from_str(name))
        .ok_or_else(|| format!(" can't be compiled for Metal: no function {name}"))
}

/// The MSL version Rune compiles for on this macOS (`rune-gpu`'s `msl_version`).
fn msl_version() -> ((u8, u8), MTLLanguageVersion) {
    match NSProcessInfo::processInfo().operatingSystemVersion().majorVersion {
        26.. => ((4, 0), MTLLanguageVersion::Version4_0),
        15 => ((3, 2), MTLLanguageVersion::Version3_2),
        14 => ((3, 1), MTLLanguageVersion::Version3_1),
        _ => ((3, 0), MTLLanguageVersion::Version3_0),
    }
}
