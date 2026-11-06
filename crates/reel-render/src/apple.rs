//! Using CoreVideo buffers as textures with no copying: decoded frames as inputs, and the encoder's
//! buffers as render targets (T2 in `docs/spikes.md`).

use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBuffer, kCVMetalTextureUsage, kCVReturnSuccess,
};
use objc2_metal::{MTLPixelFormat, MTLTextureType, MTLTextureUsage};

type Metal = wgpu::hal::api::Metal;

/// A texture wrapping part of a pixel buffer. Hold on to this until the GPU has finished with it,
/// because it keeps the IOSurface from being reused.
pub struct WrappedTexture {
    pub texture: wgpu::Texture,
    _keep: CFRetained<CVMetalTexture>,
}

/// Wraps CoreVideo buffers as wgpu textures.
pub struct TextureCache {
    device: wgpu::Device,
    cache: CFRetained<CVMetalTextureCache>,
    target_attributes: CFRetained<CFDictionary>,
}

// SAFETY: a CVMetalTextureCache can be used from any one thread at a time, which this is held to.
unsafe impl Send for TextureCache {}

impl TextureCache {
    /// Makes a cache for this device, which must be on the Metal backend.
    ///
    /// # Panics
    /// Panics if the device isn't on Metal, which can't happen on macOS.
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        // SAFETY: Metal is the only backend on this OS, and the out pointer is valid for the call.
        let cache = unsafe {
            let hal = device.as_hal::<Metal>().expect("macOS always uses Metal");
            let mut raw: *mut CVMetalTextureCache = std::ptr::null_mut();
            let status = CVMetalTextureCache::create(None, None, hal.raw_device(), None, NonNull::from(&mut raw));
            assert_eq!(status, kCVReturnSuccess, "couldn't create a texture cache");
            CFRetained::from_raw(NonNull::new(raw).expect("the cache was just created"))
        };
        // Wrapped encoder buffers need to be drawn into, which isn't the default.
        let usage = (MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget).bits() as i64;
        // SAFETY: the documented key and value type for a texture's usage.
        let target_attributes = unsafe {
            let key: &CFString = kCVMetalTextureUsage;
            let value = CFNumber::new_i64(usage);
            let dictionary = CFDictionary::from_slices(&[key], &[&*value]);
            CFRetained::cast_unchecked::<CFDictionary>(dictionary)
        };
        Self { device: device.clone(), cache, target_attributes }
    }

    fn wrap(
        &self,
        buffer: &CVPixelBuffer,
        plane: usize,
        metal: MTLPixelFormat,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        usage: wgpu::TextureUsages,
        uses: wgpu::TextureUses,
    ) -> WrappedTexture {
        let attributes = usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT).then_some(&*self.target_attributes);
        // SAFETY: the cache and buffer are live, and the plane, format and size match the buffer's
        // layout, which the callers below guarantee. The CVMetalTexture is kept with the wgpu texture.
        unsafe {
            let mut raw: *mut CVMetalTexture = std::ptr::null_mut();
            let status = CVMetalTextureCache::create_texture_from_image(
                None,
                &self.cache,
                buffer,
                attributes,
                metal,
                size.0 as usize,
                size.1 as usize,
                plane,
                NonNull::from(&mut raw),
            );
            assert_eq!(status, kCVReturnSuccess, "couldn't wrap a pixel buffer");
            let keep = CFRetained::from_raw(NonNull::new(raw).expect("the texture was just created"));
            let mtl = CVMetalTextureGetTexture(&keep).expect("a wrapped buffer always has a texture");
            let hal = wgpu::hal::metal::Device::texture_from_raw(
                mtl,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent { width: size.0, height: size.1, depth: 1 },
                None,
            );
            let texture = self.device.create_texture_from_hal::<Metal>(
                hal,
                &wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                },
                uses,
            );
            WrappedTexture { texture, _keep: keep }
        }
    }

    /// Wraps a decoded 4:2:0 frame's two planes, ready for `VideoSource::Nv12`.
    #[must_use]
    pub fn wrap_nv12(&self, buffer: &CVPixelBuffer, size: (u32, u32)) -> (WrappedTexture, WrappedTexture) {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING;
        let uses = wgpu::TextureUses::RESOURCE;
        let luma = self.wrap(buffer, 0, MTLPixelFormat::R8Unorm, wgpu::TextureFormat::R8Unorm, size, usage, uses);
        let half = (size.0.div_ceil(2), size.1.div_ceil(2));
        let chroma = self.wrap(buffer, 1, MTLPixelFormat::RG8Unorm, wgpu::TextureFormat::Rg8Unorm, half, usage, uses);
        (luma, chroma)
    }

    /// Wraps an encoder's BGRA buffer as something to render into.
    #[must_use]
    pub fn wrap_target(&self, buffer: &CVPixelBuffer, size: (u32, u32)) -> WrappedTexture {
        self.wrap(
            buffer,
            0,
            MTLPixelFormat::BGRA8Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
            size,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            wgpu::TextureUses::COLOR_TARGET,
        )
    }
}
