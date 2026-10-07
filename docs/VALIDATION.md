# Validation

Automated checks and packaged app rendering apply to the Windows x64 build of Shard Typer 0.1.5. Earlier live desktop checks are identified by version below.

## Automated checks

- Rust formatting and Clippy with warnings treated as errors: passed.
- 23 tests: passed. These cover sampled bounds, skew versus the normalized curve and closed-form moments, density changes with deviation, uniform and constant timing, extreme bounds/tiny deviation, pace and duration feasibility, countdown cancellation, focus-loss pause, resume position, Unicode graphemes, modifier release, partial-input failure, live timing changes, BOM imports, atomic settings replacement/corrupt-file recovery, and migration from the original tab layout and glass appearance.
- Native frame regression: a hidden scratch HWND verifies caption removal across pin/collapse-style changes, full client/outer geometry, edge resizing, the actual polygon region at corners and straight edges, pill hit testing, shadow lifecycle, and unchanged foreground focus. Negative monitor-coordinate hit testing and Gaussian shadow fade are also covered. Shared continuous-corner geometry tests check convexity, symmetry, edge joins, signed shadow distance, pills, and extreme radii; pulse mesh fade/end caps are tested separately.
- Native blur regression: passed on Windows build 26200. A disposable off-screen HWND verifies actual Gaussian-effect factory load success and `S_OK` extended status, HostBackdropBrush binding, attached sprite/root, matching native silhouette/bounds, main/blur/shadow ordering, pin/unpin, deactivation, minimize/hide, destruction, and unchanged foreground focus. It does not measure the composed desktop pixels.
- Native optimized Windows executable: built successfully.
- PE inspection: GUI subsystem and embedded resources present; imported DLLs are Windows system libraries. No developer/runtime DLLs need bundling.

## Desktop checks

| Scenario | Status |
| --- | --- |
| App rendering and screenshot capture | Passed using the app's own RGBA framebuffer capture; this is not a composed desktop screenshot. The restored curve halo was compared with 0.1.4 and inspected in idle and simulated typing previews: its gradient and rounded endpoints fade smoothly without clipping. |
| Alpha output | Checked: all four outer corner pixels have alpha 0; the default empty surface has alpha 140/255. Foreground text remains opaque. Captures exclude the native blur layer and external shadow. |
| Combined text/timing view and preview-pill layout | Inspected idle, simulated typing, Settings, target feedback, and pill. Timing sliders and Mid fit at the default size. At the minimum 400 × 640 logical size, text and timing scroll independently with a visible overflow handle. Settings now has a distinct framed surface and fixed title/Back toolbar outside its scroll area. |
| Native caption, edge resizing, region, and shadow lifecycle | Hidden native-window regression passed; all four corners against live light/dark backgrounds and visible shadow still need a desktop check |
| Live desktop UI checks | The 0.1.4 safe preview launched through the documented Computer Use API. Settings opening, selected icon, full pane height, scrolling to the final action, fixed toolbar, and Back navigation passed. An initial height bug was found and corrected before packaging. Preview windows were closed; no input was sent to the user's existing app. Captures are inconsistent: some show foreground, while another shows only underlying desktop imagery despite the correct accessibility tree. |
| Native blur effect | Real shader load and lifecycle passed the native diagnostic. Persistent blur pixels and foreground readability over live light/dark/busy backgrounds remain visually unverified by the capture tool. |
| Hover, selected, pressed, keyboard focus, and control interaction | Rendering paths and selected states inspected in framebuffer captures. Native Settings/Back navigation and scrolling passed. Exhaustive pointer/keyboard states and all remaining controls still need the manual checklist. |
| Packaged portable executable launch | Passed in preview mode; rendered and exited successfully |
| Installer build | Passed with Inno Setup 6.7.3; install/uninstall on a fresh account still needs a manual check |
| Windows input probe | Could not complete: the execution desktop refused foreground focus. The probe's guard prevented input from reaching another window. |
| Real input into Notepad | Not verified here |
| Chrome/Edge text fields and Google Docs | Not verified here |
| Cross-monitor moves and different DPI settings | Rendering/native masks share the actual egui paint scale; material and geometry refresh on monitor, DPI, display, and theme changes. Framebuffer captures ran at 125% scaling. Moving between the user's physical displays is not verified. |
| Fresh Windows account without developer tools | Not verified on a separate account; binary dependencies inspected |

## Manual acceptance checklist

1. Paste a short multiline sample with accents and emoji. Start Cursor mode, select a new empty Notepad document, and verify exact output, paragraph breaks, and progress.
2. Select Hotkey mode and verify start, pause, and resume using Ctrl + Alt + F8. Confirm Esc stops and resets a session.
3. Switch to a different window mid-run. Confirm typing pauses and resumes at the next character after selecting a destination and pressing the hotkey.
4. Change timing while paused; resume and confirm the live curve and subsequent pace match the new settings. Enter an invalid bounds draft and confirm the last valid configuration remains active.
5. Exercise shaped, skewed, uniform, equal-bound, and zero-deviation settings. Check that the optional target duration leaves timing unchanged.
6. Verify Enter and Shift + Enter in each intended editor. Test Chrome and Edge textareas plus an empty Google Docs document; record compatibility per editor.
7. Drag the text/timing divider and confirm both panels remain visible and scroll independently. Click Mid with asymmetric bounds and confirm Center becomes their midpoint. Open Settings, verify its highlighted title-bar icon, distinct panel and larger heading, then return using the fixed Back to typing button. Adjust tint over light, dark, and busy backgrounds. On Windows 11, toggle Blur background and verify text behind the app becomes softened while foreground text stays crisp; select another application and confirm blur remains. On Windows 10, confirm the explicit support notice and tinted transparency. Pin, collapse, expand using the same pointer position, minimize, close, and reopen. Confirm saved text/settings and idle restore. Move between displays at different scaling settings; inspect all corners, foreground clarity, caption removal, resize edges, and shadow.
8. Unzip and launch the portable download on another Windows account. Install using the setup executable, find the Start menu shortcut, optionally create a desktop shortcut, and uninstall.

The input probe is opt-in: `cargo run --locked --example input_probe`. It creates a disposable editor, refuses to type if that window is not foreground, and closes the editor after testing.

The interactive visual probe is opt-in: `cargo run --locked --example visual_probe`. It renders the current UI without an input worker, hotkey registration, or settings writes. Framebuffer preview switches are documented in README.md.
