# Shard Typer

A small Windows typing utility with a floating charcoal glass interface and pale icy highlights. Paste a block of text, pick a destination, and let it arrive one character at a time.

## Open the app

- **Portable:** unzip the portable download and double-click `ShardTyper.exe`.
- **Installer:** run the setup download, then search **Shard Typer** in the Start menu. A desktop shortcut is optional.

Release downloads contain the finished app. You do not need Rust, .NET, a browser runtime, or developer tools to run it.

## Use it

1. Paste your text or import a UTF-8/UTF-16 `.txt` file in the text editor.
2. In **Cursor** mode, click **Start**, then select the destination during the five-second countdown.
3. In **Hotkey** mode, select the destination and press **Ctrl + Alt + F8**.
4. The hotkey pauses and resumes the session. **Esc** or **Stop** cancels and resets it.

The app pauses when the destination window loses focus. Select your destination and press the hotkey to resume at the saved position. The pin keeps the panel above other windows; collapse switches to a thin preview pill.

Text and timing share one screen. Drag the divider between them to give either panel more room; double-click it to reset. Each panel scrolls independently.

**Timing** shows a compact, softly glowing icy curve and fading pulses for recent measured intervals. The curve uses a consistent density scale as deviation grows. The average marker and resulting spread account for skew and truncation. The centered Normal/Uniform selector and aligned parameter rows share one control grid. **Mid** sets Center halfway between Min and Max. Change the bounds, center, standard deviation, and skew while typing: the next unscheduled character uses the new settings. Invalid drafts leave the last valid settings active.

**Target** calculates the WPM needed for your chosen duration. It is a planning aid and does not automatically change your typing settings. WPM uses five output characters per word. Countdown and pause time are excluded from estimates.

Open **Settings** using the sliders icon. A distinct glass pane contains appearance and typing settings, with a fixed **Back to typing** button above it. The highlighted title-bar icon makes the current view clear. Adjust background blur, tint, startup delay, hotkey, and newline behavior here. Enter is the default; Shift + Enter is available. Line endings are normalized, tabs become four spaces, and emoji/combined characters are submitted together.

The default surface has a 55% charcoal tint with opaque white text, a fine border, and a restrained shadow. On Windows 11, **Blur background** uses a native composition host underneath the UI to soften desktop content, including while another app has focus. The window, blur layer, border, and shadow share a continuous-corner silhouette. Hover highlights only the current control. Windows 10 retains tinted transparency and reports that background blur needs Windows 11. See [material implementation and limitations](docs/MATERIAL.md).

## Timing defaults

| Setting | Default |
| --- | --- |
| Minimum / maximum | 50 / 450 ms |
| Center | 250 ms |
| Standard deviation | 70 ms |
| Skew | 0 |
| Average default pace | 48 WPM |

Shaped mode uses a truncated skew-normal distribution. The center and deviation describe the underlying normal distribution. Skew and bounds change the final mean and spread; the visible curve and estimates account for both. Positive skew adds a longer tail toward long delays, negative skew toward short delays. Uniform mode samples evenly across the interval. Both use a pseudorandom generator; increasing deviation is not a switch to hardware randomness.

The configured bounds apply to sampled delays. Actual measured intervals also include Windows scheduling and input-submission overhead, so the live display can show an outlier. It measures submission intervals rather than an editor's rendering or acknowledgement.

## Local storage and compatibility

Text and settings are stored in `%LOCALAPPDATA%\ShardTyper\settings.json`. Reopening always starts idle. **Forget saved text** clears the stored text. Invalid settings are backed up before defaults are saved.

V1 targets **Windows 10/11 x64**. Focus protection detects a change of foreground window, not a different tab, field, or caret within that window. Windows input support varies by editor and does not guarantee that a particular paste restriction or revision-history tool behaves a certain way. See [validation results](docs/VALIDATION.md).

## Build from source

The easiest supported development setup is [Rust via rustup](https://rust-lang.org/tools/install/) with the Windows MSVC toolchain and Visual Studio C++ build tools plus the Windows SDK. The project pins Rust in `rust-toolchain.toml` and package versions in `Cargo.lock`. The repository also supports a local GNU toolchain configured by `scripts/dev-env.ps1` when available.

From PowerShell in this folder:

```powershell
./scripts/build.ps1
./scripts/test.ps1
./scripts/package.ps1 -SkipBuild
```

The executable is created at `target\release\ShardTyper.exe`; downloads are created in `dist`. Packaging the installer requires [Inno Setup 6](https://jrsoftware.org/isdl.php). Use `-PortableOnly` to package only the ZIP or `-Iscc <path>` to select a compiler.

For development:

```powershell
cargo run --locked
cargo run --locked -- --preview --timing
cargo run --locked --example visual_probe
cargo run --locked --example input_probe
```

The opt-in input probe creates its own disposable Windows editor. Preview mode and `visual_probe` never register hotkeys, send input, or save settings. To capture the app's rendered UI for visual checks:

```powershell
./target/release/ShardTyper.exe --preview --timing --screenshot .build/screenshots/timing.png
```

Additional preview switches are `--controls` (Settings), `--pill`, `--small`, and `--target`. Captures contain the app's RGBA rendering; they exclude the native blur host and shadow, so they cannot establish desktop material appearance.

## Put it on GitHub

Upload the source files, assets, scripts, packaging, and `.github` folder. Keep `.build`, `target`, and `dist` out of Git; `.gitignore` already covers them. Do not upload your local settings file.

See [step-by-step GitHub upload instructions](docs/GITHUB.md) for the website and GitHub Desktop methods, including an existing repository with a license.

The Windows workflow runs formatting, lint, tests, and packaging, then provides downloadable artifacts. Pushing a tag such as `v0.1.5` creates a **draft** release containing the portable ZIP, installer, and checksums for you to review and publish.

## Project layout

- `src/timing.rs`: sampling, normalized curve, pace and duration estimates.
- `src/engine.rs`: independently tested session controller and background worker.
- `src/platform.rs`: Windows input, hotkeys, glass, clipboard, and instance lock.
- `src/backdrop.rs`: native Windows composition host and Gaussian desktop blur.
- `src/ui.rs`: draggable text/timing split, chart, controls, and preview pill.
- `src/design.rs`: shared surface, spacing, and interaction tokens.
- `src/silhouette.rs`: shared continuous-corner geometry for drawing, clipping, and shadow.
- `src/glow.rs`: smoothly feathered curve and measured-interval pulse meshes.
- `src/settings.rs` and `src/text.rs`: persistence and Unicode text preparation.
- `scripts`, `packaging`, `.github`: builds, downloads, and release automation.

Licensed under MIT. The application uses open-source dependencies under their respective licenses.
