# Device compatibility

[简体中文](COMPATIBILITY.md)

This table separates hardware verified support in the stable app from protocol implementations on the multi-brand development branch. Discovery, model recognition and parser tests do not verify hardware compatibility.

| Brand / protocol | Model / scope | Battery | DPI | Status |
| --- | --- | --- | --- | --- |
| ROG | HARPE ACE MINI, USB / OMNI | Verified | Verified; existing detailed settings retained | Stable release |
| Logitech HID++ 1.0 | Devices exposing battery registers 0x0D or 0x07 | Exact charge or coarse levels implemented | No writes yet | Development branch; hardware validation pending |
| Logitech HID++ 2.0 | Devices exposing 0x1000 or 0x1004 | Protocol implemented | Read and verified writes when 0x2201 is available | Development branch; hardware validation pending |
| Standard HID | Generic Device Controls / Battery Strength (06:20) in the descriptor | Read using descriptor limits | No standard DPI write operation | Development branch; hardware validation pending |
| Rapoo | VT1 | Investigation pending | Investigation pending | Unverified |

Voltage and coarse HID++ battery levels are not shown as exact percentages. Charging remains unknown when standard HID does not expose it.

HID++ 1.0 receivers that expose pairing information provide per-slot names and device types. Device identities are hashed before they are used for preferences. A temporary identity query failure retains the existing card; first-time identity confirmation migrates custom names, types, tray selection and order without treating a replacement device as the original one. If HID++ 2.0 unified battery reports only a coarse level, other battery features are queried for an exact percentage supplied by the device.

The standard HID adapter reserves listed ROG devices and receivers for detailed ROG support, and checks other devices for battery capabilities in their descriptors. Hardware compatibility still needs validation for each model.

Each backend has its own polling thread and command queue, so a timeout or DPI write does not block polling for other brands. Coarse or missing battery readings remain coarse or unknown instead of becoming 0%. Logitech DPI writes validate advertised values and verify readback; failed writes attempt to restore and verify the original value.

Settings → About → Device compatibility opens a GitHub form and exports factory model, VID/PID, usage, report lengths, and a summary of the last 128 protocol queries in the current session: feature identifiers, durations and results. The report excludes payloads, serial numbers, MAC addresses, interface paths, usernames, custom device names, DPI setting values and raw logs.

Protocol references: [Solaar](https://github.com/pwr-Solaar/Solaar) and [USB HID Usage Tables](https://usb.org/document-library/hid-usage-tables-15). The implementation does not copy driver or open source project code.
