//! Desktop blur lives in its own composition host, immediately below egui.
//! WGPU retains ownership of the UI swap chain and composition target.

use crate::silhouette::ContinuousRect;
use std::{ffi::c_void, ptr::null_mut, sync::mpsc, time::Duration};
use windows::{
    Foundation::{IPropertyValue, PropertyValue},
    Graphics::Effects::{
        IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl,
    },
    System::{DispatcherQueueController, DispatcherQueueHandler},
    UI::Composition::{
        CompositionEffectBrush, CompositionEffectFactory, CompositionEffectFactoryLoadStatus,
        CompositionEffectSourceParameter, Compositor, Desktop::DesktopWindowTarget, SpriteVisual,
    },
    Win32::{
        Graphics::Direct2D::{
            CLSID_D2D1GaussianBlur, Common::D2D1_BORDER_MODE_HARD,
            D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
        },
        System::WinRT::{
            Composition::ICompositorDesktopInterop,
            CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_DEDICATED,
            DispatcherQueueOptions,
            Graphics::Direct2D::{
                GRAPHICS_EFFECT_PROPERTY_MAPPING, GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
                IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
            },
        },
    },
    core::{Error, GUID, HSTRING, Interface, PCWSTR, Result, implement},
};
use windows_numerics::Vector2;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const BLUR_DIP: f32 = 20.;
const HOST_SUBCLASS_ID: usize = 0x5348_424c;

unsafe extern "system" fn backdrop_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

// A small, documented effect-description implementation avoids a Win2D runtime
// dependency. Composition evaluates this D2D effect on the GPU, not in egui.
#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlur {
    source: IGraphicsEffectSource,
    sigma: f32,
}

impl IGraphicsEffectSource_Impl for GaussianBlur_Impl {}
impl IGraphicsEffect_Impl for GaussianBlur_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(HSTRING::from("DesktopBlur"))
    }
    fn SetName(&self, _: &HSTRING) -> Result<()> {
        // This immutable descriptor's name is not used for animation.
        Ok(())
    }
}
impl IGraphicsEffectD2D1Interop_Impl for GaussianBlur_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }
    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        let name = unsafe { name.to_string()? };
        let property = match name.as_str() {
            "StandardDeviation" => 0,
            "Optimization" => 1,
            "BorderMode" => 2,
            _ => {
                return Err(Error::from_hresult(windows::core::HRESULT(
                    0x80070057u32 as i32,
                )));
            }
        };
        if index.is_null() || mapping.is_null() {
            return Err(Error::from_hresult(windows::core::HRESULT(
                0x80004003u32 as i32,
            )));
        }
        unsafe {
            *index = property;
            *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
        }
        Ok(())
    }
    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }
    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        match index {
            0 => PropertyValue::CreateSingle(self.sigma)?.cast(),
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?
                .cast(),
            2 => PropertyValue::CreateUInt32(D2D1_BORDER_MODE_HARD.0 as u32)?.cast(),
            _ => Err(Error::from_hresult(windows::core::HRESULT(
                0x80070057u32 as i32,
            ))),
        }
    }
    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(Error::from_hresult(windows::core::HRESULT(
                0x80070057u32 as i32,
            )))
        }
    }
    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

struct CompositionHost {
    target: DesktopWindowTarget,
    visual: SpriteVisual,
    brush: CompositionEffectBrush,
    factory: CompositionEffectFactory,
    compositor: Compositor,
}

/// A native companion, never an owner popup: Windows places owner popups above
/// their owner, which would blur the foreground UI as well as the desktop.
pub(super) struct BackdropWindow {
    pub(super) hwnd: HWND,
    composition: CompositionHost,
    dispatcher: DispatcherQueueController,
    rendered: Option<[i32; 3]>,
    sigma: f32,
    last_error: Option<String>,
}

impl BackdropWindow {
    pub(super) unsafe fn new() -> std::result::Result<Self, String> {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
                super::native::wide("STATIC").as_ptr(),
                super::native::wide("Shard Typer desktop blur").as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if hwnd.is_null() {
            return Err("Windows could not create the desktop blur host.".into());
        }
        if unsafe { SetWindowSubclass(hwnd, Some(backdrop_proc), HOST_SUBCLASS_ID, 0) } == 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
            return Err("Windows could not install the no-activate blur host.".into());
        }
        let enabled = 1i32;
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH as u32,
                &enabled as *const _ as *const c_void,
                std::mem::size_of_val(&enabled) as u32,
            )
        };
        if result < 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
            return Err(
                "Background blur requires Windows 11 or later. Dark tint is still available."
                    .into(),
            );
        }
        let result = Self::create_composition(hwnd);
        match result {
            Ok((composition, dispatcher)) => Ok(Self {
                hwnd,
                composition,
                dispatcher,
                rendered: None,
                sigma: BLUR_DIP,
                last_error: None,
            }),
            Err(error) => {
                unsafe {
                    DestroyWindow(hwnd);
                }
                Err(format!(
                    "Windows could not initialize desktop blur: {error}"
                ))
            }
        }
    }

    fn create_composition(hwnd: HWND) -> Result<(CompositionHost, DispatcherQueueController)> {
        // A dedicated STA owns Composition's dispatcher. Unlike a queue on the
        // winit thread, it can shut down without nested pumping of app messages.
        let dispatcher = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_DEDICATED,
                apartmentType: DQTAT_COM_STA,
            })?
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let hwnd = hwnd as usize;
        let enqueue_result = dispatcher.DispatcherQueue().and_then(|queue| {
            queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
                let result = (|| -> Result<CompositionHost> {
                    let compositor = Compositor::new()?;
                    let interop: ICompositorDesktopInterop = compositor.cast()?;
                    let target = unsafe {
                        interop.CreateDesktopWindowTarget(
                            windows::Win32::Foundation::HWND(hwnd as *mut c_void),
                            false,
                        )?
                    };
                    let source_name = HSTRING::from("Desktop");
                    let source = CompositionEffectSourceParameter::Create(&source_name)?;
                    let effect: IGraphicsEffect = GaussianBlur {
                        source: source.cast()?,
                        sigma: BLUR_DIP,
                    }
                    .into();
                    let factory = compositor.CreateEffectFactory(&effect)?;
                    let brush = factory.CreateBrush()?;
                    brush
                        .SetSourceParameter(&source_name, &compositor.CreateHostBackdropBrush()?)?;
                    let visual = compositor.CreateSpriteVisual()?;
                    visual.SetBrush(&brush)?;
                    visual.SetRelativeSizeAdjustment(Vector2 { X: 1., Y: 1. })?;
                    target.SetRoot(&visual)?;
                    Ok(CompositionHost {
                        target,
                        visual,
                        brush,
                        factory,
                        compositor,
                    })
                })();
                let _ = sender.send(result);
                Ok(())
            }))
        });
        let queued = match enqueue_result {
            Ok(queued) => queued,
            Err(error) => {
                let _ = dispatcher.ShutdownQueueAsync();
                return Err(error);
            }
        };
        if !queued {
            let _ = dispatcher.ShutdownQueueAsync();
            return Err(Error::from_hresult(windows::core::HRESULT(
                0x80004005u32 as i32,
            )));
        }
        let composition = match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => Err(Error::from_hresult(windows::core::HRESULT(
                0x800705B4u32 as i32,
            ))),
        };
        match composition {
            Ok(composition) => Ok((composition, dispatcher)),
            Err(error) => {
                let _ = dispatcher.ShutdownQueueAsync();
                Err(error)
            }
        }
    }

    pub(super) fn sync(&mut self, parent: HWND, radius: f32) {
        unsafe {
            if IsWindowVisible(parent) == 0 || IsIconic(parent) != 0 {
                ShowWindow(self.hwnd, SW_HIDE);
                return;
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(parent, &mut rect) == 0 {
                return;
            }
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 {
                ShowWindow(self.hwnd, SW_HIDE);
                return;
            }
            let key = [width, height, (radius * 1000.).round() as i32];
            if self.rendered != Some(key) {
                let points: Vec<POINT> = ContinuousRect::new([width as f32, height as f32], radius)
                    .points()
                    .iter()
                    .map(|p| POINT {
                        x: p[0].round() as i32,
                        y: p[1].round() as i32,
                    })
                    .collect();
                let region = CreatePolygonRgn(points.as_ptr(), points.len() as i32, WINDING);
                if region.is_null() || SetWindowRgn(self.hwnd, region, 0) == 0 {
                    if !region.is_null() {
                        DeleteObject(region);
                    }
                    self.last_error = Some("Windows could not update the blur silhouette.".into());
                    ShowWindow(self.hwnd, SW_HIDE);
                    return;
                }
                self.rendered = Some(key);
            }
            // Only DPI changes rebuild the immutable effect. Ordinary resizing
            // adjusts the relative-size visual and region without GPU churn.
            let sigma = BLUR_DIP * GetDpiForWindow(parent).max(96) as f32 / 96.;
            if (self.sigma - sigma).abs() > f32::EPSILON {
                let effect = GaussianBlur {
                    source: match CompositionEffectSourceParameter::Create(&HSTRING::from(
                        "Desktop",
                    ))
                    .and_then(|source| source.cast())
                    {
                        Ok(source) => source,
                        Err(error) => {
                            self.last_error = Some(error.to_string());
                            ShowWindow(self.hwnd, SW_HIDE);
                            return;
                        }
                    },
                    sigma,
                };
                let effect: IGraphicsEffect = effect.into();
                let result = (|| -> Result<(CompositionEffectFactory, CompositionEffectBrush)> {
                    let factory = self.composition.compositor.CreateEffectFactory(&effect)?;
                    let brush = factory.CreateBrush()?;
                    brush.SetSourceParameter(
                        &HSTRING::from("Desktop"),
                        &self.composition.compositor.CreateHostBackdropBrush()?,
                    )?;
                    self.composition.visual.SetBrush(&brush)?;
                    Ok((factory, brush))
                })();
                match result {
                    Ok((factory, brush)) => {
                        self.composition.factory = factory;
                        self.composition.brush = brush;
                    }
                    Err(error) => {
                        self.last_error =
                            Some(format!("Windows could not resize desktop blur: {error}"));
                        ShowWindow(self.hwnd, SW_HIDE);
                        return;
                    }
                }
                self.sigma = sigma;
            }
            if SetWindowPos(
                self.hwnd,
                parent,
                rect.left,
                rect.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
            {
                self.last_error = Some("Windows could not position desktop blur.".into());
                return;
            }
            self.last_error = None;
        }
    }

    pub(super) fn status(&self) -> std::result::Result<(), String> {
        if let Some(error) = &self.last_error {
            return Err(error.clone());
        }
        match self.composition.factory.LoadStatus() {
            // Shader compilation starts asynchronously. Pending is healthy;
            // an actual load failure is reported through the factory status.
            Ok(
                CompositionEffectFactoryLoadStatus::Success
                | CompositionEffectFactoryLoadStatus::Pending,
            ) => Ok(()),
            Ok(_) => Err(format!(
                "Windows could not load the desktop blur effect: {:?}",
                self.composition
                    .factory
                    .ExtendedError()
                    .unwrap_or(windows::core::HRESULT(0x80004005u32 as i32))
            )),
            Err(error) => Err(format!(
                "Windows could not query the desktop blur effect: {error}"
            )),
        }
    }

    pub(super) fn hide(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    #[cfg(test)]
    pub(super) fn validate_composition(&self) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while self.composition.factory.LoadStatus()? == CompositionEffectFactoryLoadStatus::Pending
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            self.composition.factory.LoadStatus()?,
            CompositionEffectFactoryLoadStatus::Success
        );
        assert!(self.composition.factory.ExtendedError()?.is_ok());
        let root: SpriteVisual = self.composition.target.Root()?.cast()?;
        assert_eq!(root, self.composition.visual);
        let source = self
            .composition
            .brush
            .GetSourceParameter(&HSTRING::from("Desktop"))?;
        let _: windows::UI::Composition::CompositionBackdropBrush = source.cast()?;
        Ok(())
    }
}

impl Drop for BackdropWindow {
    fn drop(&mut self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        let _ = self.composition.target.Close();
        let _ = self.composition.visual.Close();
        let _ = self.composition.brush.Close();
        let _ = self.composition.factory.Close();
        let _ = self.composition.compositor.Close();
        let _ = self.dispatcher.ShutdownQueueAsync();
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
