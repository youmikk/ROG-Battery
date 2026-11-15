# Device compatibility

[简体中文](COMPATIBILITY.md)

This table separates hardware verified support in the stable app from protocol implementations on the multi-brand development branch. Discovery, model recognition and parser tests do not verify hardware compatibility.

| Brand / protocol | Model / scope | Battery | DPI | Status |
| --- | --- | --- | --- | --- |
| ROG | HARPE ACE MINI, USB / OMNI | Verified | Verified; existing detailed settings retained | Stable release |
| Logitech HID++ 1.0 | Devices exposing battery registers 0x0D or 0x07 | Exact charge or coarse levels implemented | No writes yet | Development branch; hardware validation pending |
| Logitech HID++ 2.0 | Devices exposing 0x1000 or 0x1004 | Protocol implemented | 0x2201 numerical control; actual stage reads and switching for supported 0x8100 onboard profiles | Development branch; hardware validation pending |
| Standard HID | Generic Device Controls / Battery Strength (06:20) in the descriptor | Read using descriptor limits | No standard DPI write operation | Development branch; hardware validation pending |
| Rapoo nRF54L | 77 official driver model entries, covering specific VT0 / VT1 / VT2 / VT3 / VT4 / VT6 / VT7 / VT8 / VT9 variants and ESM models | Status report reads implemented | Current DPI read-only; independent X/Y is not shown as one DPI | Development branch; per-model hardware validation pending |
| VGN Beiying | V98 Pro V3, USB / 2.4G | Wired feature and receiver status queries implemented | No DPI for keyboards | USB read verified; receiver and wake recovery validation pending |

Voltage and coarse HID++ battery levels are not shown as exact percentages. Charging remains unknown when standard HID does not expose it.

HID++ 1.0 receivers that expose pairing information provide per-slot names and device types. Device identities are hashed before they are used for preferences. A temporary identity query failure retains the existing card; first-time identity confirmation migrates custom names, types, tray selection and order without treating a replacement device as the original one. If HID++ 2.0 unified battery reports only a coarse level, other battery features are queried for an exact percentage supplied by the device.

The standard HID adapter reserves listed ROG devices and receivers for detailed ROG support, and checks other devices for battery capabilities in their descriptors. Hardware compatibility still needs validation for each model.

Each backend has its own polling thread and command queue, so a timeout or DPI write does not block polling for other brands. Coarse or missing battery readings remain coarse or unknown instead of becoming 0%. Logitech DPI writes validate advertised values and verify readback; failed writes attempt to restore and verify the original value.

Adjustable DPI lists are read across the device's reported pages, including values and step ranges spanning report boundaries. Transport padding is excluded from parsing. Incomplete lists, invalid report lengths and failed subsequent queries do not authorize DPI writes from partial data.

Logitech's actual onboard stages are separate from the hardware's allowed DPI values. Supported mice in onboard mode expose five DPI slots through `0x8100`; disabled slots do not renumber other slots, and duplicate values are retained. Length, format and CRC are validated before switching is enabled. Switching changes only the current DPI slot, without changing the whole profile, G HUB mode or Flash. Identity and profile are checked again before switching, and readback is verified afterward. A failed switch attempts to restore the previous stage only while the original profile remains active.

G HUB host mode does not guarantee that the computer's configured stages are available over HID++. The interface then retains validated numerical control through `0x2201` and common-value buttons, without labeling these shortcuts as configured stages. Failed onboard queries or unsupported formats retain confirmed current DPI readings but disable numerical writes. Onboard reads currently support known profile formats 1–5 with 256-byte sectors; other formats need additional protocol and hardware validation.

Rapoo now matches the 77 model entries explicitly assigned to `VT_nrf54L` by the official web driver. See the [Rapoo model table](RAPOO_MODELS.md) for model and USB/receiver PIDs. Only status report `0x07` on the `FF00:0002` control collection is read; other controllers and unknown PIDs are not probed with this protocol. Missing valid reports remain unknown and require a user wake test. EEPROM writes are not enabled; protocol inference is not hardware verification.

VGN currently matches V98 Pro V3 `258A:024E/024F/027E/027F` and queries only the confirmed vendor control interface. Wired battery reads use feature report `0x09`; receivers use command `0x4A` in report `0x13`, with length and checksum validation. Unknown IDs and unrelated vendor collections are not probed with this protocol. A USB device returned valid 100% battery and charging status; other battery levels, receivers and wake recovery still need testing.

Settings → About → Device compatibility opens a GitHub form and exports factory model, VID/PID, usage, report lengths, and a summary of the last 128 protocol queries in the current session: feature identifiers, durations and results. The report excludes payloads, serial numbers, MAC addresses, interface paths, usernames, custom device names, DPI setting values and raw logs.

For remote adaptation, run `rog-mouse-ctl device-report --probe` using a development build. It performs read-only queries and then generates a sanitized report from the same process, including returned battery, charging status, current DPI, available onboard stages and the current stage. It changes no settings and uploads nothing. Profile names, button settings and raw memory contents are excluded. Users can compare readings with the official driver; DPI write tests are separate. Local hardware ownership is not required for user-assisted testing, although unknown private protocols may still need driver analysis or targeted communication captures.

Other brands and models listed by FerrisPulse and similar projects have not all been integrated into this branch. Generic catalogues, shared receiver identifiers and validated protocols require separate checks and are not a statement of compatibility with this app.

Protocol references: [Solaar](https://github.com/pwr-Solaar/Solaar), [libratbag](https://github.com/libratbag/libratbag), [USB HID Usage Tables](https://usb.org/document-library/hid-usage-tables-15), [Rapoo A HUB](https://hub.rapoo.cn/), [VGN web driver](https://drive.vgnlab.com.cn/) and [FerrisPulse protocol research](https://github.com/Ferris-echo/FerrisPulse). The implementation does not copy driver or open source project code.
