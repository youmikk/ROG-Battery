# Multi-brand protocol research

[简体中文](PROTOCOLS.md) · [Implemented compatibility](COMPATIBILITY.en.md)

Reviewed on 2026-11-17. The [Chinese research record](PROTOCOLS.md) contains the full source and protocol matrix. An upstream catalog entry, a protocol implementation, simulated tests, a community report, and hardware verification are separate evidence levels.

## Reviewed sources

| Project | Pinned revision | Main evidence | License of reviewed sources |
| --- | --- | --- | --- |
| [mouse-protocol](https://github.com/OpenMouse-Project/mouse-protocol/tree/5a4f6433da3d70865d008874c7055016b336c197) | `5a4f643` | Registry; Logitech, VGN, ATK/VXE, Lamzu, WLMouse, Razer, Corsair Bragi and SteelSeries drivers; sensor catalogs; CompX/Lamzu/WLMouse/Razer codecs; Razer device metadata, tests and captured replies in `docs/razer-testing.md` | Root LICENSE: AGPL-3.0 |
| [OpenMouse](https://github.com/OpenMouse-Project/openmouse/tree/28f08ddf61ce0bc97a4eaa780b98bfe31a9f39fc) | `28f08dd` | Client selection, receiver roles and control-interface selection | Root LICENSE: AGPL-3.0 |
| [FerrisPulse](https://github.com/Ferris-echo/FerrisPulse/tree/335fb15ba98be94a85e13061dc0812a04b3aadf6) | `335fb15` | Full source checkout; built-in driver registry, command bus, transport, Logitech/VGN/Rapoo helpers and capability model | GPL-3.0 |
| [MyFinder](https://github.com/wushuai0802/myfinder-device-battery/tree/73d99e2087dd70ae8eef4199854e0c495a66f0bc) | `73d99e2` | Battery observations, identity, caching, connection evidence, protocol providers and normalization | AGPL-3.0 |
| [Solaar](https://github.com/pwr-Solaar/Solaar/tree/e7304c4c451cc9bb4f206a914844525e67856a28) | `e7304c4` | DPI/rate settings, request timeouts, notification routing | GPL-2.0-or-later |
| [libratbag](https://github.com/libratbag/libratbag/tree/8235b5bb6032dea15901ab58a8218956f4494d08) | `8235b5b` | `src/hidpp20.c`, feature discovery and onboard resolutions | MIT for the reviewed file |
| [OpenRazer](https://github.com/openrazer/openrazer/tree/dfe987c8b2053f1c111117ea20163cdb2a07f893) | `dfe987c` | Mouse driver and common DPI, stages, rate and battery commands | GPL-2.0-or-later |
| [Non-Synapse-Mouse](https://github.com/Damnedapa/Non-Synapse-Mouse/tree/267cf595507a9ca1617019885a99ee601b792d06) | `267cf59` | Windows feature reports, transaction IDs, bounded busy retries and DPI control | GPL-3.0-or-later source declaration |
| [orpheus](https://github.com/lindestad/orpheus/tree/693538c8dc885340eff0b4844ed82f37de48b76d) | `693538c` | Transport abstraction and explicit unsupported operations across five protocol families | MIT |
| [Voltide](https://github.com/sdparklab/Voltide/tree/c1872c4c93124d62c05bf5887fe89c32c6be8dd6) | `c1872c4` | DeviceManager, Models, HidNative; Vxe, ATKA9Airadd, Rapoo, Razer, Mchose, AsusHeadset, ATKN9V2Ultimate, MchoseV9Headset, DualSense and XInput providers | MIT, Copyright (c) 2026 Joko |
| [HeadsetControl](https://github.com/Sapd/HeadsetControl/tree/62e361b085d7716805a3bf524bc9cca3761e0e15) | `62e361b` | Nova 5, 7, 7P and 3P Wireless device files and the shared Nova status protocol: model IDs, offsets and battery generations | GPL-3.0 |

The official `Logitech/Logitech-HIDPP` repository returned 404 during this review and is not counted as examined evidence. Protocol implementations are independently written; importing upstream code, generated catalogs or assets requires a separate license review.

## Findings

- OpenMouse has 51 distinct registry brand labels, with multiple protocol generations within several brands. The complete list is in the Chinese record. Registry inspection does not establish full support for each model.
- HID++ `0x2201` / `0x2202` provide current DPI and allowed values. Actual onboard stages belong to `0x8100`. Common shortcuts and sensor ranges must not be presented as saved stages. Extended `0x8061` report-rate limits depend on the connection type.
- VGN, VXE and ATK share parts of the EEPROM16 transport: report `0x08`, read/write commands `0x08/0x07`, stage count `0x02`, active stage `0x04`, older stage records `0x0C`, rate scalar `0x00`. CID/MID and sensor layout still matter. New PAW3955 layouts use different addresses and record sizes.
- Razer's common feature-report generation separates battery `07:80`, charging `07:84`, XY DPI `04:85/05`, and stage list `04:86/06`. Transaction ID, command, status, length and checksum must match; newer and older generations need separate handling.
- Some Lamzu/WLMouse devices share page-command codecs and use sibling interfaces for notifications. Beast X 4K is an explicit protocol exception. Corsair Bragi requires a serialized slot-aware queue; the reviewed driver deliberately remains read-only.
- The reviewed SteelSeries Aerox 3 Wireless driver can read battery but seeds several settings from cached defaults. Non-Synapse-Mouse can infer some profiles from application presets. These are not evidence of reading actual onboard settings.
- Silence does not establish disconnection. Identity, connection evidence and independently timed measurements must be preserved separately. Notifications and command replies must be routed rather than consumed interchangeably.

## Implemented in version 1.4.5

Polling now preserves interface-enumeration evidence, last valid battery observations and Live/Cached/Stale/Unknown quality. Failed polls do not immediately remove known devices; expired numbers are hidden. Cached observations cannot authorize DPI changes or trigger new low-battery notifications.

An enumerated interface that temporarily cannot be opened still counts as present. Standard HID caches now retain the original sample timestamp rather than publishing cached battery values with a new measurement time.

Logitech now prefers the long-report collection within a Windows HID interface group, distinguishes physical receiver instances, and uses different ping and command timeouts. The adapter adds `0x2202` DPI and Y-axis reads plus validated scalar writes, preserving lift-off settings and checking readback and rollback. Different active X/Y values remain read-only in the current scalar interface. Existing `0x2201`, supported `0x8100` formats 1–5 and ROG controls remain in place.

Version 1.4.5 also contains a CompX Report 8 / EEPROM16 mouse adapter. Descriptor, runtime CID/MID and sensor layout select battery, current X/Y, real stage records and profile reads. Stage switching, stage editing and polling-rate changes are separate operations. F58A writes preserve the whole system row or neighboring DPI record, recheck original bytes and verify readback. The exact limits and receiver exceptions are listed in [compatibility notes](COMPATIBILITY.en.md).

An independent polling-rate capability is connected to the performance page. Logitech `0x8060` decodes the millisecond bitmap; `0x8061` uses a link selector when reading and only the rate code when setting. Unknown connections or unavailable modes do not authorize writes. Onboard mode is never changed automatically.

Logitech and CompX operations are serialized per physical control channel within this process. DPI drafts retain their device, profile and original axis values. Original-byte checks and readback detect some external changes but cannot provide atomic access against another application.

Razer operations share a feature-report codec with separate connection capability data. Writes require hashed serial identity, normal mode, configuration checks and readback; transaction IDs are confirmed per command. The common `StageContext` distinguishes real profile IDs from complete configuration fingerprints, allowing stale-edit protection without inventing a Razer profile. Captured one-based records, the active index and zero storage in replies are handled separately from zero-based write records.

The Lamzu / WLMouse page-command family now provides read-only battery, actual profile, complete X/Y stages, active stage and rate for reviewed layouts. Shared receivers query the paired model first. Unknown identities and different layouts stay battery-only; Beast X 4K and bootloaders are excluded. ATK N9 V2 Ultimate and MCHOSE V9 Turbo+ share one FF90 headset battery adapter.

All 148 local regression tests passed on 2026-11-17 and the Windows x64 release build completed without compiler warnings. Coverage includes rates, draft/concurrency checks, CompX echoes, Razer writes/readback, complete page-command configurations and headset replies. New mouse and headset capabilities still await hardware verification; simulated exchanges do not establish hardware compatibility. The overall goal remains incomplete.

## Voltide comparison

Voltide is useful for Windows transport exceptions and captured community replies. DPI, stage and rate semantics still need cross-checking with OpenMouse, Solaar, libratbag and OpenRazer. Its 23 registered providers are not evidence of complete brand support.

- VXE `373B:10C9` and ATK `373B:1278` require control Output Reports. Query echoes can precede real replies. This revision adds bounded, cancellable SET_REPORT transfers and echo filtering; these connections remain read-only. The unrelated 20-byte sibling collection is excluded.
- Rapoo VT3S uses a retained read-only handle and preserves the original timestamp when reusing a sample. This agrees with our existing VT1 fix; our per-interface listener and separate publication/expiry model remain in place.
- DeviceManager shares HID enumeration across providers. We now share descriptor scans for 500 milliseconds while retaining independent backend I/O. Enumeration failure stays an error, and reuse does not renew the scan timestamp. Voltide's approximately 400ms scan comment describes its own environment, not a local benchmark.
- ATK N9 V2 and MCHOSE V9 Turbo+ share FF90/64-byte `55:65` headset commands via WriteFile. One adapter now implements both and passes simulated protocol tests. Upstream hardware/community comments remain separate from local hardware validation, which is pending.
- DualSense needs distinct USB/Bluetooth parsing. XInput provides coarse grades, not exact percentages or a charging flag. Wired mode must not imply 100% charge.

The reviewed Razer percentage expression switches scale at raw 100: 100 becomes 100%, while 101 becomes about 40%. Our implementation uses the consistent 0–255 scale from OpenRazer/Non-Synapse and queries charging separately. It validates transaction ID, class, command, length, status and XOR, and does not permanently reject silent devices. PID-only de-duplication in some upstream providers can merge identical receivers; we retain Windows instance identity. No standalone tests were found in the reviewed Voltide checkout, so hardware comments remain upstream evidence.

Razer exposes battery, charging, current X/Y, complete stage records and separately decoded rate generations. Setters are enabled only by reviewed connection data and successful live checks, not merely by a working query. The stock 1K `00A6` receiver uses extended rate commands while `00B7/00B8` use legacy commands; wireless transport alone does not identify a generation. The `00C1` persistent setting needs selector 1 / transaction `1F` after live readback and identity checks. Persistence across power cycles remains unverified locally.

## Remaining work

Prioritize multi-brand battery coverage and basic DPI controls, retaining detailed ROG support. Advanced settings for other brands must not displace keyboard and headset battery coverage.

- [x] Save 1–4 custom DPI presets per device, sorted by value after editing. A stage-count selector hides the highest values and retains them for restoration; the main window and tray share the enabled list.
- [x] Distinguish actual device stages from software presets; require confirmed numerical write capability, validate supported values and verify readback.
- [x] Explain writes that alter stored DPI. Presets cannot bypass missing write support, stale identity, changed configuration or disconnection.
- [x] Implement Rapoo nRF54L profile-scoped X/Y stage reads and basic switching with protocol regressions. Hardware switching, wake and official-driver coexistence remain pending.
- [ ] Extend existing upstream battery protocols, recording mouse, keyboard, headset and gamepad coverage separately.
- [x] Implement MCHOSE 20-byte keyboard status with Z75S identity checks and Arctis Nova percentage/level status. Protocol and refresh regressions passed; hardware acceptance remains pending.

1. Hardware validation for CompX, Logitech and Razer changes, including writes, wake recovery, device replacement and competing official software.
2. Additional CompX/Beken runtime layouts and transport data; newer Logitech onboard layouts, static feature/identity caching and continuous notification routing.
3. More Razer connection data, including paired-device and transport limits on shared high-rate receivers.
4. Validated page-command writes and sibling-collection notifications; separate treatment for Beast X 4K.
5. Corsair, SteelSeries and further headset/gamepad families, without replacing unreadable settings with defaults.
6. Distributable compatibility data, account-free feedback service and Beta updates. The current feedback path remains a user-submitted GitHub form and sanitized report.

Custom presets are unreleased development changes. Local regressions and Chinese/English render checks passed; per-model hardware acceptance remains pending. CompX presets bind the active stage at click time and verify the active stage and X/Y before and after writing. The current official Rapoo nRF54L driver uses profile-specific addresses and up to seven stages, unlike fixed-address, six-stage reference implementations. The implementation reads the actual profile and complete X/Y tables separately from the older VT9Pro protocol.

## In development: Rapoo nRF54L stages

The independent register implementation follows the current [Rapoo A HUB](https://hub.rapoo.cn/) address map: current profile at 0x010C, profile base 0x0600 + profile × 0x0400, X/Y offsets 648/712, enabled-count offset 662 (zero-based), active-stage offset 664. It preserves all seven slots, independent axes, duplicates, unused data and reserved bytes. Wireless uses A5 and wired uses FF; the older VT9Pro BA protocol is separate.

FF00:000E Output33 and FF00:000F Feature33 collections are paired by Windows instance and USB interface. Output uses write access and Feature uses zero access, both with bounded, cancellable I/O. The independent status listener remains active.

No address or transaction echo is confirmed in the public ACK layout. The reader checks profile before and after, repeats both tables in reverse order and compares active X/Y with telemetry. Writes bind the complete configuration fingerprint and change only one active-index byte. They are not resent or blindly rolled back; full readback and a newer status report must agree. Configuration failure retains independent battery data. Configuration reads have a two-second budget. Numerical DPI edits and Rapoo software-preset writes remain disabled.

Hardware validation is pending. Simulated transcripts do not establish race-free coexistence with other drivers or compatibility across every firmware. The official scripts are protocol references and are not distributed.

## In development: keyboard and headset battery families

The keyboard implementation combines Voltide's M HUB captures and Windows SET_REPORT transport with FerrisPulse's Z75S identity handshake. FA09 uses the captured control endpoint; Z75S uses the documented interrupt OUT path, without automatically retrying through another endpoint. A battery reply must start with `13 4A 01 00 02`, have a checksum equal to the sum of the first 19 bytes, and contain a percentage at byte 5. The high nibble of byte 6 is the charging state. Zero-payload query echoes are not measurements, and a shared product ID cannot establish a particular keyboard model.

Nova model IDs and status offsets are cross-checked against HeadsetControl. FerrisPulse provides Windows 65-byte transport evidence and Nova 5 sample vectors. After removing Windows report ID 0, the B0 Nova 5 / 3 Wireless layout stores connection, percentage and charging at offsets 1, 3 and 4. Nova 7 uses offsets 2 and 3 for battery and state. The PID selects discrete levels versus direct percentages; values at or below 4 cannot select the protocol. Original 0–4 levels are retained, with 0/25/50/75/100 used only as display estimates prefixed with ≈, separate from exact measurements. Unknown states are rejected or left unknown rather than treated as a valid connection.

These adapters implement read-only battery status with per-interface serialization, measurement timestamps, cache expiry and sanitized reports. No upstream implementation code or assets are distributed. Exact scope and outstanding hardware checks are in the compatibility notes.

Further gamepad review found that Voltide maps XInput wired connections to 100% charging and converts four levels into unmarked percentages. Wired mode does not establish full charge or charging; any level-derived percentage must be marked ≈ and kept separate from exact measurements. DualSense's ten-step scale and Bluetooth report integrity need separate verification before integration.
