//! Platform swapchain color-space hooks for HDR display outputs. wgpu does not
//! expose swapchain color spaces, so Windows reaches the DXGI swap chain through
//! the wgpu-hal escape hatch and macOS configures the window's CAMetalLayer.

use crate::native_hdr::OutputFormat;

pub struct ColorSpaceReport {
    pub color_space: String,
    pub monitor_hdr: Option<bool>,
    pub monitor_max_nits: Option<f32>,
}

#[cfg(windows)]
pub fn apply_color_space(
    surface: &wgpu::Surface<'_>,
    _window: &crate::Window,
    format: OutputFormat,
) -> Result<ColorSpaceReport, String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_COLOR_SPACE_TYPE,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIOutput6, IDXGISwapChain3, DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT,
    };

    if format != OutputFormat::Pq {
        return Err("Windows display outputs support SDR and PQ only.".to_string());
    }
    let hal_surface = unsafe { surface.as_hal::<wgpu::hal::api::Dx12>() }
        .ok_or_else(|| "HDR output requires a DirectX 12 swap chain.".to_string())?;
    let hal_chain = hal_surface
        .swap_chain()
        .ok_or_else(|| "The output swap chain is not configured.".to_string())?;
    // wgpu-hal links a different `windows` crate version, so the same COM
    // pointer is re-borrowed through this crate's interface definition.
    const _: () = assert!(
        std::mem::size_of::<IDXGISwapChain3>() == std::mem::size_of::<*mut std::ffi::c_void>()
    );
    let raw: *mut std::ffi::c_void = unsafe { std::mem::transmute_copy(&hal_chain) };
    let chain: IDXGISwapChain3 = unsafe { IDXGISwapChain3::from_raw_borrowed(&raw) }
        .ok_or_else(|| "The output swap chain pointer is null.".to_string())?
        .clone();
    let space: DXGI_COLOR_SPACE_TYPE = DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
    let support = unsafe { chain.CheckColorSpaceSupport(space) }
        .map_err(|error| format!("DXGI color space query failed: {error}"))?;
    if support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT.0 as u32 == 0 {
        return Err(
            "The graphics driver cannot present the Rec. 2100 PQ color space on this swap chain."
                .to_string(),
        );
    }
    unsafe { chain.SetColorSpace1(space) }
        .map_err(|error| format!("Setting the DXGI HDR color space failed: {error}"))?;
    let (monitor_hdr, monitor_max_nits) = unsafe { chain.GetContainingOutput() }
        .ok()
        .and_then(|output| output.cast::<IDXGIOutput6>().ok())
        .and_then(|output| unsafe { output.GetDesc1() }.ok())
        .map(|desc| {
            (
                Some(desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020),
                Some(desc.MaxLuminance),
            )
        })
        .unwrap_or((None, None));
    Ok(ColorSpaceReport {
        color_space: "DXGI RGB full G2084 P2020 (Rec. 2100 PQ)".to_string(),
        monitor_hdr,
        monitor_max_nits,
    })
}

#[cfg(target_os = "macos")]
pub fn apply_color_space(
    _surface: &wgpu::Surface<'_>,
    window: &crate::Window,
    format: OutputFormat,
) -> Result<ColorSpaceReport, String> {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{msg_send, Encoding, RefEncode};
    use std::ffi::c_void;

    #[repr(C)]
    struct CGColorSpace {
        _opaque: [u8; 0],
    }
    // The selector's argument encoding is checked in debug builds, so the
    // opaque CoreGraphics type must carry its Objective-C encoding.
    unsafe impl RefEncode for CGColorSpace {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColorSpace", &[]));
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        static kCGColorSpaceITUR_2100_PQ: *const c_void;
        static kCGColorSpaceITUR_2100_HLG: *const c_void;
        fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut CGColorSpace;
        fn CGColorSpaceRelease(space: *mut CGColorSpace);
    }

    // AppKit state is read on the main thread: surface creation already runs
    // there, and later reconfigures only re-apply this layer-level setting.
    let view = window.native_window_pointer()? as *mut AnyObject;
    if view.is_null() {
        return Err("The output window has no NSView.".to_string());
    }
    let (name, label) = match format {
        OutputFormat::Pq => (unsafe { kCGColorSpaceITUR_2100_PQ }, "Rec. 2100 PQ"),
        OutputFormat::Hlg => (unsafe { kCGColorSpaceITUR_2100_HLG }, "Rec. 2100 HLG"),
        OutputFormat::Sdr => return Err("SDR does not configure an HDR color space.".to_string()),
    };
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        if layer.is_null() {
            return Err("The output view has no CAMetalLayer.".to_string());
        }
        let space = CGColorSpaceCreateWithName(name);
        if space.is_null() {
            return Err("CoreGraphics could not create the HDR color space.".to_string());
        }
        let _: () = msg_send![layer, setWantsExtendedDynamicRangeContent: Bool::YES];
        let _: () = msg_send![layer, setColorspace: space];
        CGColorSpaceRelease(space);
    }
    Ok(ColorSpaceReport {
        color_space: format!("CAMetalLayer ITUR_2100 ({label}), EDR enabled"),
        monitor_hdr: None,
        monitor_max_nits: None,
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn apply_color_space(
    _surface: &wgpu::Surface<'_>,
    _window: &crate::Window,
    _format: OutputFormat,
) -> Result<ColorSpaceReport, String> {
    Err("HDR display output is not available on this platform.".to_string())
}
