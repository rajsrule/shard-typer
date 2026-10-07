# Window material and silhouette

Shard Typer paints a translucent charcoal surface with fully opaque foreground text and icons. On supported Windows 11 builds, a separate native composition host blurs the desktop behind that surface. Background blur is enabled by default and stays active when focus moves to the destination editor.

## Why a separate blur host is needed

Earlier builds used `DWMSBT_TRANSIENTWINDOW`. Windows documents this as its brightest Desktop Acrylic variant. That API selects a predefined material with no blur-strength or tint-opacity parameters. Windows also replaces background Acrylic with solid color when the application deactivates, which is unsuitable when this utility gives focus to another application.

The egui renderer's WGPU DX12 backend privately owns the UI's DirectComposition target and swap chain. It exposes no hook for placing a custom backdrop brush beneath that UI visual. A second, application-owned native host provides the necessary composition tree while retaining the existing renderer and controls.

This host is an unowned, taskbar-hidden, nonactivating companion HWND directly beneath the UI and above its shadow. It is not an owner popup, since Windows normally keeps owned popups above their owners. It shares the app's exact silhouette, position, size, and visibility. Moving, resizing, pinning, collapsing, minimizing, or closing synchronizes the layers. The host never captures desktop screenshots or activates itself to keep a material alive.

Sources: [DWM backdrop types](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type), [Acrylic adaptability and fallback](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic).

## Native effect and support

`DWMWA_USE_HOSTBACKDROPBRUSH` enables the documented host-backdrop capability on the companion window. A WinRT `Compositor` attaches a `DesktopWindowTarget` to it. The root sprite uses an explicit Direct2D Gaussian blur effect sourced from `CreateHostBackdropBrush`, with a standard deviation of 20 logical pixels scaled for the monitor DPI. Tint remains in the egui surface, so blur affects background content while foreground text remains sharp.

A small `IGraphicsEffectD2D1Interop` effect descriptor supplies the supported Gaussian effect without shipping Win2D or the Windows App SDK runtime. Composition resources use a dedicated STA dispatcher queue, which is shut down when the host is released. Desktop blur continues while the main window is inactive; it does not use Acrylic's activation fallback.

The documented Win32 host-backdrop enablement requires Windows 11 build 22000 or later. Windows 10 retains the translucent tint and shows the support limitation in Settings. Initialization, clipping, or positioning failures are also reported in Settings. No opaque imitation is described as working blur, and no undocumented accent-policy calls are used. Windows composition and graphics policy can still affect the actual desktop result.

Sources: [DWM host-backdrop attribute](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute), [Host backdrop brush](https://learn.microsoft.com/en-us/uwp/api/windows.ui.composition.compositor.createhostbackdropbrush), [Effect factory](https://learn.microsoft.com/en-us/uwp/api/windows.ui.composition.compositor.createeffectfactory), [Dispatcher creation and shutdown](https://learn.microsoft.com/en-us/windows/win32/api/dispatcherqueue/nf-dispatcherqueue-createdispatcherqueuecontroller). The renderer ownership finding comes from the pinned local eframe 0.36.2 and WGPU 30 source.

## Matching the complete silhouette

`src/silhouette.rs` defines one convex continuous-corner path. The UI surface and border, native polygon region, and shadow all use that geometry. The corners use fourth-order superellipse arcs whose curvature approaches zero at the straight-edge joins. Native regions remain pixel-based; their mask cannot provide fractional alpha antialiasing, so the GUI paints the fine antialiased border within the same boundary.

The separate shadow window stays click-through and does not activate. Native edge hit testing remains available while the main window is resizable.

## Verification limits

The app's framebuffer screenshot contains only the egui rendering. It excludes both companion windows, so it cannot demonstrate the desktop blur or visible shadow. Native diagnostics verify effect/resource creation and the companion's window lifecycle separately. Actual visual appearance over light, dark, and busy backgrounds must be checked on the composed desktop; see VALIDATION.md for recorded results.
