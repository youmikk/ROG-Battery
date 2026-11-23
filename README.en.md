# R Battery

[简体中文](README.md) | **English**

A lightweight peripheral battery monitor and mouse settings utility for Windows, retaining detailed ROG support while adding selected Logitech, Rapoo, VGN, VXE, ATK, Razer, Lamzu, WLMouse, MCHOSE, AULA, AJAZZ, Dareu, Edifier, 8BitDo, HECATE, Sony, HyperX, Cherry and Arctis Nova devices, plus XInput and standard HID battery queries. It communicates through Windows HID and system APIs, runs as a single executable, and uses no background service or network connection. ROG devices do not require Armoury Crate.

[Download portable build](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=en-US) · [Changelog (Chinese)](CHANGELOG.md)

Support covers selected devices with implemented protocols, not entire brands. New adapters have simulated protocol coverage but still require per-model hardware validation. See [compatibility notes](COMPATIBILITY.en.md) for the exact scope.

## Unreleased improvements

The taskbar battery strip uses native Windows fonts and device icons, with widths measured from the rendered glyphs. Both single and multiple devices show an icon and percentage, with bright green charging indicators throughout the app. Device reordering, background operation with only the strip enabled, and malformed headset reports are also fixed.

Battery support adds VGN Dragonfly3 Pro and six ryMs receivers, selected MCHOSE 4D receivers used by K7 V2 / A7 V3 mice, the MCHOSE K99 V3 keyboard, the MOONDROP GM01 Pro headset and a legacy Rapoo VT receiver. Windows unnumbered HID input now uses the full receive capacity so valid replies are not lost to an incorrectly sized buffer. These new paths read battery only and do not expose DPI settings. Exact model, connection and hardware-validation limits are in the [compatibility notes](COMPATIBILITY.en.md). The usage notes below describe the current source; the published download remains 1.4.12.

## Version 1.4.12

This version is released as a portable build first. It adds MCHOSE V9 PRO headset support after reading receiver firmware through a descriptor-proven `FF22` Feature `AA/64`; versions below `0008` are blocked, while unknown versions retain the conservative fallback. Per-model hardware validation is still pending.

## Version 1.4.11

This version is released as a portable build first. FF90 headset handling now adds ATK N9 V2 ULTRA and distinguishes ATK discharging, charging and full states. MCHOSE V9 Turbo+ now separates external power with unknown level from explicit offline, so silence or external power is never shown as 0% / 100%.

## Version 1.4.10

This version is released as a portable build first. It adds read-only battery support for EWEADN S9 PRO 8K, EWEADN S9 (M156), MCHOSE G87 V2, Flydigi APEX5 and GameSir Nova / Cyclone 2. The user-verified S9 PRO 8K path reported 23%–98% and charging data. Every path sends only reviewed queries; no configuration, rumble, lighting or mode writes are exposed.

## Version 1.4.9

This version is released as a portable build first. The taskbar battery strip now uses compact dimensions that match the system tray. A single device shows only its number; multiple devices are shown in order with a type glyph and percentage, using enough spacing and only whole entries that fit in the available taskbar space. VGN / VXE adds strict read-only battery support for five 2.4 GHz keyboard receivers; `258A:024F` uses the documented charging bit while charging remains unknown on the other IDs.

## Version 1.4.8

This version is released as a portable build first. It adds the taskbar battery strip and continues support for commonly used Chinese brands including Rapoo, VGN / VXE / ATK, MCHOSE, AULA, AJAZZ, Dareu, Edifier, 8BitDo and HECATE. New protocols pass local simulated regressions but still require per-model hardware acceptance. Exact receiver and model limits are documented in the [compatibility notes](COMPATIBILITY.en.md).

MCHOSE mouse work adds Feature-protocol battery, identity and actual-stage reads for A7 V2 and selected L7 models, with existing-stage switching on A7 V2 Ultra+ / L7 Pro+. Some older A7 Ultra receivers are documented not to return battery; a shared name does not guarantee compatibility. A7 V3 and old A5 use other protocols and are outside version 1.4.8's scope.

AJAZZ AJ179 PRO adds 2.4 GHz battery and actual X/Y stage reading/switching over USB or its receiver. The mouse's own ID establishes its model; switching reads the current profile and preserves all stage values and colors. Charging and wired battery remain unknown, Bluetooth is not integrated, and hardware acceptance is pending.

AULA SC380 PRO adds 2.4 GHz battery and actual X/Y DPI-stage reading/switching. The model signature is checked before reading configuration; switching preserves other settings and verifies the complete block afterward. Charging remains unknown. Wired mode and other AULA models are not integrated yet; hardware acceptance is pending.

AULA F75 and F87 PRO now receive 2.4 GHz battery and charging reads. They share a `3554:FA09` receiver with some MCHOSE keyboards, so the app selects framing from the complete model code and verifies identity before and after the battery query. F87 remains disabled because its official wireless collection entry conflicts with this path.

8BitDo Pro 3 adds read-only battery and charging display in D-input mode. The adapter requires the standard gamepad collection and only Input Report `04`, reads the existing state stream and sends no mode, rumble or lighting command. XInput mode continues to use the system battery API.

VGN Dragonfly3 SE receivers add a battery query. The adapter requires `391D:1A01 / FF01:0010` with 65-byte unnumbered input/output reports and sends only the fixed `55 30` query. It reads no configuration and changes no DPI stage; a wired reply clears the old wireless percentage, while charging remains unknown.

HECATE G3M PRO adds battery percentage reads through its 2.4 GHz receiver. The adapter requires `320F:706E / FF1C:0092` and 64-byte Report `04`, reads no DPI, lighting or other configuration, and keeps charging unknown.

## Version 1.4.7

USB and Bluetooth battery reads are added for Sony DualSense / DualSense Edge, displaying coarse values as numbers prefixed with ≈. Protocol regressions pass; hardware acceptance remains pending. See [compatibility notes](COMPATIBILITY.en.md).

Windows XInput battery reads are also integrated when the current driver returns a valid battery grade. System slots are shown separately, four-level estimates carry ≈, and wired or unknown battery types do not produce invented charge or charging status. Hardware acceptance remains pending.

Sony INZONE H5 and H9 II receiver battery queries use model-specific control interfaces and separate percentage and charging data. No headset settings are written; hardware acceptance remains pending.

HyperX Cloud Alpha 2 Wireless base-station battery, charging and voltage queries are also integrated, checking headset connection before and after battery reads and filtering unrelated traffic. Hardware acceptance remains pending.

Keyboard coverage adds CHERRY MX 2.0S WL Pro USB / 2.4 GHz battery and charging queries, limited to reviewed identities and management collections. Bluetooth and other Cherry models are outside this addition; hardware acceptance remains pending.

Standard HID strengths scaled from descriptor ranges also carry ≈, while native percentages retain ordinary numbers. Version 1.4.7 is released as a portable build first; Microsoft Store submission follows stability acceptance separately.

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
