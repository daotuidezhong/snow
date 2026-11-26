//! GPU renderer: a premultiplied-alpha DirectComposition swap chain covering the
//! overlay window, drawn with one Direct2D sprite batch per frame.

use std::mem::ManuallyDrop;

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext3, ID2D1Factory1, ID2D1SpriteBatch,
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_SPRITE_OPTIONS_NONE,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::snow::Flake;

/// Side of the square flake sprite texture, in pixels.
const SPRITE: u32 = 64;
/// The sprite's white core fills this fraction of its radius; the rest is a soft shadow.
const CORE: f32 = 0.6;

pub struct Renderer {
    swap: IDXGISwapChain1,
    dc: ID2D1DeviceContext3,
    sprite: ID2D1Bitmap1,
    batch: ID2D1SpriteBatch,
    rects: Vec<D2D_RECT_F>,
    colors: Vec<D2D1_COLOR_F>,
    // Kept alive for as long as the window shows the swap chain.
    _d3d: ID3D11Device,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

fn create_device(kind: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            kind,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    Ok(device.expect("D3D11CreateDevice returned no device"))
}

/// Premultiplied BGRA pixels of a round snowflake: a white core with a soft
/// edge, wrapped in a faint slate-coloured shadow so it still reads on white windows.
fn sprite_pixels() -> Vec<u8> {
    let n = SPRITE as usize;
    let mut px = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    let edge = 3.0 / n as f32;
    for y in 0..n {
        for x in 0..n {
            let d = (((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt()) / (n as f32 / 2.0);
            let core = ((CORE + edge - d) / (2.0 * edge)).clamp(0.0, 1.0);
            let t = ((d - CORE) / (1.0 - CORE)).clamp(0.0, 1.0);
            let shadow = if d < 1.0 { 0.32 * (1.0 - t).powf(1.8) } else { 0.0 };
            // Composite the core over the shadow (both premultiplied).
            let (cr, cg, cb) = (0.97, 0.98, 1.0);
            let (sr, sg, sb) = (0.30, 0.36, 0.46);
            let a = core + shadow * (1.0 - core);
            let r = cr * core + sr * shadow * (1.0 - core);
            let g = cg * core + sg * shadow * (1.0 - core);
            let b = cb * core + sb * shadow * (1.0 - core);
            let i = (y * n + x) * 4;
            px[i] = (b * 255.0).round() as u8;
            px[i + 1] = (g * 255.0).round() as u8;
            px[i + 2] = (r * 255.0).round() as u8;
            px[i + 3] = (a * 255.0).round() as u8;
        }
    }
    px
}

impl Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let d3d = create_device(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create_device(D3D_DRIVER_TYPE_WARP))?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1),
                Height: height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForComposition(&dxgi, &desc, None)?;

            let opts = D2D1_FACTORY_OPTIONS::default();
            let d2f: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&opts))?;
            let d2dev = d2f.CreateDevice(&dxgi)?;
            let dc: ID2D1DeviceContext3 = d2dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?.cast()?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let pixels = sprite_pixels();
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                colorContext: ManuallyDrop::new(None),
            };
            let sprite = dc.CreateBitmap(
                D2D_SIZE_U { width: SPRITE, height: SPRITE },
                Some(pixels.as_ptr() as *const _),
                SPRITE * 4,
                &props,
            )?;
            let batch = dc.CreateSpriteBatch()?;

            let r = Self {
                swap,
                dc,
                sprite,
                batch,
                rects: Vec::new(),
                colors: Vec::new(),
                _d3d: d3d,
                _dcomp: dcomp,
                _target: target,
                _visual: visual,
            };
            r.bind_target()?;
            Ok(r)
        }
    }

    fn bind_target(&self) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bmp = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bmp);
        }
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        unsafe {
            self.dc.SetTarget(None);
            self.swap.ResizeBuffers(2, width.max(1), height.max(1), DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.bind_target()
    }

    /// Draw one frame and queue it for display. An error means the GPU device
    /// was lost; the caller should drop this renderer and build a new one.
    pub fn draw(&mut self, flakes: &[Flake]) -> Result<()> {
        self.rects.clear();
        self.colors.clear();
        for f in flakes {
            // The sprite's core is CORE of its radius, so scale the quad up to match.
            let r = f.r / CORE;
            self.rects.push(D2D_RECT_F { left: f.x - r, top: f.y - r, right: f.x + r, bottom: f.y + r });
            let a = f.alpha;
            // Premultiplied tint: scales the whole sprite by `a`.
            self.colors.push(D2D1_COLOR_F { r: a, g: a, b: a, a });
        }
        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            if !self.rects.is_empty() {
                // Sprite batches require aliased mode; the sprite itself is antialiased.
                self.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                self.batch.Clear();
                self.batch.AddSprites(
                    self.rects.len() as u32,
                    self.rects.as_ptr(),
                    None,
                    Some(self.colors.as_ptr()),
                    None,
                    std::mem::size_of::<D2D_RECT_F>() as u32,
                    0,
                    std::mem::size_of::<D2D1_COLOR_F>() as u32,
                    0,
                )?;
                self.dc.DrawSpriteBatch(
                    &self.batch,
                    0,
                    self.rects.len() as u32,
                    &self.sprite,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    D2D1_SPRITE_OPTIONS_NONE,
                );
            }
            self.dc.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    /// Show nothing (used right before hiding the overlay).
    pub fn clear(&mut self) -> Result<()> {
        self.draw(&[])
    }
}
