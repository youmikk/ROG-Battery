# Multi-brand protocol research

[简体中文](PROTOCOLS.md) · [Implemented compatibility](COMPATIBILITY.en.md)

## 2026-11-26: Corsair Bragi receiver integration deferred

Rechecked the [IRONCLAW capture](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/captures/corsair-ironclaw-rgb-wireless/README.md) and [SCIMITAR capture](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/captures/corsair-scimitar-rgb-elite/README.md) at OpenMouse `5a4f6433da3d70865d008874c7055016b336c197`. The Windows capture for `1B1C:1BDC` contains DPI writes and acknowledgements while iCUE was running, without GET replies, HID report descriptors, or hardware-mode battery evidence. That source has no traffic from receiver `1B66`. Catalog entries do not establish a verified battery channel.

The descriptor-proven `1BE3` is a wired SCIMITAR: `FF42:0001`, unnumbered 128-byte protocol reports, requiring a 129-byte Windows buffer including the zero ID slot. Its battery property returns unsupported. Neither this interface nor its input-only notification collection establishes wireless-receiver compatibility.

[ckb-next `e22eaf2fba7f93764b4c6d4413bb83eeca75d10a`](https://github.com/ckb-next/ckb-next/blob/e22eaf2fba7f93764b4c6d4413bb83eeca75d10a/src/daemon/device_bragi.c) actively switches receivers into software mode during initialization. It does not establish battery access without changing mode. Bragi GET replies echo neither property nor transaction ID; local serialization cannot fully distinguish another application's GET on the same slot. No backend or probe commands are added. Receiver descriptors and battery, charging, offline, slot, and multiple-receiver captures without mode changes are still required. The inspected sources are AGPL-3.0 and GPL-2.0 respectively; no implementation code is imported.

## 2026-11-26: Razer receiver battery interfaces (unreleased)

Pinned OpenRazer `ea55c1b5bda0b4112fcef91dd38395675151af33`: [razermouse_driver.h](https://github.com/openrazer/openrazer/blob/ea55c1b5bda0b4112fcef91dd38395675151af33/driver/razermouse_driver.h), [razermouse_driver.c](https://github.com/openrazer/openrazer/blob/ea55c1b5bda0b4112fcef91dd38395675151af33/driver/razermouse_driver.c), and [razerchromacommon.c](https://github.com/openrazer/openrazer/blob/ea55c1b5bda0b4112fcef91dd38395675151af33/driver/razerchromacommon.c), GPL-2.0-or-later. Model and packet facts extend the existing local Feature transport and codec; upstream implementation code is not bundled.

Receivers `00B4/00B9/00C5/00CD` use transaction `1F`, battery command `07:80`, two data bytes and a 0–255 scale. `00C5/00CD` separately query `07:84`, accepting charging 0/1 only. The other two omit that query and leave charging unknown.

Protocol facts are separate from interface mode: only `Feature=0` mouse collections on `MI_00` use the new battery-only path. Existing `Feature=91` discovery and reading capabilities remain intact, with no additional write permissions. Tests cover protocol validation, busy/rejected/malformed replies, independent receivers and old-path compatibility. These are protocol regressions, not local Windows hardware captures.

## 2026-11-26: Arctis 7+ family (unreleased)

Pinned [HeadsetControl `bd56cf875b736ff91f69ebf17b519a79d0270564`](https://github.com/Sapd/HeadsetControl/blob/bd56cf875b736ff91f69ebf17b519a79d0270564/lib/devices/steelseries_arctis_7_plus.hpp) supplies the model/PID list, interface 3, `FFC0:0001` and five battery levels; its `protocols/steelseries_protocol.hpp` supplies the `00 B0` query. Reviewed sources are GPL-3.0. The addition reuses this project's existing communication and state implementation, without porting upstream code.

[Linux-Arctis-Manager `684f07c96bd93416d4f1dc22c67b089f2a08d2a0`](https://github.com/elegos/Linux-Arctis-Manager/blob/684f07c96bd93416d4f1dc22c67b089f2a08d2a0/src/linux_arctis_manager/devices/arctis_7_plus.yaml), GPL-3.0, independently documents body byte 1: offline=1, online=0/2/3; byte 2: battery level 0–4; byte 3: charging 0/1. Unknown charging remains unset. These are not Nova 7 charging semantics. Echo/all-zero ambiguity remains unknown.

[OpenChatMix `2413361931f2a4e28389b2a818af646df30c5487`](https://github.com/Swiftlyx/OpenChatMix/blob/2413361931f2a4e28389b2a818af646df30c5487/src/chatmix_device.cpp), MIT, cross-checks Windows' zero report-ID slot and separate input/output lengths. This implementation requires descriptor-proven unique `00/65` reports. Other sizes are not assumed compatible, and this project has no captures from these receivers.

## 2026-11-26: MCHOSE receiver additions (unreleased)

Reviewed MyFinder revision `a763adb5a0c44076f9331ab8fad63a899b5326c7`: [mchose.py](https://github.com/wushuai0802/myfinder-device-battery/blob/a763adb5a0c44076f9331ab8fad63a899b5326c7/engine/providers/mchose.py) and [test_mchose_v3.py](https://github.com/wushuai0802/myfinder-device-battery/blob/a763adb5a0c44076f9331ab8fad63a899b5326c7/engine/tests/test_mchose_v3.py), under AGPL-3.0. Protocol facts extend our existing implementation; no upstream implementation or runtime dependency is bundled.

V7 Pro `3837:101D` uses `FF01:0001`, numbered `4D` reports of 64 bytes including the report ID, and the existing `0900` status request. Replies validate the command, declared length and optional XOR; byte 20 carries percentage and byte 19 charging. V7 zero placeholders and busy replies do not publish measurements. Existing A7 zero-percent behavior remains unchanged. V7 vectors are synthetic, not captured hardware reports.

The `3837:1014 / MCHOSE A7 V3 Ultra+ / MI_02` descriptor variant has an upstream Windows success report dated 2026-11-22. Its public `ULTRA_42` prefix represents 42%, not charging; the test reconstructs trailing zero padding. The exception does not apply to other names sharing the PID. Unresolved V7 topology rejects access; failed rescans preserve existing A7 candidates.

Last reviewed on 2026-11-23. The [Chinese research record](PROTOCOLS.md) contains the full source and protocol matrix. An upstream catalog entry, a protocol implementation, simulated tests, a community report, and hardware verification are separate evidence levels.

This work is consolidated into **1.4.7**, combining the earlier 1.4.7–1.4.12 and unreleased development records. All 526 Rust tests passed in the combined run. Historical test counts below provide traceability; new hardware acceptance remains recorded per model.

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

From 2026-11-23, development proceeds in protocol-family batches. Transport and malformed-report checks are shared, while battery reads and DPI controls are validated independently. Implemented source, public hardware samples and this project's hardware tests are recorded separately. Regression tests, builds and commits cover a batch; individual models do not each trigger a release.

| Batch | Scope | Current state |
| --- | --- | --- |
| Receiver battery | VGN YJX Pro, six ryMs receivers, MCHOSE 4D | Read-only battery implemented within the compatibility limits; hardware validation pending |
| Domestic keyboards and headsets | MCHOSE K99 V3, MOONDROP GM01 Pro | Independent readers implemented; hardware validation pending |
| Other Rapoo receivers | VT `1851`, MT760 `1870` | Independent battery paths implemented; hardware validation pending |
| Basic DPI | Other Rapoo generations, VGN YJX, MCHOSE 4D | Battery support grants no setting-write capability; actual stages and readback need further review |
| Further battery work | ATK NANO, newer AULA keyboards, MCHOSE headsets | Second-batch scope below; system Bluetooth not yet integrated |

- [x] Save 1–4 custom DPI presets per device, sorted by value after editing. A stage-count selector hides the highest values and retains them for restoration; the main window and tray share the enabled list.
- [x] Distinguish actual device stages from software presets; require confirmed numerical write capability, validate supported values and verify readback.
- [x] Explain writes that alter stored DPI. Presets cannot bypass missing write support, stale identity, changed configuration or disconnection.
- [x] Implement Rapoo nRF54L profile-scoped X/Y stage reads and basic switching with protocol regressions. Hardware switching, wake and official-driver coexistence remain pending.
- [x] Add Dareu Jm battery and PAW3395 stage switching with runtime pairing/configuration checks; hardware acceptance remains pending.
- [x] Add MCHOSE Feature mouse battery/stage reads and full-configuration-preserving stage switching for A7 V2 Ultra+ / L7 Pro+; hardware acceptance remains pending.
- [x] Add AJAZZ AJ179 PRO receiver battery and USB / receiver X/Y stage switching, querying the current profile independently; hardware acceptance remains pending.
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
6. Distributable compatibility data, feedback-service deployment and acceptance, and Beta updates. The current source provides Settings → About → Device feedback, optional diagnostic previews and My feedback. Configured builds submit in the app; other builds copy content for browser submission. Service deployment and device-protocol acceptance are tracked separately.

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

## Version 1.4.7: Lofree HYZEN

The official [Lofree HUB](https://hyzen.lofree.tech/) `assets/index-D0x9DTYM.js` was directly inspected on 2026-11-21; SHA256 `078b76071f7f4cdf7851611c3a2cd5fba90b6da18502b3a844845404a059dafa`. No open-source license was found for this script. Only protocol facts were independently implemented; vendor scripts, assets and obfuscated code are not distributed.

The device list has `388D:0024` wired, `0025` wireless and `0029` single-mode, collection `FF1C:0092`. Battery calls pass `AA` or `1A`, empty data and offset 0 to `Dx`, using Report `04`. Wireless payloads are 31 bytes, or 32 with the Windows report ID. START / END use payload offset 2 values 1/2; data uses command at offset 2, address at 4/5 and data from offset 7. MyFinder `73d99e2`'s `lofree.py` was cross-checked. Its test comment references issue #82, but that issue currently describes another device and is not counted as Lofree hardware evidence.

Only online and battery commands are allowed. Each phase is bounded, requires nonempty data with zero offset, attempts END after failures and requires END acknowledgement before publication. Online status is rechecked; charging remains unknown. The vendor UI's initial wired value of 100 is not reused as a measurement. Wired suppression currently uses model presence without proven pairing identity, potentially affecting another identical keyboard; this limit is documented.

All 218 local regressions and the release build passed during integration; this capability is now included in version 1.4.7. No matching Lofree hardware was available to verify Windows collections, response sizes and sleep recovery.

## Version 1.4.7: shared VGN / VXE 320F:5088 keyboards

The saved official [VGN HUB](https://drive.vgnlab.com.cn/) bundle `assets/index-BfN6tzNB.js`, SHA256 `2dc306331d30f874eec8a006f975ef5cac3ed2f18d3bef58e5dd089dc8d5cd5f`, was inspected directly. The `weisheng` driver sends Report `04` with payload `00 00 1A 06 00 00 00`, and reads payload byte 7 when byte 2 is `1A`. MyFinder `73d99e2` supplies independent protocol and published-prefix evidence. Vendor code and assets are not distributed.

Queries require `320F:5088 / FF1C:0092`, 64-byte input/output with Report `04` only, and one management collection per USB interface group. This protocol does not use Cherry's `04 20` prefix. A published response took approximately 1.03 seconds; the new backend allows 2.5 seconds and runs independently of V98 Pro V3. Sleep `FF`, echoes and invalid reports cannot renew the sample. Values expire after 120 seconds and the maximum background wait before another query is ten seconds.

Shared IDs/product strings cannot exclude Rainy75 or establish a VGN model. Product names and instance identities are retained; charging, cross-process attribution without transaction IDs and full sleep cycles need hardware verification. All 223 regressions and the release build passed, including shared UI/tray expiry and background recovery; upstream observations are not local hardware acceptance.

## Version 1.4.7: VGN / VXE 20-byte keyboard receivers

Pinned VGN HUB resource `index-ZngoA_qG.js`, MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` files `vgn_beiying.py` and `by_keyboard.py`, and issue #190 hardware evidence were cross-checked. The official catalog assigns five wireless identities to `FF02:0002`, Report `13`: `258A:024F/027F/027B` and `391D:2A01/2A02`. Wired `FF00:0001 / 09` and shared `3554:FA09` remain outside this reader; the unified MCHOSE keyboard backend retains exclusive ownership of FA09.

The complete request is `13 4A 01`, sixteen zero bytes and trailer `5E`; the trailer is the wrapping sum of the preceding 19 bytes. A reply must be exactly 20 bytes with header `13 4A 01 00`, must not equal the request or identity query, and must pass the same checksum. Full-frame byte 5 is a percentage from 0–100. Byte 4 is packet metadata and is not interpreted as a payload length. Only `258A:024F` has issue #190's controlled external-power comparison for byte 6 bit `0x10`; charging remains unknown on the other four IDs. MCHOSE high/low-nibble rules are not inherited.

The descriptor must prove one unique `FF02:0002` management collection per USB interface group and that both directions contain only a 20-byte Report `13`. Product strings and a model query are not used to establish eligibility. The exact VID/PID table supplies product names, but it is not treated as full hardware acceptance. Each query allows 2.5 seconds and failures do not renew a sample, which expires after 120 seconds; the ten-second background publication limit follows the existing tray state.

All five new identities still require matching wireless hardware. The existing wired `258A:024E` V98 Pro V3 path remains separate and returned a valid 100% plus charging in this project; that result does not validate the wireless transports. Descriptors, 0%, charging transitions, sleep recovery and coexistence with VGN HUB require later hardware records.

## Version 1.4.7: EWEADN S9 (M156)

Only the `089D:062F / FF01:0010` M156 query is independently implemented from packet facts published under MIT in HaloBattery PR #352 and MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` files/tests `eweadn.py`. It accepts a 33-byte output and a 16–34 byte Windows input capacity and requires a descriptor proving one unnumbered Report 0 in each direction. Wired `088D:062E`, `FF00:0000`, M302 and other product names are excluded; passive `D1` notifications are not used.

The output request is `00 30` plus 31 zeroes. The first byte is Windows' Report ID slot; payload byte 0 is `30` and the remaining payload bytes are zero, so the checksum over payload bytes 5–31 is zero. Input uses the actual descriptor's complete capacity before removing the zero Report ID, rather than assuming a fixed 33-byte buffer. The earlier reduction of the receive-buffer length was corrected on 2026-11-23.

A reply must be 15–33 bytes, begin `30 00`, declare enough data for bytes 4–14 without exceeding the frame, contain ASCII `M156` at bytes 4–7, place a 0–100 percentage at byte 13 and the observed connected value `01` at byte 14. No charging field is established, so charging remains unknown. `30 02`, other states, wrong identity, truncation and request echoes never become 0%. One send may ignore unrelated input for up to one second and accept the first valid M156 frame; no extra online or configuration command is sent.

All 356 local regressions and the warning-free Windows x64 release build pass, covering 15–33 byte inputs, every percentage, strict identity and malformed frames. No local S9 was available. Real Windows caps, comparisons such as 70% / 69%, sleep, removal, cold start and vendor-driver coexistence remain pending.

## Version 1.4.7: EWEADN S9 PRO 8K

The read-only `372E:1026` basics query is independently implemented from pinned EWEADN HUB `vendor-bytech`, MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc`'s `eweadn_s9pro.py`, and user hardware evidence in issue #315. Other S7/S9 catalog entries share the same VID/PID, so the current display name is not established by a runtime model query.

The vendor catalog splits control across `FF00:0001` and `FF06:0001`. Exactly one collection in an interface group must have a runtime descriptor proving Feature Report `03` is 64 bytes. A HidP collection maximum by itself is not report-ID proof, and another ID may own a larger report. A proven map may contain both `03:64` and `09:128`, while only `03` is transferred. Zero or multiple proven collections fail closed; the reader neither prefers FF00 nor retries the other page after a failure.

The complete Feature request is `03 20 50 00 00 4F 81` plus 57 zeroes. Checksum `20` is the wrapping sum of the zero-initialized 63-byte payload and excludes the Report ID. The reviewed controller performs SET_FEATURE, waits 100 ms, then GET_FEATURE `03`; no state-changing handshake prerequisite is established. Two reply shapes are accepted:

- The 16-byte user-verified form `03 50 00 0B 00 01 <battery> ...` requires presence byte `01` and a percentage from 1–100.
- The 64-byte descriptor-capacity form requires ID `03`, ACK `00`, encapsulation index `00` and declared length 7–58; percentages from 0–100 are accepted. Empty, echoed, truncated, oversized, wrong-ID/ACK/index and out-of-range packets are rejected.

Both forms use the low seven data bits as percentage and the high bit as charging. No response checksum, fixed TYPE or fixed OFFSET is invented. One query permits one request and one receive, with no retry and no UUID, online, reset, DPI, polling-rate, macro, display, pairing or firmware request.

Issue #315 records the 16-byte reply, stable 23%–98% readings and charging changes on Windows x64. This project has no independent matching hardware. Other models under the shared ID, real 64-byte replies, sleep caching, wake-up, cold start and vendor-driver coexistence remain unverified.

## Version 1.4.7: Flydigi APEX5

The `37D7:2501 / FFA0:0001` GET_INFO battery query is independently implemented from MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc`'s `flydigi.py`, the three batch protocol records and their published license boundaries. A descriptor must contain only 32-byte Input Report `04` and 32-byte Output Report `03`, with one candidate in the interface group.

The sole request is `03 5A A5 01 02 03` plus 26 zeroes. Replies must be 32 bytes and match `04 5A A5 01 01 00 80`, with mode byte 7 equal to `01` or `02`. Byte 12 bits 0–3 encode tiers `0..5`, mapped to 0/20/40/60/80/100% and displayed uniformly as `≈` estimates. Bits 4–7 accept only 0 or 1. A high nibble of 1 is a vendor charging sentinel and is never interpreted as a sixth tier or illegal percentage; it reports charging with unknown level. Other tiers, charge values, echoes and malformed frames are rejected.

No reply checksum is established, so request checksum rules are not applied to replies. Exactly one GET_INFO request is sent; no IMU, rumble, trigger, lighting, mode or configuration command is reachable. All 356 local regressions and the warning-free Windows x64 release build pass. No local APEX5 was available; real caps, sleep, charging transitions, 0%/100% and desktop-software coexistence remain pending.

## Version 1.4.7: GameSir Nova / Cyclone 2

Protocol facts are independently implemented from the MIT-licensed Wireless-Device-Battery-Monitor driver records and public GameSir client descriptions; no driver code is copied. The Nova family is limited to `3537:1098/100F/1040` and `FF7A`. A descriptor must prove one unnumbered Report 0 in each direction, with input capacity at least 19 and output at least 3. The request is Windows' Report ID slot `00` followed by `01 01`. A `F4` state frame may arrive first; the reader waits up to 800 ms for a frame beginning with `01` and reads byte 18: 0–100 is an exact percentage and `FF` is a charging sentinel; other values are rejected.

Cyclone 2 is limited to `3537:100B / FFF0:0040`, 64-byte input/output and Output Report `0F`, Input Report `12`. The fixed request is `0F 03`, which only causes the controller to stream state. Replies must be Report `12`, byte 35 accepts only 0/1 and byte 36 is a 0–100 percentage. Other reports, input lengths, charging enums and levels are rejected.

Neither path exposes settings writes; rumble, trigger, lighting, button, mode and configuration commands are unreachable. All 356 local regressions and the warning-free Windows x64 release build pass, covering exact identity, descriptors, requests, 0%/100% and malformed frames. No matching hardware was available. Real caps, sleep, wired/2.4 GHz transitions, multiple controller instances and vendor-software coexistence remain pending.

## Version 1.4.7: ATK N9 V2 ULTRA / MCHOSE V9 headsets

The FF90 headset backend gates identities by exact `FF90:0001`, Reports `55` in both directions and descriptor capacity. `373B:12D6` ATK N9 V2 ULTRA is added with a 22–64 byte input range; `373B:1262` N9 V2 Ultimate uses the same protocol. The fixed request is `55 65 01` plus 61 zeroes. ATK states `02/03/04` mean discharging, charging and full, all with byte 2 in 0–100; state 4 preserves the reported percentage.

`3837:600A` MCHOSE V9 Turbo+ retains 64/64 bytes. State `02` reads a percentage, `00/03` means external power with unknown level, and `1A` means explicitly offline; other states are rejected. External power carries a charging flag without substituting 100% or the previous percentage. Offline and silence are separate outcomes. The `3837:6008` lighting interface is never probed.

`291D:385D` V9 PRO uses the same FF90 battery protocol but adds a firmware gate. The same physical receiver must expose one `FF22:0001` collection whose descriptor proves Feature `AA/64`. A read-only `AA 01 01` plus 61 zeroes is followed by a 300 ms wait and Feature `AA` receive; bytes 2–5 are concatenated as decimal text to obtain the version. Confirmed versions below `0008` block battery I/O. Unknown versions, failed reads or ambiguous descriptors retain the conservative fallback without repeatedly retrying the firmware query.

All 357 local regressions and the warning-free Windows x64 release build pass, covering ATK's 22–64 byte inputs and states 2/3/4, MCHOSE states 0/2/3/1A, echoes, firmware decimal versions and descriptors. None of these headsets is available locally. Real caps, charging/full/unplug transitions, sleep, firmware versions and vendor-software coexistence remain pending.

## Version 1.4.7: MCHOSE QHW and BY keyboards

The current [M HUB](https://www.mchose.com.cn/) entry was checked on 2026-11-21. Application `assets/main-DIt5vAcV.js` has SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`; the official manifest points to `cizhou/CZ_SHARED_DATA/main.002bbdb0f17b5f7135ee.js`, SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3`. No vendor implementation license was found. Files were inspected statically for protocol facts, without executing or distributing their code/assets, and cross-checked with MyFinder `73d99e2`.

The QHW catalog identifies G98 V3 wireless as `3837:1015 / MCHOSE G98 V3 2.4G / FF80:0001`. `getBattery` calls `_simpleGet(2308, [])`: Report `4D`, little-endian command `0904`, payload `01 03 00 04 09 00 ss cc`. Sequence increments modulo 256 and output checksum XOR excludes version, giving `0E XOR ss`. Reply version, flags, command, bizCode and sequence must match, with data length exactly two. Payload fields are charger enum and percentage; enum 2 is full, not actively charging. The official parser extracts the reply checkCode without proving its algorithm, so no incoming XOR rule is invented.

Input and output capacities are proven independently from Windows descriptors, with minimum lengths 11/9 including the report ID. Writes use the actual output capacity and replies must match actual input length. `NORMAL_MAX_PACKET_LENGTH=31` is a code limit, not a hardware descriptor measurement. Only battery GET is sent. Independent cold-start reads and stale sleep responses remain hardware questions.

BY `keyboardPwds` contains six-byte firmware model identifiers, not login credentials. Report `13` / command `05` reads the code at full-frame byte 5. The decoder never shifts bytes to force an expected model. Official `handleCommandResponse` checks packet count/index and discards byte 4 and the last byte; FA09 and new `41E4:2001` therefore do not inherit an unproven length-metadata or reply-checksum rule. Actual Report ID, 20-byte length, command, single-packet framing and echo rejection remain required. Z75S and separate VGN V98 Pro V3 retain their existing strict validation.

Recognized models also accept `0A`, type `05` battery notifications during the bounded battery-query window. M HUB's `batteryParser(false)` skips bytes 2–4 and takes percentage/flags at 6/7. Both flag nibbles must be 0/1; high nibble 1 means charging only below 100%, and full flags never replace the percentage. Unknown FA09 models accept only valid `4A` percentages, with unknown charging. Other named brands do not receive an extra MCHOSE identity query. Wireless BY zero is suppressed because it may be a sleep placeholder.

FA09 retains captured SET_REPORT transport; Z75S and `41E4:2001` use interrupt OUT without fallback resends. The latter is restricted to exact generic product names, `FF04:0002` and recognized model codes. Cross-USB-interface FF02 sibling pairing is not implemented. All entries require bidirectional Report `13` descriptor evidence and an unambiguous interface-group control collection; feature-only AULA/FA09 paths receive no command.

All 236 local regressions and a warning-free release build passed. Coverage includes QHW sequence wrap, invalid replies and distinct capacities; BY unknown identity, notifications, sleep placeholders and background recovery. MyFinder #476's two complete frames are retained verbatim: the identity does not prove G75 Pro, while battery gives a neutral 100% with unknown charging. #312 provides a prefix with synthetic trailing test padding. Existing ROG read 79% / DPI 800 and VGN read 100% / charging in a read-only probe; no newly supported keyboard was present for hardware acceptance.

## Version 1.4.7: MCHOSE G87 V2 GLW keyboard

This reader is independently implemented from pinned static M HUB/CZ SDK evidence and MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` files `mchose_glw_keyboard.py` and `mchose_keyboard_hid.py`. `3837:3033` is shared by multiple keyboards, so entry requires the exact product `MCHOSE G87 V2 2.4G`, collection `0001:0000` and 65-byte input/output capacities. A descriptor must prove a sole input report ID, which may be zero, and a sole output Report 0. Multiple matching collections in one interface group are refused.

Windows HidP capacities include the Report ID slot, and ReadFile must also use the complete capacity. For descriptor-proven unnumbered input, the leading zero is removed after reception before parsing the 64-byte payload. The earlier reduction of the receive-buffer length was corrected on 2026-11-23. Interrupt OUT remains restricted to this collection. Only 64-byte payloads with a leading Report ID are allowed:

```text
55 04 00 SS 38 00 00 00 ...    // read 56 bytes of base information
55 05 00 SS 38 OO OO 00 ...    // read the first 56 bytes of the active profile
55 05 00 SS 08 OO OO 00 ...    // read the final 8 bytes of the active profile
```

`SS` is the wrapping sum of payload bytes 4–63 and `OO OO` is the little-endian active-profile offset. Base byte 0 is the current index; a complete `0..3` permutation at bytes 2–5 maps to a real slot, while other layouts retain the documented legacy direct index. Each `05` response must declare 56 or 8 bytes and echo the offset. The combined 64-byte configuration uses byte 32 bits 0–6 for 0–100 percentage and bit 7 for charging.

Replies must match the `AA` marker, command, length and offset. Reply checksum and other header flags are unconfirmed and are neither required nor guessed. Base repair, mode changes, polling-rate, DPI, lighting, macro, pairing and firmware commands are unreachable. All 356 regressions and the warning-free release build pass, covering zero/nonzero input IDs, request checksums, all three read exchanges, slot permutations and malformed frames. No G87 V2 hardware was available; real caps, sleep, cold start, profile changes and vendor-software coexistence remain pending.

## Version 1.4.7: Edifier Halo Space

Battery command `D0` is independently implemented from FerrisPulse `335fb15`'s [public protocol record](https://github.com/Ferris-echo/FerrisPulse/blob/335fb15ba98be94a85e13061dc0812a04b3aadf6/docs/EDIFIER_HALO_SPACE_PROTOCOL.md), helper and packet tests. It specifies `2D99:E049 / FF14:0001`, 64-byte input/output, input prefix `2F BB EC` and output `2E AA EC`. `FF13` is a distinct collection. Upstream's broad collection fallback and permissive alternate headers are not used.

The fixed query is `2E AA EC D0 00 00 66` with zero padding. Its first byte already occupies the Windows report-ID slot; no extra `00` is prepended. Reply big-endian payload length must be one. Sum wire bytes 1–6 against byte 7 before unmasking byte 6 with XOR `A5`. Published prefix `2F BB EC D0 00 01 86 FE` yields 35%. Trailing historical scratch data never supplies battery or charging information.

Descriptors must additionally show only input `2F` and output `2E`; this conservative entry condition needs real caps acceptance. No C8 MAC, C9 name, C6 firmware or unsupported F2 status query is issued. Charging stays unknown rather than borrowing the upstream UI's false as evidence of discharge. Cross-process reply attribution, charging-cable disconnection and sleep-cache behavior remain hardware questions.

All 241 regressions and a warning-free build passed, covering valid zero/full, bad checksums, wrong endpoints, lengths, stale tails, identical units and shared main/tray expiry and recovery. Upstream hardware claims are protocol-source evidence only; no matching headset was available locally.

## Version 1.4.7: Dareu Jm mice

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

## Version 1.4.7: MCHOSE Feature mice

Cross-checked the current M HUB `main-DIt5vAcV.js` (hash above), including device/config parsers, write schemas, Feature transport and product catalog, against OpenMouse `5a4f643`'s [MCHOSE protocol record](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/mchose-protocol.md) and published L7 Pro+ captures. No vendor or upstream implementation is distributed.

Entry is restricted to `3837:100B/1020/4018/4023/4019/4021/4015` or `5253:1020/1011`, `FF01:0001`, and a 65-byte Feature capacity. Descriptor caps must contain Feature 11/12; Feature 14 is allowed, other IDs are rejected. Firmware collection FF0B, MagDock, QHW keyboards, A7 V3 Report 4D, old A5 and Bluetooth are excluded. Ambiguous management collections are rejected and identical serial strings do not merge receiver instances.

Report 11 commands 03/06 invert the command and 19 zero argument bytes, leaving the remaining wire padding zero. Report 12 command 67 reads configuration. Each query sends once, then accepts two consecutive equal valid payloads within 1.9 seconds/20 reads at 90 ms intervals. Gaps, foreign commands and empty echoes reset that proof. Short replies may be the captured 21 bytes or Windows' 65-byte capacity; only their defined 7/11 bytes count. Config replies must contain 65 bytes, with all 63 configuration bytes stable.

Command 06's VID/PID identifies the mouse, not the shared receiver. The vendor parser's little-endian bit fields start at bit zero: mode is flags bits 0–2 and connection is bit 3. OpenMouse's high-bit interpretation is not reused. Published L7 Pro+ cable data decodes to `3837:4015`, 57%, charging, flags 00; receiver data gives the same mouse, 100%, not charging, flags 09. Wireless reads also check receiver VID/PID, pairing and online status with command 03 before/after. Charge enums other than 0/1 stay unknown. Wireless zero does not renew a sample because the vendor uses it as a sleep placeholder; valid wired zero remains zero. Explicit offline clears old battery, silence does not renew it, samples expire after 120 seconds, and the maximum background wait is ten seconds.

Runtime models include A7 V2 Pro `4018`, Pro+ `4023`, Ultra `4019`, Ultra+ `4021`, L7 Pro+ `4015`, L7 Pro `00B0` and A7 Ultra `1011`. Unknown models may retain valid battery but receive no configuration access. Some old `5253:1020` A7 Ultra receivers are documented to return empty 06 replies; product strings do not fabricate battery or establish A7 Ultra identity.

Read only the active one of three profiles. Config bytes 1/2 contain wired/wireless stage indexes in their low nibbles; byte 16 is the enabled count. Preserve raw indexes, duplicates and 50-DPI steps; six populated slots do not prove six enabled stages. Only the verified X values are displayed, without assuming Y equality or its enable mode. A7 V2 Ultra+ and L7 Pro+ have upstream whole-config write evidence and receive stage switching; other models remain read-only.

The fingerprint includes mouse VID/PID, firmware, transport and all 63 configuration bytes. A click reconfirms link, identity and config twice. Command 57 changes only the live link's stage nibble; all other 62 bytes and the polling-rate nibble are retained. After 400 ms, full readback must match every expected byte and the same connected identity. A send timeout is never retried or blindly rolled back. This switches existing stages only; arbitrary DPI/custom-preset writes are not offered. External-driver races after the last check and replacement by an identical model remain unresolved protocol limits.

All 268 regressions and a warning-free build passed. Battery/config decoding uses published L7 Pro+ raw frames; other tests use synthetic reports for interleaved commands, partial replies, changing tails, unknown models, sleep/offline, preserved configuration, concurrent edits, lost send completion, ignored writes and corrupted neighbors. Wireless-zero placeholders cannot enable settings. Main and tray controls share the configuration token and backend queue. No matching MCHOSE mouse was available locally; Windows caps, stale receiver caches, complete sleep cycles and stage changes await hardware acceptance.

## Version 1.4.7: AJAZZ AJ179 PRO / GearHub

Cross-checked OpenMouse `5a4f643` [AJ179 PRO notes and original replies](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/ajazz-aj179-pro.md) against the current [GearHub](https://qmk.top/) bundle and its [v4 driver](https://qmk.top/v4/) catalog/Pan1080 classes. Vendor source and assets were inspected statically, not copied into the implementation or distribution. Source SHA256 values are recorded in the corresponding Chinese section, including v4 `f1e16e07.js` (`43e9a29f48d16ee292aea1f229c0e17b984a039971cde54ad3917de2578e90cc`) and the AJ179 subclass `1167ee1d.js` (`af7392515ea3384e4db151de2d2b6c4c5165c9edd3ac5cb1a92424c9207a6106`).

Both vendor catalogs map mouse ID 1851 to AJ179 PRO; command 8F returns its little-endian 32-bit ID at body bytes 1–4. Host `3151:402D/4026` identifies receiver/USB transport only. Discovery requires one `FFFF:0002` management collection per Windows USB interface group, Feature capacity 65 and exclusively ID 0. The public descriptor has an unnumbered 64-byte body; Windows prepends the zero report ID. Truncated or shifted reports are rejected.

The receiver flow is F6/05 target selection, F7 send-ready, mouse command, F7 read-ready, FC notice-read and Feature GET. F7 bytes 0/5 are read/send readiness, byte 4 is mouse offline and byte 2 is mouse percentage; byte 1 is the keyboard's battery. Flags must be 0/1. All-zero, echoed, foreign-command and out-of-range replies fail validation. Each readiness wait is bounded to eight attempts and a deadline; individual Feature transfers use a 150 ms timeout. Charging has no verified field and USB does not query receiver battery.

Mouse requests store the complemented sum of their first seven bytes at byte 7. Reply byte 7 is not recalculated over returned fields, so no invented response checksum is imposed. Responses must match their command and differ from an unchanged request. The protocol has no transaction ID: identity boundaries and reverse configuration rereads reduce mistakes but cannot establish freshness of internally cached replies or exclude other processes' same-command responses.

The vendor's D3 `getMouseOption0` reads the current profile from byte 8; the AJ179 catalog has eight profiles. D4 uses that profile rather than OpenMouse's fixed profile 0. D4 bytes 2/3 carry active index/slot capacity; X starts at byte 8, Y at 24, with 16-bit little-endian values, and three-byte colors begin at 40. Each both-zero X/Y pair is disabled, including internal gaps; raw indices and duplicate values survive. The current catalog allows 50–32000 in steps of 50, whereas upstream hardware write evidence covers 50–26000. Higher valid readings remain visible, but any enabled axis over 26000 prevents whole-table writes.

The configuration token covers model ID and complete D3/D4 blocks. Switching reconfirms identity and configuration twice, then sends command 54 once, changing only command, active index and request checksum. All eight slots, disabled-slot colors and reserved bytes are preserved. After 400 ms, full table, original options and identity must match. Lost send completion leads to readback, never a resend or blind rollback. D3, key maps, macros, profile selection and polling rate are not written. Rate reads only decode established values; unknown codes have no default.

Published original frames decode AJ179 PRO, 85%, active stage 3 at 1000 DPI and five enabled slots in an eight-slot table. Synthetic tests cover nonzero profiles, internal disabled slots, separate axes, high-DPI read-only limits, configuration races, lost completion, ignored writes, altered neighbors, offline/wake and background refresh. No local matching hardware was available. Windows caps/Feature transport, receiver caches, physical switching, persistence, identical-model replacement and cross-transport deduplication remain hardware or protocol limits.

On 2026-11-21, all 285 local regressions and the warning-free Windows x64 release build passed, including PE/version checks and the copied executable's SHA256. Existing ROG returned 77% / DPI 800 and VGN returned 100% / charging in a read-only probe. No GearHub control interface was enumerated and no physical DPI writes were performed.

## Version 1.4.7: AULA SC380 PRO

The official SC380 PRO installer (SHA256 `99a0987b762d0dc6e4b3ecb23afc32c2c57debf547f9b0e9ed503830e096b9b5`) contains `Dev/ms/MS.ini`, which identifies wired `258A:0136`, receiver `258A:0138`, model signature `04 00 00 00 00 2E` and PAW3311. The adapter was independently implemented from these facts and OpenMouse `5a4f643`'s [SC380 PRO protocol record](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/aula-sc380-pro.md). Only receiver `FF02:0002 / Feature 09 / 520`, which has upstream Windows evidence, is enabled. A unique management collection per interface group and descriptor proof of exclusively Feature 09 are required. Wired mode, name-based matching and other AULA devices are excluded; vendor resources and upstream code are not distributed.

The Windows header is `09 checksum command argument 01 00 lengthLE16`, with payload at byte 8 padded to 520 bytes. The request checksum sums byte 2 onward. Short replies retain the request-header checksum, while settings replies include their payload in the sum. Full length, command, argument and declared data length must match; stale trailing bytes provide no freshness evidence. `05/01/6` reads the model, `07/00/1` reads link state and `09/01/1` reads battery. Only online value 1 is accepted. The zero battery payload is byte-for-byte the request payload, so this path publishes neither a real 0% nor explicit offline. No charging field is established. Queries wait 30 ms, each Feature transfer is limited to 150 ms and each operation has a 3.5-second deadline.

`44/00/400` reads the full settings block. Marker `64` and the PAW3311 low-five-bit sensor code `18` are required. Byte 3's low nibble is the one-based count 1–6 and its high nibble is the one-based active stage. X starts at byte 7 and Y at 27 as little-endian 16-bit words; DPI is `(word + 1) × 100` from 100–12000. Byte 2's low nibble maps 1–4 to 125/250/500/1000 Hz and is read-only. Original order and duplicate values are retained. The six default stages in the installer are not used as observed device data.

Configuration reads require two identical complete replies. The switching token covers the model and all 400 bytes. Before writing, identity, online state and the complete configuration are rechecked. `04/00/400` changes only byte 3's active-stage nibble and preserves every other byte. After 250 ms, the complete block is read twice and identity/online state is checked again. A lost send completion leads only to readback, never a retry or blind rollback. Device replacement, configuration races or changed neighbor fields fail the operation. The protocol has no cross-process transaction and cannot uniquely distinguish same-model replacement; hardware limits remain open.

On 2026-11-21, all 298 local regressions and the warning-free Windows x64 release build passed. Tests cover malformed headers, different checksum scopes, zero-payload echoes, range overflow, configuration races, complete-block preservation, readback after a lost completion and shared main-window/tray expiry recovery. New fixtures are synthetic protocol reports rather than local hardware captures. No matching device was available, so actual descriptor caps, wake behavior, physical stage switching and power-cycle persistence remain unverified.

## Version 1.4.7: AULA F75 / F87 PRO keyboards

The [official AULA HUB page](https://www.aulastar.com/aula-hub/) lists F75/F75 MAX, F87 PRO V2, F99 and F108 PRO/V2. Its web application returned 403 on 2026-11-21, so HUB 1.1.22's official offline archive was inspected without installing or running vendor programs. Archive SHA256: `f18f4bfa2a6d66f24bed8d2c17b61eff8530ebc9ad0615344f72003f5bf9db08`. Catalog bundle `index-npIkQlTa.js`: `0b97fd16dc42ad77da883c318af8924318aeebd759270c944518c8afe7278090`; Bytech bundle `index-BPTK-vUa.js`: `3090652c88abbaf61e68f1b643929ff4995068d309dbd6307f8aa6452e00d19b`. This batch integrates only exact F75 and F87 PRO identities; other catalog entries remain outside the claimed scope.

F75 code `03 00 00 00 00 CD` and F87 PRO code `03 00 00 00 03 E3` join the existing `Keyboard20` shared-receiver backend rather than starting another reader on the same `3554:FA09` channel. The interface must match the existing `FF02:0002`, 20-byte Report `13` and unique management-collection requirements. F87 is excluded because its official wireless catalog entry uses `FF00:0001`, which conflicts with this receiver path. An unknown code is never named as AULA from the receiver ID alone.

The AULA identity request uses Report `13`, command `05`, argument `01`, padded to 20 bytes; its additive checksum covers complete-frame bytes 0–7 and occupies byte 8. Battery command `4A 01` has no corresponding checksum. Replies require 20 bytes, the `13 command 01 00` prefix and freedom from an unchanged request echo; the model occupies complete-frame bytes 5–10. Battery percentages 1–100 are valid, with high nibble 1 indicating charging and low nibble 1 full. Unknown nibbles and the zero-payload placeholder/query echo are rejected. Per the AULA UI, charging may remain true at 100%; MCHOSE's full-battery suppression is not applied.

The shared backend tries a known cached protocol first, then probes AULA before MCHOSE on a fresh channel. A successful choice is cached for that Windows HID instance for 120 seconds, avoiding a 2.2-second timeout against the other framing on every poll. AULA battery is queried only after an exact model match, and identity must match again after the reading. Once a channel is definitively AULA, a failed sleep/wake read is not handed to MCHOSE for reinterpretation. Tests cover both request layouts, both recognized models, wrong codes, charging/full nibbles, unknown flags, zero placeholders, replacement between reads and instance isolation.

F75/F87 Pro receiver catalog entries use `3554:FA09 / FF02:0002` and six-byte model codes `03 00 00 00 00 CD` / `03 00 00 00 03 E3`. Wired entries use `258A:010C / FF00:0001`. The F87 receiver entry instead specifies FF00:1 and cannot be assumed equivalent. The wireless class selects actual input/output report IDs with a 19-byte body, command 05 for identity and 4A for battery. Identity appends an additive checksum at complete-frame byte 8, while the battery request has no equivalent checksum. Existing MCHOSE requests therefore cannot be extended just by adding names.

The wireless parser reads identity and battery from WebHID body byte 4; the following byte's high/low nibbles indicate charging/full. The vendor notification handler gives one message different interpretations by caller, which does not establish reliable standalone connection/battery evidence and is not subscribed here. Wired Feature bodies are 519 bytes, use identity 82/battery 87 and skip seven header bytes after removing the report ID. This does not validate the older eight-byte Feature 06 byte-3 battery guess, contradicted by MyFinder issues #99/#485/#477; that path remains disabled. F108ProV2C now has a separate implementation from its official catalog and 56-byte HFD information reader, described in the MT560, R9 and AULA / Dareu section below. This does not establish support for the non-C variant; actual caps and cold starts still require acceptance.

On 2026-11-21, all 303 local regressions and the warning-free Windows x64 release build passed. New fixtures are synthetic reports constructed from the official static code. No local F75 / F87 PRO was available, so Windows SET_REPORT transport, receiver caching, switching between MCHOSE and AULA devices on one receiver and vendor-software coexistence remain unverified.

## Version 1.4.7: 8BitDo Pro 3 controller

Cross-checked MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` against the [8BitDo HID driver](https://github.com/libsdl-org/SDL/blob/5e7b0bd9f1fbe7e661cf6bb0cdbf31d37be2cf50/src/joystick/hidapi/SDL_hidapi_8bitdo.c) at pinned SDL commit `5e7b0bd9f1fbe7e661cf6bb0cdbf31d37be2cf50`. The SDL file SHA256 is `084a79270595e7a99c15b89c7e7b5f167fc1ae0156df8c51537e5c085fe8ed53`. Only battery facts were independently implemented; no SDL source or assets are copied.

Discovery is limited to D-input mode `2DC8:6009`, standard Gamepad `0001:0005`, and a descriptor proving that every Input Report ID is `04` with 15–64-byte input capacity. XInput mode uses another PID and does not enter this parser. The adapter opens a read-only handle for at most 64 reports within 400 ms and sends no Feature, Output, mode, lighting, rumble or initialization command. SDL reads Feature 06 to enable enhanced reports on other 8BitDo models, but a Pro 3 user confirmed Input `04` already carries battery in D-input Bluetooth with Steam closed and the controller freshly started, so that initialization is not copied here.

Byte 14 of the full input report stores the percentage in bits 0–6 and a charging bit at bit 7. Values 0–100 are retained and 101–127 are rejected. Following SDL's charged-state mapping, 100% is treated as full with `charging=false` rather than continuing to show an active charge in the tray. Each valid report supplies a new sample time; silence does not renew the previous value, numbers expire after 120 seconds and background publication remains capped at ten seconds.

On 2026-11-21, all 308 local regressions and the warning-free Windows x64 release build passed. Tests cover wrong VID/PID, wrong usage, report ID, short frames, 0%, 101–127, the charging bit, full-at-100%, noise skipping, bounded report counts and identical-model instance isolation. Fixtures are synthetic field constructions. No local Pro 3 was available, so actual Windows descriptors, charging transitions, sleep recovery, multiple controllers and coexistence with Steam or vendor software remain unverified.

## Version 1.4.7: VGN Dragonfly3 SE

Cross-checked pinned VGN HUB resource `index-ZngoA_qG.js` (SHA256 `53adfab22f478249bdb9117bb26cedf8a7d3c6c561f2f2eb735e7d86e943be63`) at `Yd.GetBattery=48`, `oc.getBattery` and `sendHidBuffer`, against MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` file `engine/providers/vgn_dragonfly3.py` (SHA256 `e91a2f755b2d6b84de3d088cfd7900d62300b3fb67a3762b88ccea9106c2e4bc`). Only protocol facts are independently implemented; vendor scripts and upstream implementation are not distributed.

The initial implementation enabled 2.4 GHz receiver `391D:1A01 / FF01:0010`. Its descriptor must prove unnumbered input and output Report 0 at 65-byte Windows capacity and one eligible collection per interface group. Pro `1A03` was added on 2026-11-23 as described below; the diagnosed `FF05:0002` collection, wired `1001` and other PIDs remain excluded. The request is `00 55 30` plus 62 zeroes. ReadFile uses the complete 65-byte capacity, then the zero ID is removed before parsing the 64-byte protocol payload.

The reply is correlated only by byte 1 equal to `30`. Byte 9 of 1 identifies wired mode, publishes no battery and explicitly clears any previous wireless percentage. With byte 9 zero, byte 8 accepts 0–100. Exact request echoes, short frames, out-of-range levels and unknown connection enums create no fresh sample. The vendor path establishes no charging field, so charging remains unknown. Each query has a one-second, 32-report bound; background publication is capped at ten seconds and a silent device stops renewing the old sample.

On 2026-11-21, all 313 local regressions and the warning-free Windows x64 release build passed. Tests cover exact VID/PID, usage, unnumbered 65-byte capacities, request layout, 0/100, 101–255, wired clearing, request echoes, short frames and instance isolation. Fixtures are synthetic field constructions. No local Dragonfly3 SE was available, so actual Windows caps, receiver caching, wake recovery, physical accuracy, 0% and vendor-software coexistence remain unverified.

## Version 1.4.7: HECATE G3M PRO

Cross-checked MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` file `engine/providers/g3m.py` (SHA256 `520c397ec971eba8d8d49eb2720721c3b9bcf15bcf1e5f6426c3991ca848349b`) and the protocol facts it cites from BoBppy/g3mpro-macos commit `6271ac2`. Only battery facts are independently implemented; upstream code and assets are not copied.

Only `320F:706E / FF1C:0092` is enabled, with a descriptor proving that input and output each contain only a 64-byte Report `04` and one eligible collection per interface group. The request is `04 20 00 1A 06` plus 59 zeroes; byte 1 (`20`) equals the sum from request byte 3 onward. Replies require 64 bytes, Report `04`, command `1A`, byte 7 not equal to `FE/FF`, and a percentage byte 8 from 0–100. Exact request echoes, short frames, wrong report IDs or commands and error status are rejected.

No response checksum algorithm or charging field is established, so neither is inferred. The adapter does not start a configuration session or read/write DPI, lighting, online handshakes or firmware. Queries are limited to one second, background publication to ten seconds, failed reads do not renew the old sample, and values expire after 120 seconds. On 2026-11-22, all 319 local regressions and the warning-free Windows x64 build passed. Fixtures are synthetic protocol constructions; no local G3M PRO was available, so actual caps, sleep/wake, 0%, physical battery accuracy and vendor-software coexistence remain unverified.

## Version 1.4.7: receiver, keyboard and headset protocols

Only reviewed protocol facts are independently implemented. External source code, test tools and vendor assets are not distributed. Exact model, connection and hardware-validation limits are recorded in the [compatibility notes](COMPATIBILITY.en.md).

| Path | Evidence | Protocol facts and limits |
| --- | --- | --- |
| VGN YJX Pro / ryMs | Saved official VGN HUB `vgn-main.js`, SHA256 `2dc306331d30f874eec8a006f975ef5cac3ed2f18d3bef58e5dd089dc8d5cd5f`; FerrisPulse `a916c657c18034a30300dd08b33c6c192c86fb06`'s `VgnDeviceHelper.cs`; MyFinder `aa0228abbc850b693620d466147b35dfe50c8dca`'s `vgn_ry_ms.py` and PR #717 | `1A03` shares YJX command `30` with SE; the official parser identifies a cable flag rather than charging. Six ryMs receivers use F7, complemented-sum byte `08` and Feature 0/65; full-frame bytes 3/5 are percentage/sleep. No F6 initialization or FC configuration command is sent. |
| MCHOSE 4D | Saved official M HUB `main.js`, SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`; Voltide `c1872c4c93124d62c05bf5887fe89c32c6be8dd6` K7 provider; MyFinder `aa0228a` protocol notes and #535 Windows descriptor | Fixed `4D 01 01 00 00 09 00 00 08`, padded to 64. Version, command, business code, sequence, declared length, optional XOR, mouse VID, connection and level are checked. Public Ultra+ 85%-charging and 42%-discharging payloads are replayed with reconstructed transport padding; they are not local captures. Unknown charge flags remain unknown. |
| MCHOSE K99 V3 | [mchose-hid-bridge `ad74f3e75fd5ae099b2ea4af039678c54dea73ac`](https://github.com/arutkowski00/mchose-hid-bridge/tree/ad74f3e75fd5ae099b2ea4af039678c54dea73ac), complete 40% reply, descriptor and 100% record; M HUB SDK SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3` | Report 0 with a 64-byte body; `55 03 00 38 38` query and percentage at byte 8 of the `AA 03` reply. G87 V2 configuration fields cannot establish K99 charging semantics. |
| MOONDROP GM01 Pro | [MyFinder #127](https://github.com/wushuai0802/myfinder-device-battery/issues/127#issuecomment-6687188223), pinned `aa0228a` provider; saved BatteryHub `632f204c50c43355616bd32d17483c63b1bcac54`'s `moondrop.cpp`, SHA256 `c6b0bf3bb9496740487f65fa64f3d17d982d99381d8218f41f1195afe617753d` | Public prefix `4B 80 27 06 00 00 00 00 01 32` reports 50%. Firmware 1.5.3 records place all four collections on MI03, with volume and battery collections distinct. The original official HUB script was not independently obtained for this batch. |
| Rapoo VT 1851 | [FerrisPulse #76](https://github.com/Ferris-echo/FerrisPulse/pull/76); pinned `a916c657` helper SHA256 `1916585f63a15e2e1108fd5b643d65ad8d9a02f353d470de3c010085302cdee0` | Output BA followed by GET_REPORT(Input), not the ReadFile stream. The reader handles 31/32-byte layouts with a retained or replaced report ID. Configuration marker 0 is rejected; 1/2 mean discharging/charging. No EEPROM writes; receiver caches need hardware validation. |

Windows ReadFile requires the full HID capacity including the zero Report ID slot. A descriptor-proven unnumbered report loses that zero only after reception; subtracting one from the receive-buffer length is incorrect. This batch fixes the shared interrupt-input path used by VGN SE / Pro, K99 V3 and existing callers. GET_REPORT(Feature) has different returned-length semantics and retains its existing behavior.

All new paths use background polling, sanitized diagnostics and the shared sample-expiry model. Protocol tests, public-reply replays and this project's hardware acceptance remain separate. No newly supported model has been validated on local hardware; sleep caches, charge transitions and vendor-software coexistence remain pending.

All 397 local regressions, the warning-free Windows x64 release build and version/PE checks passed. A read-only probe confirmed that the existing ROG mouse and VGN V98 Pro V3 keyboard still returned valid battery data. No newly supported device was attached; their hardware acceptance remains pending.

## Version 1.4.7: domestic mouse, keyboard and headset protocols

The following protocols are implemented separately and query status only. Completed modules are reviewed and integrated into regressions while further protocol research proceeds. Registration, shared transport and state handling are integrated centrally; releases still follow acceptance batches.

| Path | Evidence | Implementation and limits |
| --- | --- | --- |
| Rapoo MT760 `24AE:1870` | FerrisPulse `a916c657c18034a30300dd08b33c6c192c86fb06` MT760 helper and PR #75; PyFlat/Device-Battery-Info PR #36 complete Windows 47% reply; Linux descriptor cross-check | One SET_OUTPUT_REPORT sends `BA A5 AA`; GET_INPUT_REPORT requests BA but returns a direct status layout. Only complete 32-byte replies are accepted. Busy `02` repeats GET only, bounded to 24 reads / 700 ms. `01 02 7F` means charging with unknown percentage. No transaction echo; receiver freshness remains unverified. |
| ATK / VXE NANO `373B:1216` | Official [ATK HUB 3.2.33](https://v3-hub.atkgear.com/) `useRemoteConfig-DYQXy-Fo.js`, SHA256 `78a7c52ed2cbd859f47ac68d535a369fa26d7775817d0b20e8c020f1418f24d3`; HaloBattery PR #350 commit `926bfc0affaf293d0c7af0bc949929568281d68f`; atk-tool PR #15 | Fixed BITMOUSE `08 7D 72 02 00 01 07 01` GET07. Input ID and capacity are independently descriptor-proven; Windows Configuration Manager identifies the USB parent for uniqueness. The public 90% record supplies a prefix, with synthetic test padding. Sequence is not interpreted as charging or voltage. |
| AULA F87ProV2 | MyFinder `aa0228a` complete 48% record; official HFD SDK SHA256 `888b4493bf0bcd25aceaefe29f4a9b0a6ee84b486cab92bbd31d531be4394d45` | GET_DEVICE_INFO page 0. Payload bytes 3/4 are low/high address bytes and both must match; byte 25 is percentage. The official FF68 catalog does not widen the observed FF60 path. Charging remains unknown. |
| AULA F99 MAX | [MyFinder #858](https://github.com/wushuai0802/myfinder-device-battery/issues/858) complete 68% two-page reply and 71% after correcting Windows report length; dedicated F99MAX 1.0.1 SDK `index-CPx-FK1k.js` | Model-specific contentSize=48, pages 0/18, each with a 32-byte body plus zero ID. This differs from the generic 56-byte, three-page reader. Firmware `0C45:800A`, wireless mode and battery must match before page two. The charge byte is range-checked as 0/1 without claiming its meaning. |
| EWEADN X87 | futao-awa/EWEADN_x87_Agreement `cea036b` `hid.js`, SHA256 `f04e7dc9c28e9584da2572cd565bf358ee762a9ad944c5a1de80300311683495`; marcelowbr/eweadn-x87 `3316109` `protocol.py`, SHA256 `2b7adb1e158fb8f2ebe1030166ffa674cddfac36b246ea9c6a306c59c38aab8c`; MyFinder #262 Windows interface record | One GET_INFO page, named X87 only after manufacturer=2 / product=5 / mode=2. Zero placeholders are rejected. Shared IDs do not establish other keyboard models. Upstream battery validation is also pending; local fixtures are synthetic. |
| MCHOSE V9 PRO 2 | The complete M HUB `main.js` snapshot and MC codec cited above; saved BatteryHub `632f204c50c43355616bd32d17483c63b1bcac54` hardware record; MyFinder `aa0228a` | Fixed `55 AA 01 00 01 00 01 0B 00`. MC type, subcommand, length and optional XOR validate `0001/0B` replies or `0004/22` notifications. The first body byte indicates sleep; the next contains a low-seven-bit percentage and charging bit. Upstream verified battery only; charging and sleep need local acceptance. |
| MCHOSE X9 | HeadsetControl `62e361b085d7716805a3bf524bc9cca3761e0e15` `mchose_x9.hpp`, documenting USBPcap comparison with M HUB; official BLK codec | Separate `6045 / MI00` identity, existing fixed FF90 query, no V9 PRO firmware request. `02` percentage, `03` charging only, `1A` clears the old value; `04` never fabricates 100%. The test's 20% is synthetic, not an X9 hardware measurement. |
| XIBERIA K06S | [MyFinder #846 hardware reply](https://github.com/wushuai0802/myfinder-device-battery/issues/846#issuecomment-7071675681) | Successful FF90 / Col03 only, fixed `55 65 00 00`; failed FF82 excluded. The public 64-byte reply supplies only prefix `55 65 00 64`, so test padding is reconstructed. Zero is indistinguishable from an echo; charging unknown. |

Shared state now retains fresh charging-only reports by replacing the entire old battery sample. The percentage is cleared, charging is displayed and the new state expires from its own timestamp. No previous percentage is copied into the new sample. Regressions cover first-read unknown charge, numeric-to-charging-only transitions, failure expiry and later percentage recovery across the main window and tray.

Hardware acceptance remains pending for every new model. Offline tests cannot establish NANO's actual topology/descriptor, receiver power-off caches, hot-plug behavior or vendor-software coexistence. The following sections record separate implementations for ATK 8K Nano, Rapoo MT560, MCHOSE K87S and AULA F108 Pro / V2C. Each has its own evidence and matching rules rather than a wider PID table for this batch.

All 440 final local regressions, the warning-free Windows x64 release build and version/PE checks passed. Configuration Manager uses the existing winapi dependency's system bindings, with no new third-party library. The latest read-only probe returned valid battery and charging data from VGN V98 Pro V3; ROG supplied no valid reply, and no newly supported device was attached. These results remain separate from protocol replay tests.

## Version 1.4.7: ATK 8K Nano, Lingbao, keyboard and headset protocols

The five paths below independently implement protocol facts. Upstream implementations, vendor assets, analysis tools and test-device data are not distributed.

| Path | Evidence | Implementation and limits |
| --- | --- | --- |
| ATK 8K Nano `373B:1178` | Official [ATK HUB 3.2.33](https://v3-hub.atkgear.com/) `AtkRadioGroup-_tra2Wmk.js`, SHA256 `20497b7e1f847bf95474971c585a690a5e3d899a4278c20fe369075b52f8cc2b`; cross-check against BatteryHub `632f204c50c43355616bd32d17483c63b1bcac54` `atk_bitmouse.cpp` | Separate Holtek path: `FF10:0001`, bidirectional Report0/65 and Feature0/9. Input opens first, a bounded drain removes queued reports, then one `00 82 00 00 00 00 00 00 00` query is sent. At most 900ms / 32 replies. In the complete 65-byte Windows frame, payload8 contains the 1–100 percentage and charge bit; payload9=0 clears the old value. Real USB-parent uniqueness is required. No version, pairing or settings command is sent. |
| Lingbao M1 Pro `088D:052F` | Two complete 65% replies in [MyFinder #171](https://github.com/wushuai0802/myfinder-device-battery/issues/171); pinned `aa0228abbc850b693620d466147b35dfe50c8dca` `lingbao.py`, SHA256 `6d243070991a115a03b468495bd03c0fd749129191e0d194de168a40c361c084`, and its protocol note | Exact product, MI01, FF00:0001, real USB parent and sole Feature0F/33 with a proven per-report byte array. One GET_FEATURE and no write transfer. The 33-byte reply requires `M536` at bytes7–10 and `LX` at13–14; byte19 must be0 and byte18 must be1–100. All flag1 placeholders are rejected without renewing old samples. Charging and byte21 semantics remain unknown. |
| MCHOSE K87S `3837:3033` | Official [M HUB SDK](https://www.mchose.com.cn/cizhou/CZ_SHARED_DATA/main.002bbdb0f17b5f7135ee.js), SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3`, type130 / GLW catalog and codec; official `audioTrayDevices-O2LjYPNA.js`, SHA256 `0ccd97c9d36a216c7c3c0c29f76fd772a25a0f4b0d7964e1a577fcd4cef60d5c` | Exact `MCHOSE K87S 2.4G` only. Command04 reads base,05 reads the active profile in56+8 bytes, then04 verifies the first six base bytes and active slot are unchanged. Four reads within3 seconds, at most32 frames per page. Config32 low7 is a level, but this model has no1%-precision flag, so official ten-point bands are shown with≈. Zero/out-of-range values are rejected; charging unknown, no configuration repair. |
| Original AULA F108 Pro `05AC:024F` | Official [AULA F108Pro Driver1.0.0.3](https://cdn.aulacn.com/keyboard/AULA%20F108Pro%20Driver-1.0.0.3.exe), installer SHA256 `62babdfdca614e5c27919d26278c845c549f3943bf2076cef0b7d56ae7e2afe8`, internal DeviceDriver.exe SHA256 `c5dda120a840de278076c405c5faf3d1137bc440de882087c77f69d61e1a3ad7`; MyFinder#187/#862 interface inventories | Static extraction and disassembly only; vendor executables were not run. Native battery function0x4357D0 calls transaction0x44F4E0 with33-byte `00 20 01 00…21`, checksum at index32. The normalized32-byte reply must echo20/01/00 and byte3 must be1–100; charging remains unknown. Exact product, MI03,FF60:0061 and sole bidirectional Report0/33;600ms/32-frame bound. The F99 MAX HFD protocol is not used. |
| FIFINE HT6 `3142:00C3` | [HeadsetControl `62e361b085d7716805a3bf524bc9cca3761e0e15`](https://github.com/Sapd/HeadsetControl/blob/62e361b085d7716805a3bf524bc9cca3761e0e15/lib/devices/fifine_ht6.hpp) documents SoundLink capture, receiver firmware1.8 and headset4.2;80% prefix in `test_fifine.cpp` | Exact MI00,FF90:0001 and sole55/64 in each direction. One `55 65 01 00` request within1 second/32 frames. `55 65 percent 02/03` means discharging/charging, with0–100 percent. Command0A is power-off; state1A in a65 reply means lost link. Both clear old battery data. A1A byte in another command cannot disconnect the headset. No RGB, sleep-setting or Flash operation. |

A successful Lingbao receiver Feature reply does not independently prove mouse RF presence or a fresh physical measurement. The shared snapshot currently has a boolean connection field; an accepted reading means the receiver query succeeded, not that sleep caches are solved. Only two complete65% replies are public. Other68%/66% and fixed100/flag1 charging observations are tabular, and related test variants are explicitly synthetic. ATK8K Nano, K87S and F108 Pro fixtures are also synthetic; HT6's public prefix is distinguished from its reconstructed padding.

This batch adds 29 module tests and extends shared silent-expiry and explicit-link-loss recovery coverage. Samples expire after 120 seconds; background publication is capped at 15 seconds for F108 Pro and 10 for the other paths. Silence and Lingbao charging placeholders never renew old values. Registration, GUI background polling and sanitized probing are connected; the combined 526-test Rust suite passed, while local hardware acceptance remains pending for all five models. Independent MT560 and F108ProV2C implementations are described in the MT560, R9 and AULA / Dareu section below. Actual descriptors and complete hardware replies remain pending; the original F108 Pro protocol is not reused for V2C.

<a id="domestic-147"></a>

## Version 1.4.7: MT560, R9 and AULA / Dareu devices

Five independent protocol paths cover MT560, V9Pro+, R9, EM901 / CM610 Pro and three AULA HFD mechanical keyboards, consolidated into version 1.4.7. The combined 526-test Rust suite passed; matching hardware acceptance remains pending. Public contributor observations and synthetic cases are identified separately. Upstream implementations, vendor resources and research files are not distributed with the source release.

### Rapoo MT560: persistent Telink8273 BB input

The official [A HUB script](https://hid.rapoo.cn/AHUB_WEB/dist/assets/js/index-cgGQw9wn.js), SHA256 `09a7906e669d75275a77ab394207bb6ab7264efa6eb6998fea3c792a31a36ecf`, maps MT560 to Telink8273_Mouse: wired `24AE:4504`, wireless `24AE:1872`, primary collection `FF00:000E`. [MyFinder #1060](https://github.com/wushuai0802/myfinder-device-battery/issues/1060) provides the wireless identity and collection inventory, without a complete BB capture or per-report descriptor. Only `1872` is enabled; MT760 and legacy VT BA queries are not used.

The official handler directly consumes Report `BB`, payload marker `B0` and at least six payload bytes. In a full Windows frame, byte 5 = 1 validates byte 6 as 0–100%; byte 5 = 0 is invalid battery data, not proof of disconnection or charging. Runtime selection requires one management collection under the actual USB parent, byte-array input IDs restricted to BA/BB, and a separately proven BB length of at least seven bytes. The parser accepts BB's own length or padding to the proven collection capacity, not arbitrary intermediate lengths. Only a read-only input handle is opened; no Output or Feature operation is sent.

The listener stays open across polls and processes at most 32 frames per batch. Once the queue empties, it publishes the newest valid sample with its original host receipt time. Silence does not renew that timestamp; samples expire after 120 seconds and background publication is capped at two seconds. Thirty seconds of silence or an immediate read failure triggers reopening with a two-second backoff, creating a short gap that may miss sparse reports. This recovery policy still needs real report-cadence testing. Native I/O cancellation waits for Windows completion, so stopping cannot be promised to have a strict deadline under a faulty driver. Receiver arrival time also does not prove a fresh physical measurement.

### AULA V9Pro+: Report 04 battery command 1A

Sources are the pinned [MyFinder `aa0228a` reader](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/providers/aula_mouse.py) and [protocol record](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/upstream-protocols.md#aula-v9pro-320f22ba-24-ghz-receiver). On 2026-11-18, three groups of nine unplugged / charging / unplugged observations reported 80→82→82% and charge bytes 0→1→0. These are upstream contributor hardware observations.

Matching is exact `320F:22BA`, `AULA-V9Pro+`, `MI_01 / FF1C:0092`, sole bidirectional byte-array Report `04/64`, and one management collection under the actual USB parent. The sole request is `04 00 00 1A` padded to 64 bytes. A full reply with that prefix uses byte 8 for 0–100% and byte 9 for charging 0/1. Parameter byte 4 is not a transaction echo, and no unproven reply checksum is imposed. Discharging zero is indistinguishable from an echo and rejected; charging zero is retained. One GET has a one-second / 32-frame bound without fallback interfaces. Topology and descriptors are rechecked under the receiver lock; unreadable same-model collections cannot be treated as absent.

### MCHOSE R9: MC battery with separate topology checks

Official [M HUB `main-DIt5vAcV.js`](https://www.mchose.com.cn/assets/main-DIt5vAcV.js), SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`, maps `3837:6013 / MCHOSE R9` to MC and Report `55`. Public [#373](https://github.com/wushuai0802/myfinder-device-battery/issues/373), [#495](https://github.com/wushuai0802/myfinder-device-battery/issues/495), [#496](https://github.com/wushuai0802/myfinder-device-battery/issues/496) and [#497](https://github.com/wushuai0802/myfinder-device-battery/issues/497) add `FF07:0222`, without a full R9 reply or per-report descriptor. THX audio MI00 is not battery-interface evidence, and V9 PRO 2's MI03 is not borrowed.

Runtime Configuration Manager lookup establishes the actual USB parent, a unique `FF07:0222`, exact product name and bidirectional `55/64` byte arrays. The fixed `55 AA 01 00 01 00 01 0B 00` request is zero-padded to 64 bytes. GET `0001/0B` and notification `0004/22` validate header, version, flags, declared length and optional XOR; unchecked frames still require a zero checksum byte. Body byte 0 = 0 clears battery data; other states use body byte 1's low seven bits for 0–100% and its high bit for charging. Query echoes never create samples.

Enumeration and report proof are refreshed under the physical-parent lock before each query. Unknown topology or unreadable same-model collections block it. After a bounded old-queue drain, one query allows at most one second / 32 frames, without handshakes or settings. Test reports and interface numbers are synthetic; the official codec does not establish local cold-start, charging, sleep or vendor-software acceptance.

### Dareu EM901 / CM610 Pro: Feature 08 and Input 09

The pinned [MyFinder `aa0228a` reader](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/providers/dareu.py), [protocol record](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/upstream-protocols.md#dareu-em901-260d1074) and [#81](https://github.com/wushuai0802/myfinder-device-battery/issues/81) document EM901's query. The analyzed driver snapshot has SHA256 `afac433fbd34b67994886a079a917c832c7f5e12cd22189e5fd84b303f56052e`. A [CM610 Pro contributor report in #725](https://github.com/wushuai0802/myfinder-device-battery/issues/725#issuecomment-6969007615) and its [reader script](https://github.com/user-attachments/files/35007352/dareu.py) establish the split collection and reply checksum.

Only `260D:1074` and product `2.4G Wireless Receiver` are enabled. Feature `08/17` sends `08 04`, fourteen zeroes and checksum `49`. Input must be a 17-byte `09 04 00` reply, with byte 6 in 0–100 and nonzero byte 7 meaning charging. If `FF02:0002` declares Input 09, its length must be proven there; a failed response never falls back to another interface. Only when that input is absent may a unique `FF01:0000` under the same actual USB parent receive replies, with manufacturer `Compx` required on both interfaces. Split replies additionally require a modulo-256 byte sum of `55`. No reply checksum is established for the same-collection path, so that rule is not imposed there.

The input handle opens first, old input is drained within a fixed count, and one Feature query is followed by at most one second / 32 reads. Sending itself is limited to 250 ms and failed sends are not retried. Enumeration and reports are rechecked under the receiver lock; unreadable same-model collections block the query. No Output, GET_FEATURE, handshake, configuration, DPI or firmware operation is included. The display name remains EM901 / CM610 Pro because the shared PID and generic product do not prove one retail model; PID `1090` is disabled. Public split-route success is upstream evidence; this project's stricter Windows report checks remain unverified on hardware.

### AULA F108 Pro V2C, F87 Pro V2C and NOVA98: HFD56

The official [AULA HUB download page](https://www.aulastar.com/aula-hub/) links [AULA HUB.zip](https://www.aulastar.com/AULA%20HUB.zip), SHA256 `f18f4bfa2a6d66f24bed8d2c17b61eff8530ebc9ad0615344f72003f5bf9db08`. Static inspection of Hub 1.1.22 covers `index-npIkQlTa.js` (SHA256 `0b97fd16dc42ad77da883c318af8924318aeebd759270c944518c8afe7278090`), `info-DZvc7M4P.js` (`536e776b9dd8daff26a0be434bf47bf9960d550714fffb0b2ec206556bfc79a8`) and `hfd-sdk.es-CwIM3CGY.js` (`888b4493bf0bcd25aceaefe29f4a9b0a6ee84b486cab92bbd31d531be4394d45`). No vendor executable or script was run.

| Exact receiver product, shared `0C45:FEFC` | Collection | Firmware identity gate in information data |
| --- | --- | --- |
| `AULA F108ProV2C Dongle` | `FF68:0061` | `38A6:273A/2749` |
| `AULA F87ProV2C Dongle` | `FF68:0061` | `38A6:2712/2738/2747` |
| `AULA NOVA98 Dongle` | `FF80:0071` | `38A6:273B/275D` |

The catalog routes Pi / qH to the same Info class and 56-byte `getDeviceInfo` reader. FF67 and FF70 are permission aliases or auxiliary collections, not query fallbacks. Each actual USB parent may have only one matching management collection. Runtime proof requires sole Report 0 in each direction and 33/33- or 65/65-byte capacities; no uncaptured MI number is assumed. Discovery and reads re-enumerate under the parent lock, reject unreadable same-PID collections and then prove the target report.

The full Windows request starts `00 AA 10 len addrLo addrHi 00 last 00`. A 33-byte capacity reads offsets 0/24/48 with 24/24/8 data bytes; a 65-byte capacity reads 56 bytes once. After removing Report 0, replies must match `55 10`, both address bytes and the complete frame length; data begins at byte 8. The SDK calls byte 2 `lenOrType` and does not establish a common response checksum or tail flag, so neither is guessed. The first page's data bytes 4/5 and 6/7 must match the firmware identities above before later pages are requested. This is a conservative catalog/field consistency gate, not a mapping confirmed by a complete physical capture.

Only data byte 17 values of 1–100 are accepted. Data byte 16 (work mode) and byte 18 (charging) lack sufficient model-specific evidence and remain uninterpreted. Sample time is the first information page's receipt time, not completion of later pages. Background publication is capped at thirty seconds and samples expire after 120 seconds. At most three pages, 600 ms / 32 frames per page, and two seconds total are allowed. Original F108 Pro, non-C F108ProV2, F99 MAX and other shared-PID models do not enter this reader. No initialization, calibration, lighting, configuration or firmware command is sent.

Cases cover identity conflicts, report lengths, invalid replies, expiry and recovery, and passed the combined 526-test Rust suite. Public V9Pro+ and CM610 Pro observations remain separate from synthetic MT560, R9 and HFD keyboard data. None constitutes this project's hardware acceptance or whole-brand compatibility.
