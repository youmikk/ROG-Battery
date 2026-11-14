# Device compatibility

[简体中文](COMPATIBILITY.md)

This table separates hardware verified support in the stable app from protocol implementations on the multi-brand development branch. Discovery, model recognition and parser tests do not verify hardware compatibility.

| Brand / protocol | Model / scope | Battery | DPI | Status |
| --- | --- | --- | --- | --- |
| ROG | HARPE ACE MINI, USB / OMNI | Verified | Verified; existing detailed settings retained | Stable release |
| Logitech HID++ 2.0 | Devices exposing 0x1000 or 0x1004 | Protocol implemented | Read and verified writes when 0x2201 is available | Development branch; hardware validation pending |
| Standard HID | Generic Device Controls / Battery Strength (06:20) in the descriptor | Read using descriptor limits | No standard DPI write operation | Development branch; hardware validation pending |
| Rapoo | VT1 | Investigation pending | Investigation pending | Unverified |

Voltage and coarse HID++ battery levels are not shown as exact percentages. Charging remains unknown when standard HID does not expose it.

Settings → About → Device compatibility opens a GitHub form and exports a report containing factory model, VID/PID, usage and report lengths. It excludes serial numbers, MAC addresses, interface paths, usernames, custom device names and raw logs.

Protocol references: [Solaar](https://github.com/pwr-Solaar/Solaar) and [USB HID Usage Tables](https://usb.org/document-library/hid-usage-tables-15). The implementation does not copy driver or open source project code.
