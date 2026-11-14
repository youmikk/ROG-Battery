# R Battery

[简体中文](README.md) | **English**

A lightweight battery monitor and mouse settings utility for ROG peripherals on Windows. It communicates through Windows HID interfaces without Armoury Crate, runs as a single executable, and uses no background service or network connection.

[Download portable build](https://github.com/youmikk/ROG-Battery/releases/latest) · [Microsoft Store](https://apps.microsoft.com/detail/9nrc8hcd4bz7?hl=en-US) · [Changelog (Chinese)](CHANGELOG.md)

## Version 1.4.3

- Switch between known DPI stages directly from the tray mouse panel.
- Open a headset panel to adjust speaker or headset volume, mute the microphone, and open Windows sound settings.
- Headset audio controls are also available in the main window and share their state with the tray.
- Drag devices to reorder the list; the order is saved.
- Settings are organized into General, Battery & tray, Data & storage, and About.
- Smoother tray panel animations, clearer text, and a fix for occasional unintended main-window activation.
- Microsoft Store builds retain the Windows low battery notification; portable builds do not.

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
| Mouse performance | Four DPI stages, polling rate, debounce, angle snapping, angle tuning, and lift-off distance on supported models. |
| Buttons | Button remapping, configuration export, and backup restoration on supported models. |
| Lighting | Opens Windows Dynamic Lighting; Windows manages the effects. |
| Settings | Theme, startup, close behavior, refresh interval, storage locations, and diagnostic logs. |

Capabilities depend on the model, firmware, and connection. Unsupported settings are identified in the interface. Changing lift-off distance resets surface calibration; review the prompt before applying it.

## Requirements and compatibility

- Windows 10 version 1809 (build 17763) or later, x64.
- Dynamic Lighting requires a supported Windows 11 version and compatible hardware.
- A supported ROG USB HID device or wireless receiver. Bluetooth compatibility is not guaranteed.

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

Portable outputs are collected in `dist/version/`. Push the source and matching release tag to let GitHub Actions build and publish the portable version, package and validate MSIX, and submit it to Microsoft Store using the configured credentials. For manual MSIX packaging, install the Windows SDK and run:

```powershell
powershell -File msix/build-msix.ps1 -Verify
powershell -File msix/make-upload.ps1
```

Keep the version in `Cargo.toml` aligned with the four-part Store version in `msix/package.props.psd1`. Publish from a clean working tree: push the source commit, then create the matching `vX.Y.Z` tag. The tag, source version, executable version, and package version must agree. Pushing a release tag triggers portable release and MSIX workflows. Store submission also requires the configured credentials and product ID.

## Feedback and license

Report problems or compatibility results in [GitHub issues](https://github.com/youmikk/ROG-Battery/issues). Remove serial numbers and personal file paths before sharing logs.

This repository does not currently include a LICENSE file. Copyright remains with the author. Please open an issue before reusing the code in another project.
