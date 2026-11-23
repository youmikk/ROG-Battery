# R Battery

[简体中文](README.md) | **English**

A lightweight peripheral battery monitor and mouse settings utility for Windows, retaining detailed ROG support while adding selected Logitech, Rapoo, VGN, VXE, ATK, Razer, Lamzu, WLMouse, MCHOSE, AULA, AJAZZ, Dareu, Edifier, 8BitDo, HECATE, Sony, HyperX, Cherry and Arctis Nova devices, plus XInput and standard HID battery queries. It communicates through Windows HID and system APIs and runs as a single executable without a background service. Normal battery monitoring and device settings work offline. Network access occurs only when you submit feedback, add a reply, or check for replies; there is no background telemetry. ROG devices do not require Armoury Crate.

[Download portable build](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=en-US) · [Changelog (Chinese)](CHANGELOG.md)

Support covers selected devices with implemented protocols, not entire brands. See [compatibility notes](COMPATIBILITY.en.md) for model coverage, capabilities, protocol testing, and hardware validation status.

## Latest release: 1.4.7

- Expand battery queries for common domestic mice, keyboards, headsets and controllers while preserving deep ROG support. Some other brands can read and switch actual DPI stages.
- Display multiple devices in a compact taskbar battery widget with type icons and bright green charging indicators. Clicking opens the quick panel; interaction, ordering and background operation are improved.
- Redesign About and Device feedback with optional diagnostic previews and screenshots. In-app submission and replies require a configured relay; the current release opens GitHub in the browser, where screenshots must be attached manually.

This release consolidates the former 1.4.7–1.4.12 updates under 1.4.7. Users of those portable versions can exit the app and replace the executable. Microsoft Store publishing is handled separately. See [Compatibility](COMPATIBILITY.en.md) for model coverage and hardware validation, [Changelog](CHANGELOG.md) for history and [Protocol research](PROTOCOLS.en.md) for evidence.

## Getting started

Install from Microsoft Store, or download and run `rog-mouse-ctl.exe`. No administrator rights are required. Connect a supported device and the app will read its status automatically.

### Devices and tray

Click a device card to open its details. With multiple devices, return to the overview to switch devices. A single device opens directly.

One numeric tray icon shows the selected device's battery level. Click it once to open a panel centered on the tray icon, with device battery levels, refresh, and settings. Double-click to open the main window, or right-click for the menu.

A compact battery strip appears immediately left of the notification area. Both single and multiple devices show their type icon and percentage in the current device order, using native fonts at the taskbar's scale. Click the strip to open quick view and click it again to close. It can be disabled independently in Settings and hides when the taskbar is automatically hidden or a full-screen application covers its monitor.

Choosing Minimize when closing the window keeps the app running while either the tray icon or battery strip is available. If neither entry is available, the app retains an ordinary minimized window so it can still be reopened.

Device preferences provide a primary tray icon, device name, and device type, in that order. The primary selection controls the standard Windows tray icon, while the taskbar strip shows every device with battery data. Names can be edited or restored. Types default to automatic detection and can be set to Mouse, Keyboard, Headset, Gamepad, or Other. Your choice applies throughout the main window and both battery displays. Changing the type does not add hardware capabilities.

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
| Settings | Theme, startup, close behavior, refresh interval, storage locations, diagnostic logs, and device feedback. |

Capabilities depend on the model, firmware, and connection. Unsupported settings are identified in the interface. Changing lift-off distance resets surface calibration; review the prompt before applying it.

## Requirements and compatibility

- Windows 10 version 1809 (build 17763) or later, x64.
- Dynamic Lighting requires a supported Windows 11 version and compatible hardware.
- A supported USB HID device or wireless receiver. Bluetooth support depends on the protocol exposed by the device.

ROG HARPE ACE MINI has been verified over USB and through the ROG OMNI receiver. See the [model catalogue](README.md#支持的型号) for recognized mice, keyboards, controllers, and receivers. Inclusion in the catalogue does not mean every feature has been verified on that hardware. Headset battery support depends on its HID protocol; a device type label alone does not establish compatibility.

Other software such as Armoury Crate may keep the device busy. Close it if reads or writes fail. Wake a sleeping device before refreshing.

## Local data and privacy

Normal battery monitoring and device settings work offline, with no background telemetry or automatic crash reports. The app connects to the feedback service only when you submit feedback, add a reply, or check for replies. Settings, device aliases, battery history, caches, backups, and diagnostic logs stay on your computer.

The optional diagnostic summary contains only app and system versions, the original device model, VID/PID, type, connection, battery state, backend, and interface and query summaries. It does not copy serial numbers, paths, custom device aliases, personal settings, or raw logs. Submissions include the app version and basic details of the selected device. Your description, any included diagnostics, and screenshots appear in a public GitHub issue. The feedback service serves screenshots through public image links backed by a private R2 bucket. Images are not re-encoded; check the picture and its metadata for personal content before submitting.

Local feedback history and reply keys are encrypted with Windows DPAPI in the settings directory. Reply keys are never included in public issues. Deleting local history does not remove public GitHub content and removes the credentials needed to reply through the app.

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

Open Device feedback in Settings, select a device and issue category, and describe the problem. You can preview or disable Include diagnostics and attach up to three screenshots. Builds with a configured feedback service can submit directly; My feedback shows status, maintainer replies, and a field for additional information. Network requests occur only when you initiate these actions.

If the feedback service is not configured, use Copy report & open GitHub, review the content, and submit a [GitHub issue](https://github.com/youmikk/ROG-Battery/issues) in your browser. Add screenshots manually. Browser submissions do not appear in My feedback. The diagnostic log entry remains available; the separate device report window is removed, while the command-line reports above remain supported. Remove serial numbers, personal file paths, and other private content before separately sharing logs or backups.

This repository does not currently include a LICENSE file. Copyright remains with the author. Please open an issue before reusing the code in another project.
