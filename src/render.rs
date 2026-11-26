//! GPU renderer: a premultiplied-alpha DirectComposition swap chain covering the
//! overlay window. Flakes and clumps are drawn from a procedurally generated
//! texture atlas with one Direct2D sprite batch (each sprite rotated and
//! squashed by its tumble); piles are shaded vector shapes with a soft shadow,
//! a bright crest and twinkling glints.

use std::mem::ManuallyDrop;

use windows::core::{Interface, Result};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
    D2D_RECT_U, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext3, ID2D1Factory1, ID2D1LinearGradientBrush,
    ID2D1PathGeometry1, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1SpriteBatch, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_QUADRATIC_BEZIER_SEGMENT, D2D1_SPRITE_OPTIONS_NONE,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS,
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::art::{self, sprite};
use crate::pile::Pile;
use crate::snow::Snow;

/// Flakes nearer than this are drawn in front of the piles; farther ones behind.
const FRONT_DEPTH: f32 = 0.5;

pub struct Renderer {
    swap: IDXGISwapChain1,
    factory: ID2D1Factory1,
    dc: ID2D1DeviceContext3,
    atlas: ID2D1Bitmap1,
    batch: ID2D1SpriteBatch,
    snow_fill: ID2D1LinearGradientBrush,
    shadow_brush: ID2D1SolidColorBrush,
    /// Physical pixels per logical pixel, for line widths and offsets.
    scale: f32,
    rects: Vec<D2D_RECT_F>,
    srcs: Vec<D2D_RECT_U>,
    colors: Vec<D2D1_COLOR_F>,
    xforms: Vec<Matrix3x2>,
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

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn identity() -> Matrix3x2 {
    Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
}

fn translate(x: f32, y: f32) -> Matrix3x2 {
    Matrix3x2 { M31: x, M32: y, ..identity() }
}

/// Cheap integer hash for stable per-cell randomness (glint placement).
fn hash(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 29)
}

impl Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32, scale: f32) -> Result<Self> {
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

            let pixels = art::atlas_bgra();
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                colorContext: ManuallyDrop::new(None),
            };
            let atlas = dc.CreateBitmap(
                D2D_SIZE_U { width: art::WIDTH, height: art::HEIGHT },
                Some(pixels.as_ptr() as *const _),
                art::WIDTH * 4,
                &props,
            )?;
            let batch = dc.CreateSpriteBatch()?;

            // Snow is bright on top and picks up a cool blue shade toward its base.
            let stops = [
                // Mostly white so a fresh thin layer reads as snow, not a blue line;
                // only the base of a deep drift picks up the cool shade.
                D2D1_GRADIENT_STOP { position: 0.0, color: color(1.0, 1.0, 1.0, 1.0) },
                D2D1_GRADIENT_STOP { position: 0.5, color: color(0.97, 0.98, 1.0, 1.0) },
                D2D1_GRADIENT_STOP { position: 0.78, color: color(0.87, 0.9, 0.96, 1.0) },
                D2D1_GRADIENT_STOP { position: 0.9, color: color(0.8, 0.85, 0.93, 0.85) },
                D2D1_GRADIENT_STOP { position: 1.0, color: color(0.78, 0.83, 0.92, 0.0) },
            ];
            // The simple overload lives on ID2D1RenderTarget; the device context shadows it.
            let rt: ID2D1RenderTarget = dc.cast()?;
            let stops = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let snow_fill = dc.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 },
                    endPoint: D2D_POINT_2F { x: 0.0, y: 10.0 },
                },
                None,
                &stops,
            )?;
            let shadow_brush = dc.CreateSolidColorBrush(&color(0.05, 0.08, 0.14, 0.16), None)?;

            let r = Self {
                swap,
                factory: d2f,
                dc,
                atlas,
                batch,
                snow_fill,
                shadow_brush,
                scale,
                rects: Vec::new(),
                srcs: Vec::new(),
                colors: Vec::new(),
                xforms: Vec::new(),
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
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
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

    /// Queue one sprite: centred at (x, y), body radius `r`, rotated by `angle`,
    /// squashed across by `squash` (tumbling), with opacity `alpha`.
    fn push(&mut self, id: u16, x: f32, y: f32, r: f32, angle: f32, squash: f32, alpha: f32) {
        let half = r / art::body_fraction(id);
        self.rects.push(D2D_RECT_F { left: -half, top: -half, right: half, bottom: half });
        let (x0, y0, x1, y1) = art::cell_rect(id);
        self.srcs.push(D2D_RECT_U { left: x0, top: y0, right: x1, bottom: y1 });
        // Sprite colours are straight alpha: D2D premultiplies them itself.
        // (Passing a premultiplied tint darkened faint flakes into grey rings.)
        self.colors.push(color(1.0, 1.0, 1.0, alpha));
        let (s, c) = angle.sin_cos();
        // Row-vector convention: squash along local x, then rotate, then move.
        self.xforms.push(Matrix3x2 { M11: squash * c, M12: squash * s, M21: -s, M22: c, M31: x, M32: y });
    }

    /// Geometry for one pile's visible snow: every patch (for the snow itself)
    /// and the thicker patches (for their soft contact shadow). The top surface
    /// is a chain of quadratic curves through the cell heights, so mounds are
    /// round rather than faceted.
    fn pile_geometry(&self, p: &Pile) -> Result<Option<[ID2D1PathGeometry1; 2]>> {
        let thick = 2.5 * self.scale;
        unsafe {
            let geoms = [self.factory.CreatePathGeometry()?, self.factory.CreatePathGeometry()?];
            let sinks = [geoms[0].Open()?, geoms[1].Open()?];
            let mut any = false;
            let mut curve: Vec<D2D1_QUADRATIC_BEZIER_SEGMENT> = Vec::new();
            for (i0, i1) in p.runs() {
                let x_start = p.cell_x(i0) - p.cell;
                let x_end = p.cell_x(i1) + p.cell;
                let pt = |i: usize| D2D_POINT_2F { x: p.cell_x(i), y: p.y - p.h[i] };
                let mid = |a: D2D_POINT_2F, b: D2D_POINT_2F| D2D_POINT_2F { x: (a.x + b.x) * 0.5, y: (a.y + b.y) * 0.5 };
                curve.clear();
                // From the bare edge before the patch, through each cell as a control point.
                for i in i0..=i1 {
                    let c = pt(i);
                    let next = if i < i1 { pt(i + 1) } else { D2D_POINT_2F { x: x_end, y: p.y } };
                    let end = if i < i1 { mid(c, next) } else { next };
                    curve.push(D2D1_QUADRATIC_BEZIER_SEGMENT { point1: c, point2: end });
                }
                let is_thick = p.h[i0..=i1].iter().any(|&h| h >= thick);
                for (k, sink) in sinks.iter().enumerate() {
                    if k == 1 && !is_thick {
                        continue;
                    }
                    sink.BeginFigure(D2D_POINT_2F { x: x_start, y: p.y }, D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddQuadraticBeziers(&curve);
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                }
                any = true;
            }
            for sink in &sinks {
                sink.Close()?;
            }
            Ok(any.then_some(geoms))
        }
    }

    /// Soft contact shadow, then the snow body: white on top, shading to a cool
    /// grey-blue and fading out at the very bottom, so it blends into the edge
    /// it sits on (no outline).
    fn draw_piles(&self, piles: &[Pile]) -> Result<()> {
        let s = self.scale;
        unsafe {
            self.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            for p in piles {
                let Some([fill, thick_fill]) = self.pile_geometry(p)? else { continue };
                self.dc.SetTransform(&translate(0.0, 1.2 * s));
                self.dc.FillGeometry(&thick_fill, &self.shadow_brush, None);
                self.dc.SetTransform(&identity());
                // A fixed-height gradient anchored at the base: thin snow is mostly the
                // translucent bottom of it, deep drifts get the bright white top.
                self.snow_fill.SetStartPoint(D2D_POINT_2F { x: 0.0, y: p.y - 12.0 * s });
                self.snow_fill.SetEndPoint(D2D_POINT_2F { x: 0.0, y: p.y + 0.5 * s });
                self.dc.FillGeometry(&fill, &self.snow_fill, None);
            }
        }
        Ok(())
    }

    /// Draw one frame and queue it for display. An error means the GPU device
    /// was lost; the caller should drop this renderer and build a new one.
    pub fn draw(&mut self, snow: &Snow) -> Result<()> {
        let s = self.scale;
        self.rects.clear();
        self.srcs.clear();
        self.colors.clear();
        self.xforms.clear();

        // Back layer: far flakes, behind the piles.
        for f in snow.flakes.iter().filter(|f| f.depth < FRONT_DEPTH) {
            self.push(f.sprite, f.x, f.y, f.r, f.angle, f.squash, f.alpha);
        }
        let back = self.rects.len() as u32;
        // Front layer: near flakes, flying clumps and powder.
        for f in snow.flakes.iter().filter(|f| f.depth >= FRONT_DEPTH) {
            self.push(f.sprite, f.x, f.y, f.r, f.angle, f.squash, f.alpha);
        }
        for p in &snow.puffs {
            self.push(p.sprite, p.x, p.y, p.r, p.angle, 1.0, p.alpha.min(1.0));
        }
        // Glints: a few fixed spots on each pile catch the light now and then.
        for p in &snow.piles {
            for i in 0..p.h.len() {
                let hv = hash(p.id, i as u64);
                if hv % 61 != 0 || p.h[i] < 2.5 * s {
                    continue;
                }
                let x = p.cell_x(i);
                if !p.is_visible(x) {
                    continue;
                }
                let rate = 0.6 + (hv >> 8 & 0xFF) as f32 / 255.0 * 1.4;
                let phase = (hv >> 16 & 0xFFFF) as f32 / 65535.0 * 6.283;
                let tw = (snow.time * rate + phase).sin().max(0.0).powi(24);
                if tw > 0.03 {
                    let y = p.y - p.h[i] + ((hv >> 32 & 0xFF) as f32 / 255.0) * p.h[i] * 0.5;
                    self.push(sprite::GLINT, x, y, 2.6 * s * (0.6 + 0.4 * tw), snow.time * 0.3, 1.0, tw);
                }
            }
        }

        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            let n = self.rects.len() as u32;
            if n > 0 {
                self.batch.Clear();
                let tstride = std::mem::size_of::<Matrix3x2>() as u32;
                self.batch.AddSprites(
                    n,
                    self.rects.as_ptr(),
                    Some(self.srcs.as_ptr()),
                    Some(self.colors.as_ptr()),
                    Some(self.xforms.as_ptr()),
                    std::mem::size_of::<D2D_RECT_F>() as u32,
                    std::mem::size_of::<D2D_RECT_U>() as u32,
                    std::mem::size_of::<D2D1_COLOR_F>() as u32,
                    tstride,
                )?;
            }
            // Sprite batches require aliased mode; the sprites themselves are antialiased.
            if back > 0 {
                self.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                self.dc.DrawSpriteBatch(&self.batch, 0, back, &self.atlas, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_SPRITE_OPTIONS_NONE);
            }
            let piles = self.draw_piles(&snow.piles);
            if n > back {
                self.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                self.dc.DrawSpriteBatch(
                    &self.batch,
                    back,
                    n - back,
                    &self.atlas,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    D2D1_SPRITE_OPTIONS_NONE,
                );
            }
            self.dc.EndDraw(None, None)?;
            piles?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    /// Show nothing (used right before hiding the overlay).
    pub fn clear(&mut self) -> Result<()> {
        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            self.dc.EndDraw(None, None)?;
            self.swap.Present(0, DXGI_PRESENT(0)).ok()
        }
    }
}
