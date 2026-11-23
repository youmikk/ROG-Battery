# R Battery

[简体中文](README.md) | **English**

**A Windows battery monitor and quick mouse settings tool for peripherals from multiple brands.** Check mice, keyboards, headsets and controllers in one place, with a taskbar battery strip and tray panel for quick access. DPI controls follow each device's capabilities, headset pages provide Windows volume and microphone controls, and supported ROG mice retain detailed settings.

Runs as a single executable without administrator rights or a separate background service. Routine battery monitoring and device settings work offline, with no background telemetry. ROG devices do not require Armoury Crate.

[![Release](https://img.shields.io/github/v/release/youmikk/ROG-Battery)](https://github.com/youmikk/ROG-Battery/releases)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4)](#requirements)
[![Microsoft Store](https://img.shields.io/badge/Microsoft%20Store-R%20Battery-0067B8)](https://apps.microsoft.com/detail/9nrc8hcd4bz7)

[Download portable build](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=en-US) · [Device compatibility](COMPATIBILITY.en.md) · [Changelog (Chinese)](CHANGELOG.md)

## Features

| Feature | Details |
| --- | --- |
| Multiple device batteries | View battery levels and confirmed charging states across brands. Values estimated from coarse levels display `≈`; unavailable readings remain unknown. |
| Taskbar battery strip | Compact device icons and percentages next to the notification area, with multiple devices, Windows scaling and bright green charging indicators. Click to open quick view. |
| Tray controls | Single-click the numeric tray icon for quick view; double-click to open the main window. Supported mice expose DPI shortcuts, and headsets expand to audio controls. |
| Mouse DPI | Numerical DPI, actual stage switching, X/Y stage editing or polling-rate settings where supported. Connections with numerical DPI writes can also save 1–4 software presets, sorted by value. |
| Audio | Adjust speaker and headset volume or mute the microphone through Windows audio APIs. Changing the default output device opens Windows sound settings. |
| Device preferences | Rename devices, override their type, drag to reorder, and choose the device for the standard tray icon. Preferences apply across the main window and taskbar. |
| Detailed ROG settings | Supported mice retain DPI, polling rate, debounce, button mappings, sleep settings and configuration backups. The primary ROG mouse's battery page provides history and runtime estimates. |
| Low-battery notifications | Windows notifications are available in the Store edition, not the portable build. They use valid, precise readings rather than estimates or expired cache entries. |
| Appearance and language | Light and dark themes, system preferences, Simplified Chinese and English, plus startup, close behavior and refresh settings. |

Mouse controls depend on the model, firmware and connection. Hardware DPI stages and software presets are presented separately; presets do not enable writes on read-only devices. Supported Logitech onboard profiles retain their actual stages, including a fifth stage. The 1–4 software preset limit does not change those hardware stages.

## Brands and device coverage

The table groups implemented integrations by device type. **Coverage is limited to selected models and connections; it does not mean every device from a brand is supported or hardware-verified.** See [device compatibility](COMPATIBILITY.en.md) for exact models, charging behavior, DPI capabilities and validation status.

| Device type | Brands and protocols with implemented integrations |
| --- | --- |
| Mice | ASUS / ROG, Logitech, Rapoo, VGN, VXE, ATK, Razer, MCHOSE, AULA, AJAZZ, Dareu, EWEADN, Lingbao, HECATE, Lamzu and WLMouse |
| Keyboards | ASUS / ROG, Logitech HID++, VGN / VXE, MCHOSE, AULA, EWEADN, CHERRY and Lofree |
| Headsets | ATK, MCHOSE, EDIFIER, MOONDROP, XIBERIA, FIFINE, Sony INZONE, HyperX and SteelSeries Arctis Nova |
| Controllers | ASUS / ROG, Sony DualSense / DualSense Edge, 8BitDo, Flydigi, GameSir, and controllers whose current mode and driver expose battery information through Windows XInput |
| Standard HID | Devices whose descriptors expose standard Battery Strength capabilities. This battery protocol does not provide universal DPI settings. |

Rapoo VT and MT models, VGN mice and keyboards, and different AULA keyboard generations can use different protocols. A similar name or shared receiver ID does not establish compatibility. Many new integrations have passed protocol tests but still need validation on the corresponding hardware.

Further details: [Multi-brand compatibility](COMPATIBILITY.en.md) · [ASUS / ROG catalogue](ROG_MODELS.en.md) · [Rapoo nRF54L catalogue (Chinese)](RAPOO_MODELS.md) · [Protocol research](PROTOCOLS.en.md)

## Latest release: 1.4.7

- Expand battery queries for commonly used Chinese mouse, keyboard, headset and controller brands, and improve actual DPI stage reads and switching on selected mice.
- Add a multi-device taskbar battery strip and improve sizing, icons, charging indicators, clicks, ordering and background operation.
- Redesign About and Device feedback, fixing text clipping, alignment and corners, with optional diagnostic previews and screenshot selection. Check for updates is currently a placeholder.

This release consolidates the former 1.4.7–1.4.12 updates under 1.4.7. Users of those portable versions can exit the app and replace the executable. This README describes the current GitHub release; Microsoft Store publishing is separate and may lag behind it. See the [full changelog](CHANGELOG.md).

## Getting started

1. Download and run `rog-mouse-ctl.exe` from [GitHub Releases](https://github.com/youmikk/ROG-Battery/releases/latest), or install from [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=en-US).
2. Connect the device using a mode listed in the compatibility notes. Readings update automatically. A single device opens its details; multiple devices open the device list.
3. Click the numeric tray icon or taskbar strip to open quick view. Click a supported mouse or headset to expand its controls, and click again to collapse them.
4. Open **Settings → Battery & tray** for display options. Device details let you change names, types and the standard tray selection.

### Taskbar and device preferences

The standard Windows tray icon displays one selected device. The taskbar strip shows devices with valid battery readings in the saved order. When space is limited, it displays only complete device items; other devices remain accessible in quick view.

The strip hides with an automatically hidden taskbar or a full-screen app on the same monitor. Choosing Minimize when closing keeps the app running while either the tray icon or strip is available. If neither is available, an ordinary minimized window remains accessible.

Device type defaults to automatic detection, with manual Mouse, Keyboard, Headset, Gamepad or Other overrides. Your choice takes precedence and applies throughout the interface; changing the type does not add hardware capabilities.

### Language

Click the upper-right gear and choose **Settings (设置) → General (常规) → Language (语言)**, then **English**, **简体中文** or **Follow system**. Changes take effect immediately.

The default, Follow system, uses the Windows **display language**, which can differ from the order of Preferred languages. Select English directly if needed.

### Missing battery readings

Check the exact model and connection in the [compatibility notes](COMPATIBILITY.en.md), wake the device, then wait for the next background query or click Refresh. Vendor software may occupy the control interface; close it and retry if necessary. A connected receiver does not always mean the paired device is reporting valid battery data.

If readings remain unavailable, use **Settings → About → Device feedback** to share the model, connection, symptoms and an optional diagnostic summary.

## Requirements

- Windows 10 version 1809 (build 17763) or later, x64.
- A supported USB, wired or 2.4 GHz receiver connection for that model. Bluetooth support is verified separately per protocol.
- The Windows Dynamic Lighting entry requires a supported Windows 11 version and compatible hardware.

## Feedback and privacy

Open **Settings → About → Device feedback**, choose a device and category, and describe the issue. You can preview or disable Include device diagnostics and select up to three screenshots.

**The current release submits through GitHub in your browser.** Use Copy report & open GitHub, review the report and submit it. Add screenshots manually; browser submissions do not appear in My feedback. Builds with a configured feedback service can submit in the app, display maintainer replies and send additional information. See the [feedback service guide](services/feedback-relay/README.md) for deployment.

Routine battery monitoring and device settings work offline, without background telemetry or automatic crash reports. Settings, aliases, history, caches, backups and diagnostic logs stay locally. Network features run only when you submit feedback, add information or check replies.

Optional diagnostics contain app and system versions, original device model, VID/PID, type, connection, battery and query results. They exclude serial numbers, file paths, custom aliases, personal settings and raw logs. Descriptions, included diagnostics and screenshots are public, so review them before submitting. In-app feedback history and reply credentials use Windows DPAPI encryption; deleting local history does not remove public GitHub content. See the [privacy policy](PRIVACY.md) for details.

| Local data | Default location |
| --- | --- |
| Settings | `%APPDATA%/rog-mouse-ctl/settings.ini` |
| Battery history | `%LOCALAPPDATA%/rog-battery/history.tsv` |
| Snapshot cache | `%LOCALAPPDATA%/rog-mouse-ctl/snapshot.cache` |
| Configuration backups | `%USERPROFILE%/Documents/rog-mouse-ctl/backups/` |

Settings and history locations can be changed in the app. The executable and data directories retain the `rog-mouse-ctl` name for compatibility with existing installations and settings. Windows manages Store app data; portable data and files saved to custom locations may require manual removal.

## Command line

Read-only multi-brand queries and diagnostics:

```powershell
rog-mouse-ctl.exe devices
rog-mouse-ctl.exe device-report
rog-mouse-ctl.exe device-report --probe
rog-mouse-ctl.exe --help
```

`devices` lists devices and readings. `device-report` generates a device and query summary; `--probe` performs read-only queries first and reports results from the same process. Reports are not uploaded automatically and exclude raw communication payloads and unique device identifiers.

ROG-specific query and settings examples:

```powershell
rog-mouse-ctl.exe info
rog-mouse-ctl.exe quick
rog-mouse-ctl.exe setdpi 2 1600
rog-mouse-ctl.exe backup
rog-mouse-ctl.exe estimate
```

These commands use the ROG protocol rather than universal multi-brand settings. Operations such as `setdpi` change device configuration; see `--help` for keep and save options. Use the device's in-app controls for supported DPI shortcuts on other brands.

## Building

Install the Rust GNU toolchain and MinGW-w64 GCC, with GCC available on PATH.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release --locked
```

The output is `target/release/rog-mouse-ctl.exe`. Required Windows import libraries and icon resources are included; no additional runtime installation is needed. After changing icons, regenerate resources with `windres -O coff -I assets -i assets/app.rc -o assets/app.o`.

Run local tests, build and version checks before publishing:

```powershell
powershell -File tools/build-release.ps1
```

Outputs are collected in `dist/version/`. Development uses `main`; pushing source and a matching version tag triggers the GitHub Actions portable release. For a build-only check, run the portable workflow manually with an empty tag.

MSIX packaging and Microsoft Store submission use a separate manual workflow, with Store submission disabled by default. `Cargo.toml`, `msix/package.props.psd1`, the tag and executable version must agree. After stability acceptance, enable Store submission as needed and configure the required credentials and product ID.

## License

This repository does not currently include a LICENSE file. Copyright remains with the author. Please open an [issue](https://github.com/youmikk/ROG-Battery/issues) before reusing the code in another project.
