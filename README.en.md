# R Battery

[简体中文](README.md) | **English**

A lightweight peripheral battery monitor and mouse settings utility for Windows, retaining detailed ROG support while adding selected Logitech, Rapoo, VGN, VXE, ATK, Razer, Lamzu, WLMouse and MCHOSE devices and standard HID battery queries. It communicates through Windows HID interfaces, runs as a single executable, and uses no background service or network connection. ROG devices do not require Armoury Crate.

[Download portable build](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=en-US) · [Changelog (Chinese)](CHANGELOG.md)

Support covers selected devices with implemented protocols, not entire brands. New mouse and headset adapters have simulated protocol coverage but still require per-model hardware validation. See [compatibility notes](COMPATIBILITY.en.md) for the exact scope.

## In development

The development branch adds USB and Bluetooth battery reads for Sony DualSense / DualSense Edge, displaying coarse values as numbers prefixed with ≈. This feature is not part of the 1.4.6 tag. Protocol regressions pass; hardware acceptance remains pending. See [compatibility notes](COMPATIBILITY.en.md).

Windows XInput battery reads are also integrated when the current driver returns a valid battery grade. System slots are shown separately, four-level estimates carry ≈, and wired or unknown battery types do not produce invented charge or charging status. Hardware acceptance remains pending.

Sony INZONE H5 and H9 II receiver battery queries are integrated on the development branch, with model-specific control interfaces and separate percentage and charging data. No headset settings are written; hardware acceptance remains pending.

## Version 1.4.6

Devices with supported numerical DPI writes can save 1–4 software presets on the Performance page, sorted by DPI, and switch from the main window or tray. Reducing the stage count hides the highest values and keeps them for restoration. Presets remain separate from actual device stages; connections that write stored DPI show a note. Local regressions and render checks passed; per-model hardware acceptance remains pending.

This version also adds profile-scoped Rapoo stage reads and switching, selected MCHOSE keyboard battery queries, and Arctis Nova headset status. Level-only readings display estimated percentages prefixed with ≈; exact readings retain the existing numeric style. Hardware acceptance remains pending; see [compatibility notes](COMPATIBILITY.en.md).

Version 1.4.6 is released as a portable build first. Microsoft Store submission is handled separately after stability acceptance.

## Version 1.4.5

- Adds battery, actual X/Y DPI stages and polling-rate capabilities for selected VGN / VXE / ATK mice.
- Adds Razer battery and settings support, with stage switching, editing and rate controls on reviewed connections.
- Adds read-only battery, stage and rate queries for selected Lamzu / WLMouse devices.
- Adds ATK N9 V2 Ultimate and MCHOSE V9 Turbo+ headset battery queries.
- Extends Logitech DPI and polling-rate controls with readback verification.
- Fixes intermittently unknown Rapoo battery readings during continuous reports; Rapoo DPI remains read-only.
- Improves temporary read failures, cache expiry and identity changes while retaining existing ROG controls.

## Getting started

Install from Microsoft Store, or download and run `rog-mouse-ctl.exe`. No administrator rights are required. Connect a supported device and the app will read its status automatically.

### Devices and tray

Click a device card to open its details. With multiple devices, return to the overview to switch devices. A single device opens directly.

One numeric tray icon shows the selected device's battery level. Click it once to open a panel centered on the tray icon, with device battery levels, refresh, and settings. Double-click to open the main window, or right-click for the menu.

Device preferences provide tray selection, device name, and device type, in that order. Names can be edited or restored. Types default to automatic detection and can be set to Mouse, Keyboard, Headset, Gamepad, or Other. Your choice applies throughout the main window and tray. Changing the type does not add hardware capabilities.

In the tray device list, a mouse opens its known DPI stages, while a headset opens audio controls for speaker or headset volume, microphone mute, and Windows sound settings. Windows remains responsible for changing the default playback device; the app does not call undocumented interfaces.

### Switching to English

Click the gear in the upper-right corner, then choose **Settings (设置) → General (常规) → Language (语言) → English**. The change takes effect immediately.

The default is **Follow system**, which uses the Windows **display language**. Moving English to the top of the Windows Preferred languages list alone may not change the display language. You can always select English directly in the app.

### Features

| Feature | Details |
| --- | --- |
| Battery | Battery level, charging state, device information, 48-hour history, and runtime estimates. |
| Mouse performance | Existing detailed ROG DPI, polling-rate, debounce and related settings; other mice expose numerical DPI, actual stage switching, X/Y stage editing and rates according to protocol capabilities. Rapoo can read and switch stages in the current profile; hardware acceptance is pending. |
| Buttons | Button remapping, configuration export, and backup restoration on supported models. |
| Lighting | Opens Windows Dynamic Lighting; Windows manages the effects. |
| Settings | Theme, startup, close behavior, refresh interval, storage locations, and diagnostic logs. |

Capabilities depend on the model, firmware, and connection. Unsupported settings are identified in the interface. Changing lift-off distance resets surface calibration; review the prompt before applying it.

## Requirements and compatibility

- Windows 10 version 1809 (build 17763) or later, x64.
- Dynamic Lighting requires a supported Windows 11 version and compatible hardware.
- A supported USB HID device or wireless receiver. Bluetooth support depends on the protocol exposed by the device.

ROG HARPE ACE MINI has been verified over USB and through the ROG OMNI receiver. See the [model catalogue](README.md#支持的型号) for recognized mice, keyboards, controllers, and receivers. Inclusion in the catalogue does not mean every feature has been verified on that hardware. Headset battery support depends on its HID protocol; a device type label alone does not establish compatibility.

Other software such as Armoury Crate may keep the device busy. Close it if reads or writes fail. Wake a sleeping device before refreshing.

## Local data and privacy

The app makes no network connections and sends no telemetry. Settings, device aliases, battery history, caches, backups, and diagnostic logs stay on your computer.

| Data | Default location |
| --- | --- |
| Settings | `%APPDATA%/rog-mouse-ctl/settings.ini` |
| Battery history | `%LOCALAPPDATA%/rog-battery/history.tsv` |
| Snapshot cache | `%LOCALAPPDATA%/rog-mouse-ctl/snapshot.cache` |
| Backup directory | `%USERPROFILE%/Documents/rog-mouse-ctl/backups/` |

Settings and history locations can be changed in the app. Windows manages Store app data. Portable data and files saved to custom locations may require manual removal. See the [privacy policy](PRIVACY.md).

## Command line

```powershell
rog-mouse-ctl.exe info
rog-mouse-ctl.exe quick
rog-mouse-ctl.exe setdpi 2 1600
rog-mouse-ctl.exe backup
rog-mouse-ctl.exe estimate
rog-mouse-ctl.exe --help
```

Commands such as `setdpi` change device settings. See the command help for arguments.

## Building

Install the Rust GNU toolchain and MinGW-w64 GCC, with GCC available on PATH.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release --locked
```

The executable is `target/release/rog-mouse-ctl.exe`. Required Windows import libraries and the icon resource are included. To rebuild the icon resource, run `windres -O coff -I assets -i assets/app.rc -o assets/app.o`.

Local release validation runs regression tests, builds the portable executable, checks versions and the PE subsystem, and generates SHA256 checksums:

```powershell
powershell -File tools/build-release.ps1
```

Portable outputs are collected in `dist/version/`. Pushing the source and matching release tag lets GitHub Actions build and publish the portable version. MSIX packaging and Microsoft Store submission use a separate manual workflow.

Development branches can use manual GitHub Actions runs to create test packages: leave the portable workflow's tag empty; "submit to Microsoft Store" is off by default in the MSIX workflow. Download build outputs from the run's Artifacts. After stability acceptance, select the source revision to release and enable the Store submission option.

For manual MSIX packaging, install the Windows SDK and run:

```powershell
powershell -File msix/build-msix.ps1 -Verify
powershell -File msix/make-upload.ps1
```

Keep the version in `Cargo.toml` aligned with the four-part Store version in `msix/package.props.psd1`. Publish from a clean working tree: push the source commit, then create the matching `vX.Y.Z` tag. The tag, source version, executable version, and package version must agree. Tags trigger only the portable release. Manual Store submission also requires the configured credentials and product ID.

## Feedback and license

Backends poll independently and existing ROG controls are retained. Mouse, keyboard and headset support is assessed separately; a brand name alone does not establish compatibility. Supported Logitech onboard modes retain actual stages, including the fifth stage; common-value buttons are not G HUB's configured stages. Rapoo nRF54L stage switching requires matching configuration and fresh telemetry. See [device compatibility](COMPATIBILITY.en.md) for capabilities and hardware validation status.

The `devices` command reads device status; `device-report` exports descriptors and the current process's query summary. `device-report --probe` runs read-only queries first and exports battery, charging status, current DPI, available onboard stages and communication results from the same process for user-assisted testing. Reports exclude payloads and unique device identifiers and are never uploaded automatically.

Report problems or compatibility results in [GitHub issues](https://github.com/youmikk/ROG-Battery/issues). Remove serial numbers and personal file paths before sharing logs.

This repository does not currently include a LICENSE file. Copyright remains with the author. Please open an issue before reusing the code in another project.
