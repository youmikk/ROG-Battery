# Multi-brand protocol research

[简体中文](PROTOCOLS.md) · [Implemented compatibility](COMPATIBILITY.en.md)

Reviewed on 2026-11-21. The [Chinese research record](PROTOCOLS.md) contains the full source and protocol matrix. An upstream catalog entry, a protocol implementation, simulated tests, a community report, and hardware verification are separate evidence levels.

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
| [Linux hid-playstation](https://github.com/torvalds/linux/blob/4dd10fbbeb9c33709edc89916e19c03c780a0751/drivers/hid/hid-playstation.c) | `4dd10fb` | DualSense USB / Bluetooth input layouts, battery status, CRC seeds and calibration Feature Reports | GPL-2.0-or-later for the reviewed file |
| [sony-h5-tray](https://github.com/splitbrain/sony-h5-tray/tree/1d282db4ec641bfdefab1df14c3e6d05eb767706) | `1d282db` | H5 HCI frames, checksums and published captures in `src/proto.rs` / `src/hid.rs`, cross-checked with HeadsetControl's INZONE protocol and H5 / H9 II interface tables | MIT, Copyright (c) 2026 Andreas Gohr |
| [cherry-battery-state](https://github.com/zcmk123/cherry-battery-state/tree/af82d4d98cbad872a586302ba44e6a6b75ba65f4) | `af82d4d` | Fixed query and fields in `cherry_battery.py`, cross-checked with MyFinder's `cherry.py` and [issue #482](https://github.com/wushuai0802/myfinder-device-battery/issues/482) for Pro identities and descriptors | README declares MIT; no separate LICENSE in the reviewed tree. No code or assets imported |

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
- DualSense USB / Bluetooth layouts have been cross-checked against Linux. XInput is also integrated through a separate system endpoint after checking Microsoft's API documentation; it provides coarse grades, not exact percentages or a charging flag. Wired mode must not imply 100% charge.

The reviewed Razer percentage expression switches scale at raw 100: 100 becomes 100%, while 101 becomes about 40%. Our implementation uses the consistent 0–255 scale from OpenRazer/Non-Synapse and queries charging separately. It validates transaction ID, class, command, length, status and XOR, and does not permanently reject silent devices. PID-only de-duplication in some upstream providers can merge identical receivers; we retain Windows instance identity. No standalone tests were found in the reviewed Voltide checkout, so hardware comments remain upstream evidence.

Razer exposes battery, charging, current X/Y, complete stage records and separately decoded rate generations. Setters are enabled only by reviewed connection data and successful live checks, not merely by a working query. The stock 1K `00A6` receiver uses extended rate commands while `00B7/00B8` use legacy commands; wireless transport alone does not identify a generation. The `00C1` persistent setting needs selector 1 / transaction `1F` after live readback and identity checks. Persistence across power cycles remains unverified locally.

## Remaining work

Prioritize commonly used Chinese brands and basic DPI, retaining detailed ROG support. First improve Rapoo, VGN / VXE / ATK and MCHOSE model coverage, keyboards and sleep recovery, followed by AULA, AJAZZ, Dareu and Edifier with established protocol evidence. Pause new overseas-brand coverage while maintaining shipped support. Common-device battery coverage and stability take priority over advanced settings.

- [x] Save 1–4 custom DPI presets per device, sorted by value after editing. A stage-count selector hides the highest values and retains them for restoration; the main window and tray share the enabled list.
- [x] Distinguish actual device stages from software presets; require confirmed numerical write capability, validate supported values and verify readback.
- [x] Explain writes that alter stored DPI. Presets cannot bypass missing write support, stale identity, changed configuration or disconnection.
- [x] Implement Rapoo nRF54L profile-scoped X/Y stage reads and basic switching with protocol regressions. Hardware switching, wake and official-driver coexistence remain pending.
- [x] Add Dareu Jm battery and PAW3395 stage switching with runtime pairing/configuration checks; hardware acceptance remains pending.
- [x] Add MCHOSE Feature mouse battery/stage reads and full-configuration-preserving stage switching for A7 V2 Ultra+ / L7 Pro+; hardware acceptance remains pending.
- [ ] Extend existing upstream battery protocols, recording mouse, keyboard, headset and gamepad coverage separately.
- [x] Implement MCHOSE 20-byte keyboard status with Z75S identity checks and Arctis Nova percentage/level status. Protocol and refresh regressions passed; hardware acceptance remains pending.
- [x] Add selected shared VGN / VXE `320F:5088` keyboard battery queries with independent slow-response, sleep and background-recovery handling. Hardware acceptance remains pending.
- [x] Add MCHOSE G98 V3 QHW battery, BY model identification and notifications, with separate firmware-specific reply validation. Hardware acceptance remains pending.
- [x] Add Edifier Halo Space wireless battery with restricted EC collection/report matching and meaningful-payload validation. Hardware acceptance remains pending.

1. Improve integrated Rapoo, VGN / VXE / ATK and MCHOSE support first: battery, basic DPI, sleep recovery, replacement and vendor-software coexistence. Record missing hardware acceptance explicitly.
2. Extend runtime mouse identities and transports for these brands, separate MCHOSE keyboard protocols and receiver collection pairing. Keyboard queries must not inherit mouse setting commands.
3. Add evidence-backed AULA, AJAZZ and Dareu keyboard/mouse protocols and Edifier headsets. Prioritize common battery/basic controls without filling unknown settings from catalogs.
4. Expand Flydigi, GameSir and 8BitDo gamepad battery support, distinguishing percentages from levels and checking transport changes and system-API fallbacks.
5. Maintain shipped overseas-brand features; additional models, complex onboard layouts and advanced settings follow common domestic devices.
6. Distributable compatibility data, account-free feedback service and Beta updates. The current feedback path remains a user-submitted GitHub form and sanitized report.

Custom presets are included in version 1.4.6. Local regressions and Chinese/English render checks passed; per-model hardware acceptance remains pending. CompX presets bind the active stage at click time and verify the active stage and X/Y before and after writing. The current official Rapoo nRF54L driver uses profile-specific addresses and up to seven stages, unlike fixed-address, six-stage reference implementations. The implementation reads the actual profile and complete X/Y tables separately from the older VT9Pro protocol.

## Version 1.4.6: Rapoo nRF54L stages

The independent register implementation follows the current [Rapoo A HUB](https://hub.rapoo.cn/) address map: current profile at 0x010C, profile base 0x0600 + profile × 0x0400, X/Y offsets 648/712, enabled-count offset 662 (zero-based), active-stage offset 664. It preserves all seven slots, independent axes, duplicates, unused data and reserved bytes. Wireless uses A5 and wired uses FF; the older VT9Pro BA protocol is separate.

FF00:000E Output33 and FF00:000F Feature33 collections are paired by Windows instance and USB interface. Output uses write access and Feature uses zero access, both with bounded, cancellable I/O. The independent status listener remains active.

No address or transaction echo is confirmed in the public ACK layout. The reader checks profile before and after, repeats both tables in reverse order and compares active X/Y with telemetry. Writes bind the complete configuration fingerprint and change only one active-index byte. They are not resent or blindly rolled back; full readback and a newer status report must agree. Configuration failure retains independent battery data. Configuration reads have a two-second budget. Numerical DPI edits and Rapoo software-preset writes remain disabled.

Hardware validation is pending. Simulated transcripts do not establish race-free coexistence with other drivers or compatibility across every firmware. The official scripts are protocol references and are not distributed.

## Version 1.4.6: keyboard and headset battery families

The keyboard implementation combines Voltide's M HUB captures and Windows SET_REPORT transport with FerrisPulse's Z75S identity handshake. FA09 uses the captured control endpoint; Z75S uses the documented interrupt OUT path, without automatically retrying through another endpoint. A battery reply must start with `13 4A 01 00 02`, have a checksum equal to the sum of the first 19 bytes, and contain a percentage at byte 5. The high nibble of byte 6 is the charging state. Zero-payload query echoes are not measurements, and a shared product ID cannot establish a particular keyboard model.

Nova model IDs and status offsets are cross-checked against HeadsetControl. FerrisPulse provides Windows 65-byte transport evidence and Nova 5 sample vectors. After removing Windows report ID 0, the B0 Nova 5 / 3 Wireless layout stores connection, percentage and charging at offsets 1, 3 and 4. Nova 7 uses offsets 2 and 3 for battery and state. The PID selects discrete levels versus direct percentages; values at or below 4 cannot select the protocol. Original 0–4 levels are retained, with 0/25/50/75/100 used only as display estimates prefixed with ≈, separate from exact measurements. Unknown states are rejected or left unknown rather than treated as a valid connection.

These adapters implement read-only battery status with per-interface serialization, measurement timestamps, cache expiry and sanitized reports. No upstream implementation code or assets are distributed. Exact scope and outstanding hardware checks are in the compatibility notes.

Further gamepad review found that Voltide maps XInput wired connections to 100% charging and converts four levels into unmarked percentages. Wired mode does not establish full charge or charging; any level-derived percentage must be marked ≈ and kept separate from exact measurements.

## Version 1.4.7: DualSense battery

Voltide's Windows transport was cross-checked with Linux `hid-playstation.c`. USB input is Report `01`, exactly 64 bytes, with battery status at offset 53. Bluetooth extended input is Report `31`, exactly 78 bytes, with status at offset 54 and a little-endian CRC32 in the last four bytes. CRC includes prefix `A1` followed by the first 74 bytes, with all-one initialization and final inversion. Bluetooth minimal Report `01` has no battery field, even when Windows pads it to the maximum length.

The low nibble's bands 0–9 use representative values 5–95, and 10 means 100. The upper nibble is 0 discharging, 1 charging or 2 full. A/B/F faults remain unknown instead of borrowing Linux's zero capacity as a measurement. Band values use the estimate field and display ≈; malformed reports never replace or renew an existing sample.

Input handles request read access only. A 41-byte calibration Feature `05` GET is issued only after observing Bluetooth minimal mode; its CRC uses the separate prefix `A3`. Subsequent input is validated independently and supplies the battery data. There are no Output / SetFeature, lighting, rumble or control-setting operations. Each receive loop has both a deadline and a report-count bound.

The polling wait is capped at two seconds and samples expire after eight seconds. Silence retains the original timestamp until expiry, while fresh reports restore the numeric display. Windows interface instances distinguish identical controllers; USB / Bluetooth cross-transport deduplication is not implemented. No input payloads are logged. Neither controller is present locally; hardware transport, sleep recovery and Steam / DS4Windows coexistence remain pending.

## Version 1.4.7: XInput system battery

Microsoft's [XInputGetBatteryInformation](https://learn.microsoft.com/en-us/windows/win32/api/xinput/nf-xinput-xinputgetbatteryinformation) and [XINPUT_BATTERY_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/xinput/ns-xinput-xinput_battery_information) documentation were checked against Voltide. Battery types 0/1/2/3/FF mean disconnected/wired/alkaline/NiMH/unknown. Levels 0–3 mean empty/low/medium/full and are valid only for known wireless battery types. No percentage boundaries or charging flag are specified.

The adapter loads XInput 1.4 only from System32 on supported Windows 10/11 installations. It neither installs a library nor loads game wrappers beside the executable. State reads establish connection and discard button/axis data; only gamepad battery is queried, without headset-slot or vibration commands. Each of four slots is sampled separately. Other API errors preserve uncertain presence instead of inventing a disconnect.

Endpoints now distinguish HID interfaces from XInput slots, preserving existing HID keys and protocol logic. XInput presence comes from API results rather than HID enumeration. Explicit no-battery evidence clears prior charge for wired and unknown types; a transient failure does not renew sample age. Display representatives 0/20/50/100 always carry ≈ and never enter exact-percentage notifications.

XInput exposes neither model names nor persistent physical IDs. The adapter does not guess a slot's VID/PID or associate virtual controllers with HID devices. Preferences follow system slots; quick replacement and cross-protocol duplicate limitations are documented. All 192 local regressions and the release build passed; the system API loaded, with no connected controller available for battery acceptance.

## Version 1.4.7: Sony INZONE battery

HeadsetControl's `sony_inzone_protocol.hpp`, H5 / H9 II definitions and interface tests were cross-checked against sony-h5-tray's published H5 frames. H5 uses `FF04:0002`; H9 II uses `FF04:0001` / `MI_05`. FerrisPulse's H5 path falls back from usage 1 to any 64-byte output collection. This adapter does not use that broad fallback or access `FF13` update interfaces.

Report `02` contains Sony HCI in a 64-byte transport frame. Battery GET uses event `04`, type `01`, address `41`, and the sum of bytes 6–12. A battery reply has HCI length 14, source/destination `14`, event `04`, return type `10` or same-transaction `A0`, and the sum of bytes 5–14. Query IDs skip 0 and unsolicited-notification ID 1. The parameter-length field is informational upstream; the fixed outer length, source, transaction and checksum must all match.

Payload bytes are charger state and percentage. Values 0–100 retain reported percentages; `FF` is offline / no cached value; 101–254 are rejected. Only charger values 0/1 establish false/true. Other nonzero values and full charge do not imply charging. Timeouts do not renew sample age, explicit offline clears charge, and fresh replies restore it. Buds have multiple battery components and are deliberately excluded from this two-byte layout.

The independent implementation sends only status queries, using existing per-interface serialization and lifecycle rules. Protocol and state regressions pass; hardware acceptance is pending. Published frames and synthetic negative tests are not local hardware captures.

## Version 1.4.7: HyperX Cloud Alpha 2 battery

HeadsetControl `62e361b`'s `hyperx_cloud_alpha_2_wireless.hpp` and tests were checked against MyFinder `73d99e2`'s `hyperx_alpha2.py` and tests. Their published frames agree on station `03F0:08BE`, collection `FF13:FF00`, 64-byte reports and `50 02` / `51 02`. Neither the `0ABE` audio function nor invariant `61 02` audio settings supplies battery.

MyFinder treats bit 7 of byte 6 as charging. HeadsetControl's separately captured charging sample identifies byte 3 as charger state and bytes 6/7 as little-endian voltage. The 51% / 67% captures carry `00 0F` / `86 0F`, meaning 3840 / 3974 mV; they cannot establish charging. The 91% charging capture explicitly sets byte 3 to 1. These published packet facts are regression inputs; unknown charging values remain unknown.

Connection `52 01` returns `53 01` plus 0/1. ACK `FF 01` and NACK `FF 02` echo the command at offsets 14/15. Only a matching NACK terminates the query; ACKs, `FB` notifications and other commands are filtered. Connection is queried before and after battery to avoid publishing station-cached charge during power-off. Without transaction IDs or checksums, same-command traffic cannot be attributed reliably; NGENUITY coexistence requires hardware validation.

NuPhy stock-firmware coverage is not expanded yet. FerrisPulse's `55` / `AA` path fills missing modes and settings from defaults or caches. Slone123c/nuphy-air75-v2-web-panel revision `02c54f3094afc655a84271159053a19abd83f1fb` explicitly describes custom firmware in its README and cannot establish the same battery commands on stock Air V2 firmware. Model, descriptor and valid-response evidence remains necessary.

## Version 1.4.7: CHERRY MX 2.0S WL Pro

The original `af82d4d` implementation supplies fixed-command and field evidence. Pro USB / receiver identities, `MI_01` / `FF1C:0092` and bidirectional Report `04` come from MyFinder `73d99e2` and issue #482. The catalog is not widened to the full Cherry VID; original `01AC` and Bluetooth `0687:01DE` do not inherit Pro queries.

Descriptor handles request access=0 and inspect both button and value caps. Failure in either direction, mixed report IDs, wrong lengths or ambiguous candidate collections prevents a query. Uniqueness uses Windows USB interface groups, without full cross-interface physical-parent resolution; identical device instances retain separate identities.

Replies require 64 bytes, the fixed prefix, status 0, percentage 1–100 and charger 0/1. Zero and all-zero request echoes cannot be reliably distinguished and never renew a sample. Contributor evidence includes a temporary 3–4 percentage-point difference from the vendor UI after switching connections; the adapter does not alter reported charge to match a screenshot. Long-term hardware stability remains unverified.

On 2026-11-21, all 211 local regressions passed and the Windows x64 build completed without warnings. The added caps API loaded normally, and existing ROG mouse and VGN keyboard probes returned valid data. No Cherry hardware was available to verify low battery, stopped charging, long sleep or vendor-software coexistence.

## In development: Lofree HYZEN

The official [Lofree HUB](https://hyzen.lofree.tech/) `assets/index-D0x9DTYM.js` was directly inspected on 2026-11-21; SHA256 `078b76071f7f4cdf7851611c3a2cd5fba90b6da18502b3a844845404a059dafa`. No open-source license was found for this script. Only protocol facts were independently implemented; vendor scripts, assets and obfuscated code are not distributed.

The device list has `388D:0024` wired, `0025` wireless and `0029` single-mode, collection `FF1C:0092`. Battery calls pass `AA` or `1A`, empty data and offset 0 to `Dx`, using Report `04`. Wireless payloads are 31 bytes, or 32 with the Windows report ID. START / END use payload offset 2 values 1/2; data uses command at offset 2, address at 4/5 and data from offset 7. MyFinder `73d99e2`'s `lofree.py` was cross-checked. Its test comment references issue #82, but that issue currently describes another device and is not counted as Lofree hardware evidence.

Only online and battery commands are allowed. Each phase is bounded, requires nonempty data with zero offset, attempts END after failures and requires END acknowledgement before publication. Online status is rechecked; charging remains unknown. The vendor UI's initial wired value of 100 is not reused as a measurement. Wired suppression currently uses model presence without proven pairing identity, potentially affecting another identical keyboard; this limit is documented.

All 218 local regressions and the release build passed. No matching Lofree hardware was available to verify Windows collections, response sizes and sleep recovery. The 1.4.7 tag remains unchanged; this work is on the development branch.

## In development: shared VGN / VXE 320F:5088 keyboards

The saved official [VGN HUB](https://drive.vgnlab.com.cn/) bundle `assets/index-BfN6tzNB.js`, SHA256 `2dc306331d30f874eec8a006f975ef5cac3ed2f18d3bef58e5dd089dc8d5cd5f`, was inspected directly. The `weisheng` driver sends Report `04` with payload `00 00 1A 06 00 00 00`, and reads payload byte 7 when byte 2 is `1A`. MyFinder `73d99e2` supplies independent protocol and published-prefix evidence. Vendor code and assets are not distributed.

Queries require `320F:5088 / FF1C:0092`, 64-byte input/output with Report `04` only, and one management collection per USB interface group. This protocol does not use Cherry's `04 20` prefix. A published response took approximately 1.03 seconds; the new backend allows 2.5 seconds and runs independently of V98 Pro V3. Sleep `FF`, echoes and invalid reports cannot renew the sample. Values expire after 120 seconds and the maximum background wait before another query is ten seconds.

Shared IDs/product strings cannot exclude Rainy75 or establish a VGN model. Product names and instance identities are retained; charging, cross-process attribution without transaction IDs and full sleep cycles need hardware verification. All 223 regressions and the release build passed, including shared UI/tray expiry and background recovery; upstream observations are not local hardware acceptance.

## In development: MCHOSE QHW and BY keyboards

The current [M HUB](https://www.mchose.com.cn/) entry was checked on 2026-11-21. Application `assets/main-DIt5vAcV.js` has SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`; the official manifest points to `cizhou/CZ_SHARED_DATA/main.002bbdb0f17b5f7135ee.js`, SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3`. No vendor implementation license was found. Files were inspected statically for protocol facts, without executing or distributing their code/assets, and cross-checked with MyFinder `73d99e2`.

The QHW catalog identifies G98 V3 wireless as `3837:1015 / MCHOSE G98 V3 2.4G / FF80:0001`. `getBattery` calls `_simpleGet(2308, [])`: Report `4D`, little-endian command `0904`, payload `01 03 00 04 09 00 ss cc`. Sequence increments modulo 256 and output checksum XOR excludes version, giving `0E XOR ss`. Reply version, flags, command, bizCode and sequence must match, with data length exactly two. Payload fields are charger enum and percentage; enum 2 is full, not actively charging. The official parser extracts the reply checkCode without proving its algorithm, so no incoming XOR rule is invented.

Input and output capacities are proven independently from Windows descriptors, with minimum lengths 11/9 including the report ID. Writes use the actual output capacity and replies must match actual input length. `NORMAL_MAX_PACKET_LENGTH=31` is a code limit, not a hardware descriptor measurement. Only battery GET is sent. Independent cold-start reads and stale sleep responses remain hardware questions.

BY `keyboardPwds` contains six-byte firmware model identifiers, not login credentials. Report `13` / command `05` reads the code at full-frame byte 5. The decoder never shifts bytes to force an expected model. Official `handleCommandResponse` checks packet count/index and discards byte 4 and the last byte; FA09 and new `41E4:2001` therefore do not inherit an unproven length-metadata or reply-checksum rule. Actual Report ID, 20-byte length, command, single-packet framing and echo rejection remain required. Z75S and separate VGN V98 Pro V3 retain their existing strict validation.

Recognized models also accept `0A`, type `05` battery notifications during the bounded battery-query window. M HUB's `batteryParser(false)` skips bytes 2–4 and takes percentage/flags at 6/7. Both flag nibbles must be 0/1; high nibble 1 means charging only below 100%, and full flags never replace the percentage. Unknown FA09 models accept only valid `4A` percentages, with unknown charging. Other named brands do not receive an extra MCHOSE identity query. Wireless BY zero is suppressed because it may be a sleep placeholder.

FA09 retains captured SET_REPORT transport; Z75S and `41E4:2001` use interrupt OUT without fallback resends. The latter is restricted to exact generic product names, `FF04:0002` and recognized model codes. Cross-USB-interface FF02 sibling pairing is not implemented. All entries require bidirectional Report `13` descriptor evidence and an unambiguous interface-group control collection; feature-only AULA/FA09 paths receive no command.

All 236 local regressions and a warning-free release build passed. Coverage includes QHW sequence wrap, invalid replies and distinct capacities; BY unknown identity, notifications, sleep placeholders and background recovery. MyFinder #476's two complete frames are retained verbatim: the identity does not prove G75 Pro, while battery gives a neutral 100% with unknown charging. #312 provides a prefix with synthetic trailing test padding. Existing ROG read 79% / DPI 800 and VGN read 100% / charging in a read-only probe; no newly supported keyboard was present for hardware acceptance.

## In development: Edifier Halo Space

Battery command `D0` is independently implemented from FerrisPulse `335fb15`'s [public protocol record](https://github.com/Ferris-echo/FerrisPulse/blob/335fb15ba98be94a85e13061dc0812a04b3aadf6/docs/EDIFIER_HALO_SPACE_PROTOCOL.md), helper and packet tests. It specifies `2D99:E049 / FF14:0001`, 64-byte input/output, input prefix `2F BB EC` and output `2E AA EC`. `FF13` is a distinct collection. Upstream's broad collection fallback and permissive alternate headers are not used.

The fixed query is `2E AA EC D0 00 00 66` with zero padding. Its first byte already occupies the Windows report-ID slot; no extra `00` is prepended. Reply big-endian payload length must be one. Sum wire bytes 1–6 against byte 7 before unmasking byte 6 with XOR `A5`. Published prefix `2F BB EC D0 00 01 86 FE` yields 35%. Trailing historical scratch data never supplies battery or charging information.

Descriptors must additionally show only input `2F` and output `2E`; this conservative entry condition needs real caps acceptance. No C8 MAC, C9 name, C6 firmware or unsupported F2 status query is issued. Charging stays unknown rather than borrowing the upstream UI's false as evidence of discharge. Cross-process reply attribution, charging-cable disconnection and sleep-cache behavior remain hardware questions.

All 241 regressions and a warning-free build passed, covering valid zero/full, bad checksums, wrong endpoints, lengths, stale tails, identical units and shared main/tray expiry and recovery. Upstream hardware claims are protocol-source evidence only; no matching headset was available locally.

## In development: Dareu Jm mice

Independently implemented from the [official Dareu WebHID driver](https://driver.dareu.com/allinone/) 1.3.26, its pairing map and product JSONs, cross-checked against OpenMouse `5a4f643`'s A950 PRO Mg record and Voltide `c1872c4`'s `DareuA950MouseProvider`. Official `jmdevice.js` SHA256 is `997fb0cb6331a68e730db1d59499cad11aaf0a976db70383f33aa476c1370427`; `global.js` is `cdb0d2db7f36bd53e87f906bc86749e498d45739dc6f815d6c033c441842accf`. Vendor resources were inspected statically and are not distributed.

Entry requires `260D:1114 / 1117 / 1175`, `FF02:0002`, bidirectional Report `08`, 17-byte full reports and a unique management collection in its USB interface group. The official current `ParseDevices` identifies Jm through FF02:2 Report 08. This implementation proves that collection directly; it does not borrow FF05 service collections from another receiver or guess physical associations across USB interfaces. Receiver IDs 1114 and 1175 do not prove a specific paired model.

| Interface PID | Runtime CID / MID | Official model | Added capability |
| --- | --- | --- | --- |
| 1114 | 10 / 1, 10 / 3, 10 / 4, 10 / 39 | A950 PRO, A950 PRO Mg, two A950G catalog entries | Battery and actual PAW3395 DPI-stage reads/switching |
| 1117 | 10 / 3 | A950 PRO Mg wired | Same, with unverified wired transport |
| 1175 | 10 / 9, 4 / 9, 10 / 25 | Two A950 AIR entries, A950 GM | Battery and actual PAW3395 DPI-stage reads/switching |
| 1175 | 10 / 37 | LM180T | Battery/name only; PAW3950 settings are not accessed |
| Listed receivers | Unknown or missing identity | Neutral name | Valid battery only, without settings writes |

Command 03 establishes active status. Command 01 returns CID/MID/type at full-frame offsets 10/11/12. Its four random request bytes follow the vendor flow, but no challenge echo is established and they are not treated as a transaction ID. Battery command 04 requires byte 5 equal to two and percentage at byte 6. That conservative length requirement comes from Voltide's driver evidence; other 1114 firmware still needs verification. Charge enums 0/1 mean not charging/charging. Enum 2 remains unknown because the current official Jm energy pages display it as charging while Voltide describes full. Voltage and an unproven incoming checksum are not inferred.

Read the active one of four profiles without changing it. Memory addresses 2/4 contain stage count and zero-based active index with separate checksums. Read the actual 1–5 stages at `12 + 4 × index`; catalog defaults never fill missing settings. PAW3395 high bytes `00/44/88` encode successive 256-value groups at 50 DPI per step, constrained to the paired model's 100–26000, 100–30000 or 50–26000 range. Preserve raw indexes, duplicate values and X/Y values; each record must sum to `55`.

Reverse rereads verify the entire table and identity/profile boundaries. A configuration fingerprint includes identity, receiver type, profile, count, active stage and every record. A click rereads and compares this state before sending command 07 exactly once, changing only the two-byte active-stage pair at address 4. No count, DPI-table, button, macro, polling or profile edits are performed. Lost ACKs are not retried; a full readback must confirm the requested stage and unchanged surrounding records. Without a transaction ID or cross-process atomic lock, an external driver can still race the final check. Replacing a paired unit with the same model also lacks unique identity evidence.

All 252 regressions and a warning-free release build passed. New tests use synthetic reports constructed from documented fields, covering shared receivers, unsupported sensors, malformed lengths/addresses, sleep recovery, concurrent configuration changes, ignored writes, lost ACKs and the main/tray command queue. OpenMouse's 1114 hardware report is source evidence only; no Dareu device was available locally, and all new entries await hardware acceptance.

## In development: MCHOSE Feature mice

Cross-checked the current M HUB `main-DIt5vAcV.js` (hash above), including device/config parsers, write schemas, Feature transport and product catalog, against OpenMouse `5a4f643`'s [MCHOSE protocol record](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/mchose-protocol.md) and published L7 Pro+ captures. No vendor or upstream implementation is distributed.

Entry is restricted to `3837:100B/1020/4018/4023/4019/4021/4015` or `5253:1020/1011`, `FF01:0001`, and a 65-byte Feature capacity. Descriptor caps must contain Feature 11/12; Feature 14 is allowed, other IDs are rejected. Firmware collection FF0B, MagDock, QHW keyboards, A7 V3 Report 4D, old A5 and Bluetooth are excluded. Ambiguous management collections are rejected and identical serial strings do not merge receiver instances.

Report 11 commands 03/06 invert the command and 19 zero argument bytes, leaving the remaining wire padding zero. Report 12 command 67 reads configuration. Each query sends once, then accepts two consecutive equal valid payloads within 1.9 seconds/20 reads at 90 ms intervals. Gaps, foreign commands and empty echoes reset that proof. Short replies may be the captured 21 bytes or Windows' 65-byte capacity; only their defined 7/11 bytes count. Config replies must contain 65 bytes, with all 63 configuration bytes stable.

Command 06's VID/PID identifies the mouse, not the shared receiver. The vendor parser's little-endian bit fields start at bit zero: mode is flags bits 0–2 and connection is bit 3. OpenMouse's high-bit interpretation is not reused. Published L7 Pro+ cable data decodes to `3837:4015`, 57%, charging, flags 00; receiver data gives the same mouse, 100%, not charging, flags 09. Wireless reads also check receiver VID/PID, pairing and online status with command 03 before/after. Charge enums other than 0/1 stay unknown. Wireless zero does not renew a sample because the vendor uses it as a sleep placeholder; valid wired zero remains zero. Explicit offline clears old battery, silence does not renew it, samples expire after 120 seconds, and the maximum background wait is ten seconds.

Runtime models include A7 V2 Pro `4018`, Pro+ `4023`, Ultra `4019`, Ultra+ `4021`, L7 Pro+ `4015`, L7 Pro `00B0` and A7 Ultra `1011`. Unknown models may retain valid battery but receive no configuration access. Some old `5253:1020` A7 Ultra receivers are documented to return empty 06 replies; product strings do not fabricate battery or establish A7 Ultra identity.

Read only the active one of three profiles. Config bytes 1/2 contain wired/wireless stage indexes in their low nibbles; byte 16 is the enabled count. Preserve raw indexes, duplicates and 50-DPI steps; six populated slots do not prove six enabled stages. Only the verified X values are displayed, without assuming Y equality or its enable mode. A7 V2 Ultra+ and L7 Pro+ have upstream whole-config write evidence and receive stage switching; other models remain read-only.

The fingerprint includes mouse VID/PID, firmware, transport and all 63 configuration bytes. A click reconfirms link, identity and config twice. Command 57 changes only the live link's stage nibble; all other 62 bytes and the polling-rate nibble are retained. After 400 ms, full readback must match every expected byte and the same connected identity. A send timeout is never retried or blindly rolled back. This switches existing stages only; arbitrary DPI/custom-preset writes are not offered. External-driver races after the last check and replacement by an identical model remain unresolved protocol limits.

All 268 regressions and a warning-free build passed. Battery/config decoding uses published L7 Pro+ raw frames; other tests use synthetic reports for interleaved commands, partial replies, changing tails, unknown models, sleep/offline, preserved configuration, concurrent edits, lost send completion, ignored writes and corrupted neighbors. Wireless-zero placeholders cannot enable settings. Main and tray controls share the configuration token and backend queue. No matching MCHOSE mouse was available locally; Windows caps, stale receiver caches, complete sleep cycles and stage changes await hardware acceptance.
