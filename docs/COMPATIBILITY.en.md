# Device compatibility

[简体中文](COMPATIBILITY.md)

Upstream records, validation limits and admission criteria are in [Adaptation evidence and validation](PROTOCOLS.en.md#adaptation-evidence-and-validation). Implementation, synthetic tests, upstream hardware observations and this project's hardware validation are distinct evidence levels.

## Inphic IN9

Adds receiver `248A:5B2F / MI_01 Col04 / FF00:0001`, using only 33-byte Feature Report `0C`. Input/output capacities must be zero, with a descriptor-proven sole Feature ID. Discovery publishes the device before background refresh-and-read operations validate the `M9094` model signature. Sequences are isolated per instance; each round permits one retry with a new sequence. Successful queries are spaced at least ten seconds apart, with failed rounds backing off up to sixty seconds.

Discharging samples accept 1–100%; zero is an unfilled-register placeholder. Charging bit 1 publishes charging only and hides the placeholder percentage; unknown power states clear old readings. Failed reads do not renew samples, which expire after 120 seconds. Upstream reports that receivers can still return cached data after mouse power-off, so a reply does not establish that the mouse is awake. Another `5B2E` wired device is never used to infer this receiver's charging state.

Validation replays upstream complete 81% discharging and charging-placeholder frames, plus synthetic retry, timeout and multiple-device cases. No hardware acceptance has been performed here. Sole Feature ID, other firmware, cold starts without the official driver, repeated restarts, sleep caches and official-software coexistence need hardware testing. IN9 and ZHEN have separate adapters; Bluetooth, wired bodies and DPI settings are excluded. See [protocol evidence](PROTOCOLS.en.md#inphic-in9).

## MOJHON GALE2

Adds the `E2B7:122C / MI_02 / FFAB:0001` vendor interface with 65-byte input/output, no Feature reports, and descriptor-proven sole unnumbered reports. Discovery publishes metadata first. Background reads query `E2`: only `C4_SERVER` authorizes the `04` battery query; `C4_DONGLE` clears the disconnected controller's battery. Identity is checked again before publication. Identical devices retain separate Windows instance identities.

Percentages accept 0–100. Charge flags 0/1 mean discharging/charging; other flags remain unknown. Windows transfers include a leading zero ID slot, and replies must match the frame length, register and sum checksum. Failed reads do not renew samples, which expire after 120 seconds; the polling interval is capped at ten seconds. XInput's wired status is not treated as a full battery. Its slots lack a reliable physical-device mapping and may produce a separate entry without a battery. PID `122D` and other models are excluded; wired and wireless instances are not merged.

Protocol replay and synthetic regression use upstream wired/wireless observations and charging/discharging captures. Local hardware, descriptors, power cycles, same-path reconnects, multiple receivers, transport switching and official-assistant coexistence remain unverified. See [protocol evidence](PROTOCOLS.en.md#mojhon-gale2).

## Inphic ZHEN

Adds receiver `1D57:FA65 / FF00:0001`, recorded upstream with product string `Inphic KP 8K`. Input capacity must be 5 bytes with descriptor-proven sole report `03`. IN9 uses the separate adapter above.

Passive reads accept `03 76 41 <state> <percentage>` without sending commands. State `01` means discharging, `03` charging, and other states remain unknown. Discovery publishes the device before the background battery read. Each read waits at most 5 seconds or processes 32 frames; silence does not imply disconnect. Samples expire after 90 seconds and the polling interval is capped at 10 seconds. Identical receivers remain separate.

Validation covers upstream 85% discharging and 84% charging records, malformed frames, descriptor gates, stale paths and multiple receivers. Local hardware, first-report delay, sleep/wake, charging transitions, true zero, firmware variants and vendor-software coexistence remain unverified. See [protocol evidence](PROTOCOLS.en.md#inphic-zhen).

## Pulsar X2 V2

Adds a dedicated percentage, charging and voltage path for the `3554:F508` 1 kHz receiver, replacing generic CompX probing. It requires `MI_01/Col05 / FF02:0002`, 17-byte input/output and descriptor-proven sole report `08` in both directions. This adapter does not extend wired `F507`, 4 kHz `F509` or DPI settings. The USB product name is preserved when available.

Discovery reads metadata only. Background reads check connection before and after power, validate complete replies and checksums, and reject request echoes. Explicit disconnect clears the current battery; timeouts neither imply disconnect nor renew samples. Samples expire after 120 seconds, publication is capped at 10 seconds, and identical receivers retain separate identities.

Validation covers the upstream complete 20% / 3570 mV capture, synthetic corruption, zero, unknown charge flags and multi-device lifecycle cases. Local hardware, descriptors, charging, sleep, low battery and official-driver coexistence remain unverified. See [protocol evidence](PROTOCOLS.en.md#pulsar-x2-v2).

## GANSS GS3104T-Pro

Matches the receiver with exact product string `GS3104T-PRO 2.4G`: `05AC:024F / MI_03 / FF60:0061`, 33-byte input/output and sole report ID 0. It shares the established status query with the original AULA F108 Pro; GANSS separately checks the response sum and accepts 2–100%. Charging remains unknown. Same-model receivers retain separate instance-path identities.

Evidence combines VigilWatt's public 97% capture with the receiver identity recorded in a Linux commit message. Replay, boundary and dual-device tests are synthetic local validation. Physical hardware, 1%, sleep, firmware differences, Windows descriptors and driver coexistence remain unverified locally. USB, Bluetooth and other product strings are outside this addition. See [protocol evidence](PROTOCOLS.en.md).

## HyperX Cloud Alpha Wireless

Matches original Cloud Alpha Wireless `03F0:098D / MI_03 / Col01 / FF43:0202`. Windows input/output capacities are 31 bytes, Feature length is zero, and the runtime descriptor must prove sole bidirectional Report `21`. Ambiguous management collections are not queried; identical receivers retain separate path identities. Cloud II and Alpha 2 keep their existing adapters.

Each poll uses only connection `21 BB 03`, battery `21 BB 0B` and charging `21 BB 0C`, then reconfirms connection before publication. Replies must be complete and match the current command; notifications and exact request echoes are not query replies. Connection values 1/2 mean offline/online, battery accepts 1–100, and charging accepts only 0/1. Zero percent is excluded because it cannot be distinguished reliably from a request echo; a not-charging report identical to the request also stays unknown. Confirmed charging without a valid percentage produces charging-only state, never a reused percentage.

Protocol waits share a three-second budget, with at most two seconds per exchange; handle opening, process locks and Windows cancellation cleanup are not hard real-time guarantees. Unconfirmed completion produces no fresh battery. Silence does not imply offline; samples expire after 120 seconds. The protocol has no transaction ID, so late replies from other software or prior requests cannot be fully excluded.

HyperX regression coverage includes synthetic replies, exact echoes, existing models, dual receivers, mixed brands and taskbar lifecycles. Public Windows implementations establish interface and protocol evidence; this project has no hardware validation. Actual descriptors, true zero, charging percentage freeze, mute preservation, power cycles, dual receivers and NGENUITY coexistence need hardware checks. Cloud III Wireless is deferred. See [protocol evidence](PROTOCOLS.en.md).

## LANGTU T98

Only the known 2.4 GHz receiver `1A2C:8FFF / MI_00 / FF02:0002` with product string `2.4 Wireless Device` is matched. Windows input/output capacities must both be 65 bytes, with runtime descriptor proof of sole input Report ID 0. Ambiguous management collections are not read. `1A2C:7FFF`, other models, wired USB and Bluetooth modes are excluded.

A read-only input handle receives complete 65-byte status reports: `00 AA 55 CC 33 0E <battery> 01` followed by zeros. Accepts 1–100; rejects unverified zero, invalid sentinel FF and malformed frames. Upstream observations show voltage and charging affect the value, so it is displayed as an estimate with `≈`; byte 7 is not a charging flag. Silence does not renew samples or establish disconnection. Old levels disappear after 120 seconds and valid reports restore them after wake.

The background interval is capped at 15 seconds; each receiver read waits up to 7 seconds or handles 32 frames. Identical receivers are read sequentially, so their delays add up; other brands use independent workers. Discovery and sampling send no device commands.

Evidence comes from pinned VigilWatt documentation for one physical unit and recorded 78/87/FF prefixes. Local validation uses reconstructed prefixes and synthetic tests, not this project's hardware. Actual descriptors, dual receivers, sleep/wake, power-off and official-software coexistence require hardware validation. See [protocol evidence](PROTOCOLS.en.md).

## AKG N9 Hybrid

Receiver `0ECB:20D9 / MI_03 / Col02 / FF13:0001` only, 62-byte numbered input/output reports and no Feature report. The descriptor must prove sole Input07/Output06. Ambiguous management collections are rejected; receiver instances retain separate keys.

Uses a retained handle for control `GET_INPUT_REPORT(07)` to establish idle, followed by one fixed interrupt-output battery query. Ordinary interrupt input is not a substitute. Parses declared fragments, ignores stale tails, requires the current query's successful ACK before a `0CD6` result, accepts 0–100%, and leaves charging unknown. Invalid or silent queries do not publish stale readings or guessed offline states.

Normal default polling is once per minute; user refresh, configured cadence or wake probing may shorten it. Measurements expire after 180 seconds. Idle is bounded by 1 second/20 reads; response by 3 seconds/60 reads; each native I/O wait by 500ms, messages by 1024 bytes. Cancellation cleanup is not a hard real-time guarantee.

Upstream evidence includes a full recorded 50% report and six natural polls at 100%. Local validation uses replay and synthetic cases; no project hardware validation exists. No transaction number distinguishes old results after a new ACK. Official-software coexistence, discharge, power cycling and dual receivers need hardware checks. Wired USB and Bluetooth modes remain outside scope. See [protocol evidence](PROTOCOLS.en.md).

## THUNDEROBOT ML903 NearLink Se

Receiver `369B:0234 / FFA0:0002` only. Windows input/output lengths are 64 bytes; the runtime descriptor must prove sole input `A1` and output `A0`. Synthetic upstream MI/collection numbers are not treated as hardware evidence. Ambiguous collections are rejected and separate receiver instances retain separate identities.

Sends only `A0 02`, parses battery tag `0225` at known field boundaries, and preserves unknown charging (`0226` is work mode). Invalid frames or silence do not imply offline or renew old measurements. Exchanges have a 1.5-second I/O budget and 32-frame cap, with existing safe cancellation cleanup.

Upstream contributor evidence includes a recorded 35% prefix, 27% readings and off/on observations. Local validation is replay/synthetic only. There is no complete captured descriptor; hardware must satisfy the runtime gate. Without a transaction identifier, unrelated late replies cannot be ruled out. Firmware variants, sleep, dual receivers and official-software coexistence need hardware validation. See [protocol evidence](PROTOCOLS.en.md).

## CHERRY PIXIU75

Matches 2.4 GHz `046A:01E4 / PIXIU75 Dongle` and wired `046A:01E2 / PIXIU 75`. Matching requires `MI_01/Col04 / FF1C:0092`, no Feature report, and descriptor proof of a single bidirectional Report `04`, 64 bytes including its ID. Ambiguous management collections are not queried; separate receivers retain independent samples.

Each poll sends only `04 00 00 1A 04 00 00 00…` over interrupt OUT to read the battery block. It sends no device-information, lighting, or key-map commands. Replies must match the full eight-byte header. Byte 8 supplies percentage; only byte 9 values 0/1 establish charging state. Unknown charging flags preserve a valid percentage; ambiguous zero, short frames, and invalid echoes do not renew measurements. Each exchange is bounded to 1.7 seconds and 32 frames; discovery reuses the short-lived enumeration cache.

VigilWatt publishes Windows interface evidence and readings compared with the official application. This project has reconstructed public response prefixes and synthetic tests, not hardware validation. Long sleep, real zero percent, official-software coexistence, and multiple keyboards still need hardware checks. Wired and wireless histories remain separate without a verified pairing identity. See [protocol evidence](PROTOCOLS.en.md).

## HyperX Cloud II Wireless

Provides read-only battery and charging queries for HP receivers `03F0:0696/018B`. Matching requires `MI_00 / FF90:0303` and descriptor proof of a single Report `06`, 20-byte input and 52-byte output, both including the report ID. Ambiguous control collections are not probed; separate receivers retain independent identities and samples.

Only battery `06 FF BB 02` and charging `06 FF BB 03` are sent. Connection command `01`, which can change mute state, and audio settings are excluded. Percentage is byte 7 and voltage is big-endian bytes 5/6; zero percent also requires non-placeholder voltage. Zero-filled or invalid replies do not renew measurements. Charging values 0/1/2 mean not charging, charging, and fully charged; other values remain unknown. A confirmed charging report without a usable percentage produces charging-only state, never an invented full battery.

Silence does not establish disconnection. Samples expire after 120 seconds; removal and recovery use the common lifecycle. Cloud Alpha 2 queries and existing preference/history keys remain unchanged. Upstream implementations establish protocol facts, but this project has no Windows descriptor capture or hardware test for either receiver. Descriptor eligibility, real zero percent, power-off caching, mute preservation, charging, and multiple devices still require hardware validation. See [protocol evidence](PROTOCOLS.en.md).

## Battery interfaces for four Razer receivers

Provides a read-only battery path when Windows advertises no Feature report length:

| Model | VID:PID | Charging |
| --- | --- | --- |
| Naga V2 HyperSpeed | `1532:00B4` | Not queried; unknown |
| Basilisk V3 X HyperSpeed | `1532:00B9` | Not queried; unknown |
| DeathAdder V3 HyperSpeed | `1532:00C5` | Separate query; explicit states only |
| Basilisk V3 Pro 35K | `1532:00CD` | Separate query; explicit states only |

The new path requires `Feature=0 / MI_00 / 0001:0002`, uses transaction `1F` with battery command `07:80` and optional charging query `07:84`, and transfers 91 bytes including Windows' report-ID slot. It does not query DPI, identity or polling rates, and grants no settings writes. Existing `Feature=91` interfaces retain their identity, DPI, stage and polling-rate queries and write gates; these models were not necessarily unsupported on every previous connection.

Raw battery values scale from 0–255, with zero remaining valid. Failure, mismatched echoes/transactions and timeout do not create measurements. Pinned OpenRazer code provides protocol evidence; this project has not hardware-verified these four receivers. See [protocol research](PROTOCOLS.en.md).

## SteelSeries Arctis 7+ family

Matches Arctis 7+ `1038:220E`, 7P+ `1038:2212` and 7X+ `1038:2216`, restricted to `MI_03 / FFC0:0001`. Descriptors must prove one unnumbered input/output report, with both Windows report lengths equal to 65 bytes including the zero ID slot. The existing read-only `00 B0` query is reused; no audio or configuration writes are sent.

Battery reports contain five levels, displayed as `≈0/25/50/75/100%` rather than exact percentages. Only explicit charging values 0/1 are interpreted; other values remain unknown. Offline, echoed, malformed or timed-out replies do not create or renew measurements. Identical receivers stay separate, and existing Nova preference/history keys remain compatible.

This project has not hardware-verified these models. Upstream code establishes protocol evidence; synthetic vectors only validate implementation behavior. Windows descriptors, level transitions, charging, power/wake and two-receiver coexistence still require hardware checks. See [protocol research](PROTOCOLS.en.md).

## MCHOSE V7 Pro and A7 V3 Ultra+ receivers

| Model and connection | Capability | Admission rules |
| --- | --- | --- |
| MCHOSE V7 Pro `3837:101D` | Read-only battery and known charging states | Exact product name, `FF01:0001`, descriptor-proven `4D/64` input/output reports, and one management collection per physical USB receiver |
| MCHOSE A7 V3 Ultra+ `3837:1014` | Windows descriptor-variant battery support | Exact product name and `MI_02 / FF01:0001`; the reviewed initialized-array descriptor exception is restricted to this combination |

Both reuse the existing `4D 01 01 00 00 09 00 00 08` status query. No DPI, pairing or firmware commands are sent. V7 busy responses, zero placeholders, unknown charging values and malformed frames do not create a measurement; explicit offline reports clear the reading. Unresolved V7 topology prevents access, while a failed V7 rescan preserves existing A7 candidates.

V7 evidence is upstream code and synthetic vectors; the `1014` descriptor variant has an upstream hardware report. Neither addition is hardware-verified by this project. Cold start, charging, sleep/wake, two identical receivers and vendor-driver coexistence still need validation. See [protocol research](PROTOCOLS.en.md) for pinned sources.

Logitech supports `0x2202` extended DPI reads and validated scalar writes, with simulated protocol coverage and Logitech hardware validation pending. Battery observations now distinguish live, cached and stale readings; failed queries alone do not establish disconnection. See [protocol research](PROTOCOLS.en.md) for upstream catalogs and capability boundaries.

## generic Bluetooth devices

When Windows already exposes a battery level for a connected classic or dual-mode Bluetooth device, the app can display it without a vendor protocol. It does not send query, pairing, audio or settings commands. This applies to mice, keyboards, headsets, speakers and controllers that Windows has identified, and their names and device types remain editable in the app.

Only currently connected, remembered Bluetooth devices are enumerated, with the Bluetooth address providing a stable identity. Battery data comes from a Windows PnP cached property and is accepted only for complete `BYTE` or `UINT32` values in the `0–100` range. Conflicting values from several nodes for one address, missing cache data, invalid values and disconnected devices do not create a new reading. Re-reading an unchanged cached value does not renew its measurement time, and charging remains unknown. The system Bluetooth card is merged with vendor and standard HID routes for the same physical device through the PnP Container ID; older drivers without container information use a conservative address-and-name fallback.

The background query interval is 30 seconds, and valid samples expire after 120 seconds. This implementation covers classic Bluetooth and dual-mode devices with a classic Bluetooth node. Headsets or other devices that appear only through BLE / GATT may still require a separate WinRT implementation and must not be presented as universal brand support. No local Bluetooth hardware was available for acceptance testing, so sleep caching, wake recovery and multiple simultaneous devices still require hardware verification.

## CompX power-only receivers

These receivers use the verified `FF02:0002`, Report `08`, 17-byte input/output and `08 04` battery request. The new paths send that request only: they do not perform an identity, configuration or DPI read first, and charging remains unknown.

| Scope | Capability | Evidence and limits |
| --- | --- | --- |
| VGN F1 Moba `3554:F503` | 2.4 GHz receiver percentage and voltage | A complete user reply `08 04 … 5F 00 10 05 …` reports 95% at byte 6; deep sleep was observed as an `08 03` event and readings returned after wake |
| VGN F1 Moba `3554:F506` | Wired percentage | The official VGN HUB statically maps Report `08` and command `04`; no complete replayable reply is available yet |
| DELUX M900 Ultra / GH8K `3554:FB26` | 2.4 GHz receiver percentage | A contributor measured 90% and a user retest reported 85%; only the verified receiver and `FF02:0002` are matched |
| VGN shared `3554:FB3E` | 2.4 GHz receiver percentage | User replies report 40%, 90% and 85%; the shared ID is not used to infer a specific retail model |

Windows descriptors must prove exactly one Report `08` in both directions with 17 bytes. Invalid status, command, declared length, percentage or checksum produces no new reading. Other lengths and sibling collections on the same ID do not enter this power-only path. No matching hardware was available for project acceptance testing, so cold start, charging, deep-sleep wake and coexistence with VGN HUB still require hardware verification.

## EWEADN X23 Pro

Only `04F3:026F`, the exact product name `X23 Pro Receiver`, `MI_01` Collection `04` and usage `000A:0000` are accepted. The descriptor must prove one 32-byte Input report under the resolved USB receiver, with the confirmed report ID `0` or `4`. The app opens a read-only input handle and sends no Output, Feature, pairing or settings command.

The receiver pushes a status report about every five seconds, and one poll waits at most 1.2 seconds. Byte 4 after the Windows report-ID slot carries `0–100%`; all-zero frames, wrong IDs, short frames, silence and sibling collections do not create a new sample. Charging remains unknown, and an old sample is renewed only by a valid new frame. An independent Windows implementation compared this field with the vendor app, and user diagnostics confirmed the receiver collection. No matching hardware was available for project acceptance testing, so the actual cadence, deep-sleep wake, multiple receivers and vendor-software coexistence remain pending.

## AJAZZ AK980 PRO

The wired link accepts only `0C45:8039`, product `AK980 PRO`, and `MI_02 / FF68:0061`. The 2.4 GHz link accepts only `0C45:FEFE`, product `AJAZZ AK980 PRO 2.4G`, and `MI_03 / FF60:0061`. Both require descriptors proving only an unnumbered 65-byte input/output report and identify replies by the keyboard PID `8039`.

Each poll sends one `GET_DEVICE_INFO`: `AA 10 30`, address, last-packet flag and zero data. A reply must start `55 10`, declare 19–48 bytes, echo the keyboard VID/PID `0C45:8039`, use the known work mode and carry `0–100%`. Work mode is `0` on wired USB and `2` on 2.4 GHz. The `FF67` bulk channel, 2.4 GHz `MI_04` node and mismatched echoes are excluded. Two charge samples are insufficient to classify charging or full state.

The public contributor tested both links at 100%; a normal read took about 14–17 ms. No matching keyboard was available for project acceptance testing, so discharge behavior, sleep/wake, Bluetooth mode, multiple keyboards and web-driver exclusivity remain pending.

## Veekos Shine 60

Only `3151:5026`, the exact product name `Veekos 2.4G Wireless Keyboard`, and `MI_02 / FFFF:0002` are accepted, with zero input/output lengths and a 65-byte Feature length. The descriptor must prove one Feature report: ID `0x83` when present, or unnumbered ID `0x00` for the variant. Multiple or additional reports are rejected.

One GetFeature is sent, with no Output or SetFeature. The reply must be 65 bytes and echo the report ID, with byte 1 equal to zero, byte 2 containing `0–100%`, and byte 3 equal to 100. Mismatched echoes or scale, short replies and silence create no sample, and charging remains unknown. A public user measured 61% and 48% on one receiver while coexisting with the vendor driver. No matching hardware was available for project acceptance testing, so cold start, sleep/wake, 0% and multiple receivers remain pending.

## Logitech G PRO X 2 LIGHTSPEED

Only `046D:0AF7`, Usage Page `FFA0` and Report `0x51` are accepted, and the descriptor must prove one 64-byte input and output report. One fixed read-only battery query is sent every 120 seconds; no HID++ pairing, audio, lighting or firmware command is used.

A battery reply requires Report `0x51`, kind `0x0B`, tag byte 8 `0x04`, byte 10 in `0–100`, and byte 12 `0x02` means charging. Power-off kind `0x05` with byte 6 zero clears the previous reading immediately. ACKs and power-on events are not measurements. Upstream evidence includes an open-source Linux driver and an independent hardware reading from the receiver. No matching headset was available for project acceptance testing, so cold start, wireless range, device switching and Logitech G HUB coexistence remain pending.

## VGN and MCHOSE receivers

These receiver paths have not been validated on this project's hardware.

| Scope | Implemented capability | Limits |
| --- | --- | --- |
| VGN Dragonfly3 Pro `391D:1A03` | Fixed YJX battery query, alongside the existing SE transport; Pro additionally requires the `AA` response header | Only `FF01:0010`, unnumbered 65-byte Windows input/output capacities; cable state clears battery rather than claiming charging; no DPI |
| VGN ryMs `391D:1A04/1A08/1A13/1A25/1A2B/1A41` | Receiver battery for Dragonfly3 Pro Max, Y2 V2 Pro Max, F1 V2 Pro Max, Dragonfly3 Ultra, F1 V5 Ultra and F2 V2 Ultra | Only `MI_01&Col01`, `FFFF:0002`, sole unnumbered 65-byte Feature; charging unknown; explicit sleep clears battery and later polls restore it |
| MCHOSE 4D `3837:1014/1016/1018` | Battery and confirmed charge states for the shared receiver family used by K7 V2 / A7 V3 and related models | Only `FF01:0001` with independently proven `4D/64` reports in both directions; USB product name retained, otherwise MCHOSE Mouse; no model inferred from shared IDs; no DPI |

The documented Windows array descriptor on `3837:1018`, exact A7 V3 Ultra+ / Pro name, `MI_02` and 64-byte capacities may also be accepted after `HidP_InitializeReportForID` confirms report `4D` in both local buffers. This sends no initialization command to hardware. MCHOSE 41-byte, `FF0B`, unproven descriptors and wired identities remain outside these paths. Failed reads do not renew samples. The lifetime is 120 seconds and background publication is capped at 10 seconds. No matching local hardware is available; cold start, sleep, charging, receiver cache behavior and vendor-software coexistence still need hardware validation.

## MCHOSE K99 V3 and MOONDROP GM01 Pro

- **MCHOSE K99 V3:** exact `3837:3033`, product `MCHOSE K99 V3 2.4G`, `MI_01`, `0001:0000`, unnumbered 65-byte Windows input/output capacities. The independent `55 03 00 38 38` query reads the percentage from an `AA 03` reply. Charging is unknown; G87 V2 shares a PID but uses a different protocol.
- **MOONDROP GM01 Pro:** exact `35D8:0133`, `MI_03`. Report `4B` capacities are proved separately in both directions. A unique eligible `FF02:0002` takes precedence over a unique eligible `000C:0001`; volume-input collections are not queried. Only `4B 80 27 00` is sent. Charging remains unknown; invalid or silent replies produce no new measurement.

K99 is limited to one second and GM01 to 800 ms, with at most 32 input frames per query. Public evidence includes K99 40% / 100% and GM01 50% replies. No matching local hardware has been tested; real zero, power-off caches, wake and vendor-software coexistence remain pending.

## legacy Rapoo VT receiver

Only `24AE:1851`, `FF00:000E`, `Col04`, 32-byte input/output capacities and no Feature reports are accepted, with sole Report `BA` proved by the descriptor. The reader sends `BA A5 AA 00`, waits 250 ms and uses bounded GET_REPORT(Input) for the 31/32-byte status response. It is separate from nRF54L and exposes no EEPROM, DPI or polling-rate writes.

Only success, known discharge/charge markers and `0..100` percentages are accepted; configuration replies are rejected. The shared receiver is named Rapoo VT 2.4G without guessing its paired retail model. One query per poll, no retry; samples expire after 120 seconds, with a 10-second publication cap. There is no transaction or command echo, so receiver-held stale replies cannot be fully excluded. Local hardware, sleep and vendor-software coexistence remain unverified. MT760 uses the separate path below.

## domestic mouse, keyboard and headset protocols

| Scope | Implemented capability | Matching and limits |
| --- | --- | --- |
| Rapoo MT760 family `24AE:1870` | Percentage; charging `7F` retains charging only | `FF00:000E`, sole `BA/32` in both directions, no Feature; one SET_OUTPUT_REPORT followed by GET_INPUT_REPORT, busy states repeat reads only; receiver ID does not distinguish L / Mini; zero is not published |
| ATK / VXE NANO `373B:1216` | GET07 percentage | `FF05:0001`, Output `08/64`; input byte-array ID and capacity proved separately; one management collection per actual USB parent; no charging, voltage or DPI |
| AULA F87ProV2 `0C45:FEFE` | HFD page 0 percentage | Exact `AULA F87ProV2D Dongle`, `MI_03 / FF60:0061`, unnumbered 33/33; both page-address bytes must be zero; charging unknown |
| AULA F99 MAX `05AC:024F` | Two-page HFD percentage | Exact `F99 MAX Dongle`, `MI_03 / FF60:0061`, unnumbered 33/33; firmware `0C45:800A`, wireless mode and valid percentage checked before page two; charging unknown, zero placeholder rejected |
| Original EWEADN X87 `0C45:FEFE` | GET_INFO percentage | Exact `2.4G Dongle`, `MI_03 / FF60:0061`, unnumbered 33 or 65; named X87 only after firmware manufacturer=2, product=5, mode=2; excludes X87S / V2; charging unknown, zero rejected |
| MCHOSE V9 PRO 2 `3837:6011` | MC percentage, charging and explicit sleep | Exact `MCHOSE V9 Pro 2`, `MI_03 / FF07:0222`, bidirectional `55/64`; query `0B` and notification `22` validated separately; sleep clears the old value; R9 uses the separate path below |
| MCHOSE X9 `3837:6045` | BLK percentage, charging with unknown percentage and explicit offline | Exact `MCHOSE X9`, `MI_00 / FF90:0001`, sole `55/64` in both directions; `02` provides percentage, `03` charging only, `1A` clears the old value; `00/04` are not interpreted as full |
| XIBERIA K06S `36F9:F002` | Exact percentage | Exact `XIBERIA K06S Wireless`, `FF90:0001 / Col03`, sole `55/64` in both directions; FF82 never queried; charging unknown, zero indistinguishable from an echo and rejected |

These paths query status only, with no new DPI, audio, lighting, macro or firmware settings. Background publication is capped at ten seconds and samples expire after 120 seconds. A charging-only sample replaces the previous battery and clears its percentage; it never extends an old number's lifetime. Keyboard/headset reads have time and frame limits. MT760 repeats bounded reads only when busy, while NANO rechecks the physical receiver and report capabilities before querying.

None of these devices has been tested locally. Public evidence includes complete MT760 47%, F87ProV2 48% and F99 MAX 68% two-page replies, F99 MAX 71% after correcting the buffer, and NANO 90% / K06S 100% prefixes; some test padding is synthetic. X87's upstream battery reads also remain unverified. A public successful battery read does not establish this project's charging, sleep, hot-plug, receiver-cache or vendor-software acceptance.

## ATK 8K Nano, Lingbao, keyboards and a headset

| Scope | Implemented capability | Matching rules and limits |
| --- | --- | --- |
| ATK 8K Nano `373B:1178` | Holtek percentage, charging and explicit no-link state | `FF10:0001`; sole unnumbered 65-byte input/output and sole unnumbered 9-byte Feature report, with one collection under the real USB parent. Only GetConfigData `82` is sent. The ordinary NANO BITMOUSE command is not reused, and the shared receiver cannot identify the paired retail mouse model. |
| Lingbao M1 Pro `088D:052F` | Valid wireless percentage | Exact product `Wireless-Receiver`, `MI_01 / FF00:0001`, one collection under the real USB parent, and only Feature `0F/33` with a proven per-report byte length. One GET_FEATURE per poll; no setting or Output request. |
| MCHOSE K87S `3837:3033` | Coarse ten-point battery estimates displayed as `≈` numbers | Exact product `MCHOSE K87S 2.4G`, `0001:0000`, 65-byte input/output. The active profile index is checked before and after reading; a concurrent profile change rejects the result. Charging remains unknown. G87 V2 and K99 V3 are separate paths. |
| Original AULA F108 Pro `05AC:024F` | Native-driver protocol percentage | Exact product `F108Pro Dongle`, `MI_03 / FF60:0061`, sole unnumbered 33-byte input/output. Fixed `20 01` query, charging unknown. This path excludes V2(C), F99 MAX and wired identities; V2C uses the separate implementation below. |
| FIFINE HT6 `3142:00C3` | Percentage, charging, power-off and lost link | `MI_00 / FF90:0001`, sole `55/64` in each direction. Separate `55 65 01 00` query and state interpretation; MCHOSE's unknown-percentage charging handling is not reused. No RGB or other settings are exposed. |

Lingbao replies must contain both the `M536` and `LX` fingerprints, a 1–100 percentage and flag 0. Public charging observations report a fixed 100 with flag 1, so those replies create no new reading. An earlier number expires from its original timestamp; a first placeholder creates no valid battery sample. Charging stays unknown. Successfully reading a receiver does not independently establish mouse RF presence or a new physical measurement, and sleep caches remain unverified.

Background queries are capped at 15 seconds for F108 Pro and ten seconds for the other four paths. Valid samples expire after 120 seconds and failed reads never renew them. Explicit no-link replies from ATK and HT6 clear old battery data immediately. All five paths only read state; they expose no DPI, lighting, macros, pairing or firmware settings.

Replay evidence is limited: Lingbao uses two complete public 65% captures; HT6 uses a public 80% capture prefix with reconstructed padding; other frames are synthetic protocol constructions. The public K87S 64% user result contains no complete reply, and the F108 Pro command comes from static analysis of the official driver. Neither constitutes this project's hardware acceptance. No matching model was tested locally; actual descriptors, charging transitions, cold starts, sleep recovery, multiple receivers and vendor-software coexistence remain pending.

## MT560, R9 and additional AULA and Dareu devices

Hardware acceptance remains pending for the following implementations.

| Scope | Implemented capability | Matching and limits |
| --- | --- | --- |
| Rapoo MT560 `24AE:1872` | Continuous reception of percentages from `BB` status reports | Only `FF00:000E`, with one management collection under the actual USB parent. Runtime proof establishes BB's own length and the collection capacity, without assuming 64 bytes. Input only; no BA or other command. Charging, DPI and mouse RF presence are not inferred from other fields |
| AULA V9Pro+ `320F:22BA` | Percentage and charging | Exact `AULA-V9Pro+`, `MI_01 / FF1C:0092`, sole bidirectional byte-array Report `04/64`; battery command `1A` only. Discharging 0% cannot be distinguished from an echo and is rejected; charging 0% is retained |
| MCHOSE R9 `3837:6013` | MC percentage, charging and explicit sleep | Exact `MCHOSE R9`, `FF07:0222`, bidirectional `55/64` byte arrays. Uniqueness uses the actual USB parent without borrowing V9 PRO 2's fixed interface number. GET `0B` and notification `22` are validated; sleep clears old battery data |
| Dareu EM901 / CM610 Pro `260D:1074` | Percentage and charging | Product `2.4G Wireless Receiver`; Feature `08/17` query and Input `09/17` reply. Same-collection `FF02:0002` takes priority. Split `FF01:0000` input must belong to the same USB parent, and both interfaces must report manufacturer `Compx`. The shared PID does not identify one retail model; `1090` is not enabled |
| AULA F108 Pro V2C `0C45:FEFC` | Percentage from 56-byte HFD device information | Exact `AULA F108ProV2C Dongle`, `FF68:0061`; first-page firmware identity must be `38A6:273A/2749` |
| AULA F87 Pro V2C `0C45:FEFC` | Same as above | Exact `AULA F87ProV2C Dongle`, `FF68:0061`; first-page firmware identity must be `38A6:2712/2738/2747`. Separate from the existing F87ProV2D path |
| AULA NOVA98 `0C45:FEFC` | Same as above | Exact `AULA NOVA98 Dongle`, `FF80:0071`; first-page firmware identity must be `38A6:273B/275D`. The C-series FF68 and auxiliary collections are not used |

All three HFD keyboards require one management collection under the actual USB parent and runtime proof of sole unnumbered 33- or 65-byte reports in both directions. The complete 56-byte information block is read according to that capacity; a mismatched first-page identity stops later pages. Firmware restrictions are a conservative consistency check between the official catalog and SDK fields, not a hardware-confirmed reply mapping. Only 1–100% is published; charging and work-mode semantics remain unknown. F108ProV2 without the C suffix, other shared-PID models and auxiliary collections are excluded from this path.

These paths expose no DPI, button, audio, lighting, pairing or firmware settings. MT560 publishes at most two seconds apart while retaining each sample's original receipt time. The three HFD keyboards have a thirty-second publication cap; other paths use ten seconds. Samples expire after 120 seconds, and failed reads or reopened handles do not renew them. Actual report cadence, silence recovery and vendor-software coexistence still require hardware checks.

Public evidence includes V9Pro+ 80% / 82% unplugged–charging–unplugged captures and the CM610 Pro split-collection report. These are upstream contributor observations. MT560, R9 and the three HFD keyboards primarily use protocol-derived synthetic test data, not captures from local hardware. Original public sources, report formats and remaining limits are recorded in the [protocol notes](PROTOCOLS.en.md#domestic-147).

## VGN / VXE 20-byte keyboard receivers

Five 2.4 GHz identities are included: `258A:024F/027F/027B` and `391D:2A01/2A02`. Each requires a unique `FF02:0002` management collection and descriptor-proven bidirectional Report `13/20`. The request is `13 4A 01`, sixteen zeroes and checksum `5E`; replies validate full length, header, additive checksum and 0–100%. Only `258A:024F` uses the confirmed full-frame byte6 `0x10` charging bit; the others keep charging unknown. Shared `3554:FA09` remains handled by the separate MCHOSE backend.

Queries allow at most 2.5 seconds, background publication is capped at ten seconds, and samples expire after 120 seconds. Failed reads retain the old timestamp; later valid reports restore readings. Existing wired V98 Pro V3 `258A:024E` hardware results do not validate these five wireless identities. Actual descriptors, sleep, 0%, charging transitions and vendor-software coexistence remain pending.

## EWEADN S9 PRO 8K

Limited to `372E:1026` on `FF00:0001` or `FF06:0001`. Within one USB interface group, exactly one collection must have a runtime descriptor proving Feature Report `03` is 64 bytes. A 64-byte collection maximum alone is not accepted, and other Feature IDs may own a larger report. Zero or multiple proven collections cause no device open and no request.

The reader sends one 64-byte Feature request `03 20 50 00 00 4F 81` plus 57 zeroes; checksum `20` covers the payload without the Report ID. It waits 100 ms, then performs one GET_FEATURE of Report `03`. UUID, wireless-online, reset, pairing, configuration, macro, display and firmware commands are unreachable. Both the user-verified 16-byte reply and the descriptor-backed 64-byte reply are supported. Byte 6 bits 0–6 are a 1–100 percentage and bit 7 is charging; the short form additionally requires presence byte 5 to equal `01`. Wrong headers, empty packets, request echoes, out-of-range levels and silence create no fresh sample.

User issue #315 confirmed the S9 PRO 8K 16-byte format, 23%–98% readings and charging transitions on Windows x64. This project has no independent S9 PRO hardware. The same `372E:1026` is also used by other S7/S9 catalog entries, so the current name is not a runtime paired-model proof. Real descriptor shapes, 64-byte replies, sleep caching, wake-up and coexistence with EWEADN HUB remain pending.

## EWEADN S9 (M156)

Limited to `089D:062F / FF01:0010`, a 33-byte output and a 16–34 byte Windows input capacity. A descriptor must prove one unnumbered Report 0 in each direction. Wired `088D:062E`, `FF00:0000`, M302 and other product names are excluded. ReadFile receives the complete input capacity before removing the zero Report ID; the upstream fixed 33-byte buffer is not assumed for every firmware.

The fixed request is `00 30` plus 31 zeroes; its checksum covers bytes 5–31 and is zero. A reply must be 15–33 bytes, begin `30 00`, declare enough data for bytes 4–14, contain ASCII `M156` at bytes 4–7, put a 0–100 percentage at byte 13 and the connected value `01` at byte 14. No charging field is established, so charging remains unknown. `30 02`, M302, passive `D1`, malformed lengths and other status values create no reading.

Packet facts were independently implemented from HaloBattery PR #352 under MIT. Protocol regressions do not establish S9 hardware compatibility. Real caps, comparisons such as 70% / 69%, sleep, removal, cold start and vendor-driver coexistence remain pending.

## MCHOSE G87 V2

`3837:3033` is shared by multiple keyboards, so this path requires the exact product string `MCHOSE G87 V2 2.4G`, collection `0001:0000` and 65-byte input/output capacities. A descriptor must prove that input declares one report ID (which may be zero) and output declares only Report 0. Multiple matching collections in one interface group are refused. Other MCHOSE mice, QHW/BY keyboards, K99 V3 and ordinary keyboard/mouse collections do not enter this reader.

Only three read-only configuration requests are sent: command `04` reads base data, then command `05` reads the active profile in 56+8 byte segments. Base data maps the current index to a real profile slot; a valid `0..3` permutation is honored, while the documented legacy direct index remains supported otherwise. Complete configuration byte 32 encodes a 0–100 percentage in bits 0–6 and charging in bit 7. Replies must match the `AA` marker, command, requested length and offset. Reply checksum and other header flags remain unconfirmed and are not guessed; failure never becomes 0%.

No local G87 V2 hardware validation exists. Real descriptors, sleep caching, cold start, reads during profile changes and coexistence with MCHOSE software remain pending.

## Flydigi APEX5

Limited to `37D7:2501 / FFA0:0001`, 32-byte input/output and Reports `04` / `03`. The only request is the fixed GET_INFO frame `03 5A A5 01 02 03` plus 26 zeroes. IMU, rumble, trigger, lighting, mode and configuration commands are unreachable. A reply must match the `04 5A A5 01 01 00 80` header and mode `01/02`.

Byte 12 bits 0–3 encode five battery tiers, displayed as `≈` estimates of `0/20/40/60/80/100%`. Bits 4–7 accept only 0 or 1. A high nibble of 1 is a charging sentinel; it produces neither a sixth tier nor a fabricated percentage and shows charging only. Other tiers or charge values are rejected. No local APEX5 was available; real descriptors, sleep, charging transitions and desktop-software coexistence remain pending.

## GameSir Nova / Cyclone 2

The Nova family is limited to `3537:1098/100F/1040` and an unnumbered `FF7A` input/output collection. The app sends `00 01 01`, ignores `F4` state frames for up to 800 ms, and accepts only replies beginning with `01` whose byte 18 is 0–100 or charging sentinel `FF`; `FF` reports charging without inventing a percentage.

Cyclone 2 is limited to `3537:100B / FFF0:0040`, 64-byte input/output and Reports `12/0F`. The fixed read-only request is `0F 03`. Reply Report `12` accepts only 0/1 at byte 35 and a 0–100 percentage at byte 36. Both paths send only device-information reads; no rumble, trigger, lighting, button or mode command is reachable.

No matching GameSir controller has been validated locally. Windows descriptors, sleep/wake, wired/2.4 GHz transitions, multiple identical units and GameSir Nexus coexistence remain pending.

## ATK N9 V2 ULTRA / MCHOSE V9 headsets

The FF90 `55:65` headset backend supports `373B:12D6` ATK N9 V2 ULTRA and limits ATK input capacity to the descriptor-proven 22–64 bytes. ATK states `02/03/04` mean discharging, charging and full. State `04` preserves the reported percentage instead of forcing 100%.

MCHOSE V9 Turbo+ remains limited to `3837:600A / FF90:0001` and 64-byte input/output. State `02` decodes a 0–100 percentage, `00/03` means external power with unknown level, and `1A` explicitly means offline. Silence does not mean offline. The `3837:6008` lighting device and all other identities remain excluded.

`291D:385D` MCHOSE V9 PRO now requires the same physical receiver's `FF22:0001` descriptor to prove Feature `AA/64` before reading firmware; confirmed versions below `0008` block the battery query, while unknown versions retain the conservative battery fallback. All new headset paths still need matching hardware acceptance.

## AULA SC380 PRO

Limited to `258A:0138` receivers, `FF02:0002`, Windows Feature capacity 520 and only report ID `09` in a unique management collection. Each poll checks the six-byte model signature `04 00 00 00 00 2E` and online response; the receiver ID alone cannot establish the mouse model. Wired `0136`, other AULA mice and keyboards are excluded.

Battery exposes exact percentages from 1–100; charging is unknown. A zero payload is indistinguishable from a query echo on this path, so it establishes neither 0% nor explicit offline. Failed queries retain the original sample age, expiring after 120 seconds; the background wait is at most ten seconds, and new valid replies restore the number. Battery and settings reads can succeed independently.

Actual 1–6 stages contain X/Y values from 100–12000 in steps of 100. Original order and duplicate values are retained without fabricated defaults. Switching is bound to the complete 400-byte configuration, rechecks identity and configuration before writing, changes only the active-stage nibble and preserves buttons, lighting, rate and all other bytes. Full readback confirms the result. Lost send completions are not retried or blindly rolled back. Numerical edits, software presets, stage-count and rate writes are not exposed.

Evidence comes from the official model configuration and upstream Windows hardware documentation; local tests use synthetic reports, with no matching hardware available. Descriptor behavior, real transport, sleep caching, physical switching, persistence and vendor-software coexistence remain unverified. Without transaction IDs, cross-process races and same-model replacement cannot be fully distinguished.

## AULA F75 / F87 PRO keyboards

The existing `3554:FA09` shared-receiver backend recognizes F75 model code `03 00 00 00 00 CD` and F87 PRO code `03 00 00 00 03 E3`. The interface still requires `FF02:0002`, 20-byte bidirectional Report `13` and one management collection. F87 is excluded because its official wireless catalog entry uses `FF00:0001`, which conflicts with this receiver family. An unknown code is never named as AULA from the receiver ID alone.

AULA places its identity checksum at complete-frame byte 8 and sends no equivalent checksum in the battery request; MCHOSE retains its existing framing. A unified backend caches the selected protocol for that channel for 120 seconds, preventing two background readers from competing for one receiver. Every AULA battery sample requires the same six-byte model both before and after the read. Only percentages 1–100 are accepted. Charging/full nibbles must be 0 or 1, unknown flags and sleeping zero placeholders produce no fresh sample, and the AULA UI rule allows charging to remain true at 100%.

Official catalogs and static scripts provide the protocol evidence, while tests use synthetic frames; no local F75 / F87 PRO was available. Windows SET_REPORT transport, receiver caching, wake recovery, switching between AULA and MCHOSE devices on one receiver and vendor-software coexistence remain hardware checks.

## 8BitDo Pro 3 controller

Limited to D-input mode `2DC8:6009`, standard Gamepad `0001:0005`, and a descriptor proving that every Input Report ID is `04` with a 15–64-byte input capacity. The adapter opens a read-only input handle and sends no Feature, Output, rumble, lighting, mode or initialization command. XInput mode uses a different PID and does not enter this private-report parser.

Each read receives at most 64 reports within 400 ms and accepts only an Input Report `04` with the declared capacity. Byte 14 contains an exact percentage in bits 0–6 and a charging bit at bit 7. A 0% value is retained; values 101–127 are rejected. At 100%, the controller is treated as charged rather than actively charging. Background publication is capped at ten seconds, failed reads do not renew the sample time, numbers expire after 120 seconds and a new report restores them automatically.

Evidence comes from a pinned SDL driver and public MyFinder records; a reporter also confirmed Pro 3 sending this report over D-input Bluetooth without Steam. No matching local hardware was available. Windows descriptors, 0% / 100%, charging transitions, sleep recovery, multiple controller instances and coexistence with Steam or vendor software remain pending.

## VGN Dragonfly3 SE

The SE path matches receiver `391D:1A01 / FF01:0010`, with a descriptor proving unnumbered input and output Report 0 at 65-byte Windows capacity and one eligible collection per USB interface group. The same transport supports Pro `1A03` as described above; the diagnosed `FF05:0002` collection, wired `1001` and other VGN protocols do not use this reader.

The request is the fixed `00 55 30` plus 62 zeroes. A reply is received using the 65-byte Windows capacity, then the zero Report ID is removed to obtain a 64-byte protocol payload. Payload byte 1 must equal `30`. Byte 9 of 1 means wired; it publishes no battery and explicitly clears an old wireless percentage. When byte 9 is zero, byte 8 is an exact percentage from 0–100. Other values, request echoes, short frames and foreign commands create no fresh sample. No charging field is established. A query is limited to one second, background publication to ten seconds, and failures do not renew the old sample.

Evidence comes from pinned VGN HUB resources and the public MyFinder implementation; tests use synthetic field constructions. No local Dragonfly3 SE was available. Windows descriptors, receiver sleep caching, 0%, transport transitions, physical accuracy and vendor-software coexistence remain pending.

## HECATE G3M PRO

Limited to `320F:706E / FF1C:0092`, with a descriptor proving that both directions contain only a 64-byte Report `04` and one eligible collection per interface group. The fixed request is `04 20 00 1A 06` plus 59 zeroes. Replies require 64 bytes, Report `04` and command `1A`; status bytes `FE/FF` are rejected and percentages from 0–100 are accepted. An exact request echo is not a reading.

No charging field is established, so charging remains unknown. The adapter reads no DPI, lighting, online session or other configuration and never falls back to the VGN keyboard protocol sharing this vendor ID. Each query is limited to one second and failures do not renew the old sample. Tests use synthetic protocol fixtures and no local G3M PRO was available, so descriptors, sleep, 0%, charging and vendor-software coexistence remain pending.

## AJAZZ AJ179 PRO

Restricted to `3151:402D` receivers and `3151:4026` USB, `FFFF:0002`, Windows Feature capacity 65 and only report ID 0 in a unique management collection. Several brands share these IDs; only mouse identity `1851` is named AJ179 PRO and receives configuration access. Other valid identities retain a neutral name and receiver battery only. Bluetooth `402C`, newer controllers and other AJAZZ protocols are excluded.

Receiver percentage is exact; charging remains unknown. USB exposes settings without inventing battery or charging state. Explicit offline clears old readings; failed queries preserve sample age, with expiry after 120 seconds and a maximum ten-second background wait. A valid new reply restores the number. Receiver-side sleep caching still needs hardware testing.

The current one-of-eight profile is queried before its complete X/Y table; profile 0 is not assumed or selected. Disabled slots retain their original indices, including internal gaps, and duplicate values are not merged. The current vendor catalog permits reads of 50–32000 in steps of 50. Published hardware write evidence extends only to 26000, so any enabled axis above that limit makes the whole table read-only. Switching preserves all eight stored slots, colors and reserved fields in the 64-byte block, changing only the active index and protocol header before full readback. Numerical edits, custom presets, stage-count changes and polling-rate writes are not exposed.

Identity and complete configuration are rechecked before writes. A lost send completion is not retried or blindly rolled back; success requires matching readback. There is no transaction ID or system-wide exclusive lock. Vendor-software races, same-model replacement, USB/receiver deduplication and power-cycle persistence remain unresolved hardware checks.

Published original replies provide offline decoding evidence; other boundary and write cases use synthetic reports. No local AJ179 PRO was available to validate Windows Feature transport, wake or physical stage switching.

## Dareu and MCHOSE mice

These adapters have protocol regression coverage, but no matching device has undergone this project's hardware acceptance. Catalog entries and upstream hardware reports are not local compatibility certification.

| Scope | Battery / identity | DPI |
| --- | --- | --- |
| Dareu `260D:1114` receiver | Runtime CID/MID distinguishes A950 PRO, PRO Mg and A950G; valid percentage reads | Actual 1–5 stage reads/switching on identified PAW3395 models |
| Dareu `260D:1175` receiver | Distinguishes A950 AIR, A950 GM and LM180T | Identified PAW3395 AIR/GM models can switch stages; LM180T has no DPI settings yet |
| Dareu A950 PRO Mg cable `260D:1117` | Requires the same report shape and runtime identity; unverified wired path | Same guards; host PID alone never enables writes |
| MCHOSE A7 V2 Pro / Pro+ / Ultra / Ultra+, L7 Pro / Pro+ | Restricted Feature endpoints read the mouse's own PID, percentage and charge; shared receiver IDs never select the name | Actual enabled-stage reads; Ultra+ and L7 Pro+ can switch existing stages, other models are read-only |
| Older MCHOSE A7 Ultra | Restricted `5253:1020/1011` paths; some receivers are documented to return no 06 battery data | Reads require valid identity/config; no writes |

Dareu requires `FF02:0002`, bidirectional Report 08, 17-byte reports. MCHOSE requires `FF01:0001`, Feature 11/12 and 65-byte capacity; exact host PIDs are listed in [protocol notes](PROTOCOLS.en.md). Bluetooth, old A5, firmware collections and unknown sensor layouts are excluded. MCHOSE A7 V3 uses the separate 4D battery path described above and does not inherit Feature-mouse DPI write capabilities. Missing descriptor evidence never triggers an alternate write path.

Both adapters preserve raw stage indexes and actual enabled counts; no default 400/800/1600 list is substituted. Identity and full configuration are checked before writes and results are read back. Dareu changes only the two-byte active-stage record. MCHOSE echoes the original full configuration, changing only the live connection's stage nibble and preserving rate, buttons, macro mappings and all other bytes. Arbitrary DPI/software-preset value writes are not offered for these adapters yet.

MCHOSE uses published L7 Pro+ raw battery/config frames, corrects the upstream connection-bit interpretation and requires two consecutive matching replies. A wireless-zero sleep placeholder cannot enable settings or renew battery age. Dareu charge enum 2 stays unknown due to conflicting evidence. Full sleep cycles, official-driver coexistence, Windows Feature/caps, physical stage effect and multiple receivers still need hardware validation; stale receiver caches and last-moment cross-process races are not claimed solved.

## Edifier Halo Space

Only `2D99:E049` wireless receiver, `FF14:0001`, with 64-byte input/output is eligible. Descriptors must establish input Report `2F` only and output Report `2E` only, with one management collection per Windows USB interface group. There is no fallback to `FF13`, multimedia collections, other Edifier models or Bluetooth.

The only query is battery `D0`. No MAC, firmware or internal device-name query, audio/ANC/lighting change or firmware operation is sent. Existing volume, output-device and microphone controls continue through Windows audio APIs. Replies require `2F BB EC D0 00 01`, exactly 64 bytes, a valid sum over the meaningful wire bytes and an unmasked percentage in 0–100. Trailing scratch data is ignored.

Charging is unconfirmed and stays unknown. Queries allow 1.2 seconds, with a ten-second maximum background wait. Silence neither confirms disconnection nor renews the old sample; numbers expire after 120 seconds and recover on a fresh response. The protocol lacks transaction IDs, so TempoHub coexistence and cached receiver replies require hardware tests. Identical headsets retain separate instance identities.

The independent implementation uses FerrisPulse's published interface, 35% reply prefix and checksum evidence, with synthetic trailing test data. No matching headset was available locally. Descriptors, actual communication, charging-cable transitions, sleep recovery and official-software coexistence remain unverified.

## MCHOSE keyboards

M HUB and CZ SDK were inspected separately; their keyboard protocols are not interchangeable. Queries require the matching collection, input/output capacities and actual report IDs. Missing descriptor evidence or multiple eligible controls in one Windows USB interface group prevents I/O.

| Scope | Required endpoint | Battery and identity behavior |
| --- | --- | --- |
| G98 V3 2.4G | `3837:1015`, exact product `MCHOSE G98 V3 2.4G`, `FF80:0001`, Report `4D` only in both directions | Independent QHW command `0904`, matching sequence, percentage 0–100; charger 0/1/2 means discharging/charging/full |
| Shared BY receiver | `3554:FA09`, `FF02:0002` or `FF04:0002`, 20-byte input/output with Report `13` only | Generic receiver or MCHOSE product names prompt six-byte model identification; an unknown model retains its product or a neutral 2.4G keyboard name, with valid percentage and unknown charging |
| Selected MCHOSE BY receiver | `41E4:2001`, `FF04:0002`, the same 20-byte Report `13`, exact product `2.4G Wireless Receiver` or `MCHOSE 2.4G Wireless` | Requires an M HUB model-table match before battery; missing/unknown identity stops the query. Cross-interface FF02 sibling pairing is not implemented |
| Existing Z75S | `258A:01BB`, `FF02:0002`, 20-byte Report `13` | Retains complete Z75S identity and existing battery checksum checks; ordinary keyboard input collections are excluded |

The BY model-code table contains G87, K99 / V2, G98 / Pro / V2, X75 / V2, G75 Pro / V2, UT98, Z75 / Z75S, KX75, GX87 / V2, K87 and G87 V2. These identify replies on qualifying connections; **they do not establish hardware compatibility for 18 models**. The separate `3837:3033` G87 V2 GLW protocol is not implemented by this table.

FA09 retains the captured Windows SET_REPORT transport. Z75S and the new `41E4:2001` path use interrupt OUT, without retrying on another endpoint. BY sends only model `05` and battery `4A`, allowing up to 2.2 seconds per query. Identification is renewed each poll, so failure cannot permanently block recovery. Instance identities stay separate; wired/wireless deduplication is not implemented.

Shared BY replies follow M HUB's single-packet header. Byte 4 is not presumed to be a length and the trailer does not inherit another firmware's checksum rule. Recognized models may also supply `13 0A ... 05` battery notifications in the current bounded query window. Unknown models use only `4A` and no MCHOSE charging interpretation. Zero may be a sleeping-receiver placeholder and is suppressed on this BY wireless path. For recognized models, charging requires high nibble 1 and percentage below 100, following the official UI; a full flag never changes the measured percentage. VGN V98 Pro V3 retains its separate strict decoder.

G98 V3 uses separately proven descriptor capacities for input/output and a 1.5-second query window. The response checkCode algorithm is unconfirmed; actual report ID, complete length, fixed header, sequence and field ranges are validated without inventing a checksum rule. No heartbeat, occupancy declaration, lighting or settings writes are sent. Receiver-cached sleep replies, independent cold-start reads, padding and full-charge transitions need hardware validation.

All these paths use a ten-second maximum background wait and 120-second sample lifetime. Failure does not renew a sample; fresh background replies restore the display. Published frames provide offline regression evidence, with other vectors synthetic. None of these keyboards has undergone this project's hardware acceptance.

## shared VGN / VXE keyboard receivers

Matches `320F:5088`, `FF1C:0092`, 64-byte input/output receiver queries. VGN HUB lists S99, V98 Pro / V2, N75 Pro and VXE75 / V2 in this protocol family. Exactly one management collection per Windows USB interface group is required, and both input/output descriptors must contain only Report `04`. Wired `320F:5055` is excluded; the existing V98 Pro V3 backend is separate.

The shared ID cannot establish a brand or model. USB product names are retained, with a neutral 2.4G keyboard fallback. Rainy75 can share the same ID and generic product string, so it may receive this read-only query; it is not claimed as hardware-validated. Only matching battery replies produce a reading. Identical receivers retain separate interface-instance identities.

The sole request is `04 00 00 1A 06` with zero padding. Replies require exactly 64 bytes, the same five-byte prefix and a percentage in 0–100. A zero reply identical to the request is an echo, not a reading. Charging and response checksums are unconfirmed. Sleep replies starting `04 00 00 FF 06` do not refresh the sample or establish disconnection. Old readings expire after 120 seconds, and a later valid background response restores the display.

The published receiver response took about 1.03 seconds, so each query allows 2.5 seconds with a ten-second maximum background wait. An independent backend worker avoids blocking V98 Pro V3 queries. The protocol has no transaction ID, so concurrent VGN HUB queries cannot be reliably attributed and need hardware testing.

No matching keyboard has undergone this project's hardware acceptance. The published 93% reply prefix and sleep observations inform protocol tests; their trailing test padding is synthetic. This does not validate every listed model.

## Lofree HYZEN

Only receiver `388D:0025`, collection `FF1C:0092`, with 32-byte input/output is matched. Wired `0024` and single-mode `0029` are not battery-query targets. A present `0024` suppresses receiver polling, following the official driver's wired exclusion. Without a proven cable/receiver pairing identity, this may also suppress a different HYZEN receiver when multiple keyboards are attached; hardware work remains.

Only Report `04` START, online `AA` / battery `1A`, and END exchanges are used. Data must have a nonzero payload length no greater than 24, offset 0, and matching command/phase. Zero charge requires a valid data payload; zero-length echoes cannot produce battery. Online states are limited to 0/1 and percentage to 0–100. Charging remains unknown.

END is attempted even after START or data failure; unconfirmed END prevents publishing a new number. Connection is checked before and after battery, with explicit offline clearing old charge. Failures do not renew sample age and cached values expire after 120 seconds. Each transaction waits at most about three seconds; a normal sample includes connection, battery and reconfirmation, without blocking other brand workers.

The official web driver was cross-checked with MyFinder; no matching keyboard is attached locally. Windows transport, payload lengths, charging-cable transitions, long sleep, multiple units and vendor-software coexistence still require acceptance.

## DualSense gamepads

The adapter matches Sony `054C:0CE6` DualSense and `054C:0DF2` DualSense Edge, only on `0001:0005` Game Pad collections with 64-byte USB or 78-byte Bluetooth input descriptors.

- USB Report `01` carries battery status at offset 53; Bluetooth Report `31` uses offset 54 and requires CRC32 with prefix `A1`. A padded Bluetooth minimal Report `01` is never parsed as USB battery data.
- Bands 0–9 display ≈5%–≈95%; band 10 or the fully charged state displays ≈100%. Charging stays separate. Voltage, temperature and charging errors never become 0%, and invalid bands are not clamped to full.
- Input handles request read access only. Observed Bluetooth minimal mode prompts a calibration Feature `05` GET to request extended reports, with its `A3` CRC checked. Battery still comes from independently validated input reports; no rumble, lighting or settings output is sent.
- The background polling wait is capped at two seconds. Timeouts and faults do not renew the last sample; numbers disappear after eight seconds and return on a fresh valid report. Identical controllers retain separate Windows interface identities. Cross-transport deduplication of one controller connected over USB and Bluetooth is not implemented.
- Diagnostics retain protocol identifiers, lengths, timing and outcomes, without button, stick, touch or unique identity data. Estimates cannot trigger exact-percentage notifications.

Protocol regressions cover USB / Bluetooth layouts, CRC, malformed reports, minimal mode, instance isolation, expiry and recovery. Neither controller has undergone this project's hardware acceptance; hardware transport, sleep recovery and Steam / DS4Windows coexistence remain unverified.

## XInput gamepads

Windows' system XInput 1.4 provides four slots through `XInputGetState` and `XInputGetBatteryInformation`. Support depends on the current Xbox-compatible driver and connection returning battery data, not on a whole-brand claim. The adapter does not fabricate HID paths, VID/PID or physical model names, and sends no vibration or settings commands.

Only known wireless battery types supply usable levels. Empty / Low / Medium / Full display as ≈0% / ≈20% / ≈50% / ≈100%. These are application display representatives: Windows defines no percentage boundaries or charging flag. Wired and unknown battery types clear previous numbers immediately. Failed queries retain the original measurement timestamp, with expiry after eight seconds. A confirmed disconnect clears that slot's battery before another controller can reuse it.

Names, tray selection and order are saved by slot. XInput exposes no stable physical identity, so preferences follow the slot when controllers change or Windows reassigns it; a replacement between polls without an observed disconnect cannot be identified. Virtual XInput controllers can appear alongside their physical HID device, and reliable cross-protocol deduplication is not implemented. Users can select which cards appear in the tray.

XInput battery grades, wireless / wired transitions, sleep recovery and different drivers still need hardware acceptance.

## Sony INZONE headsets

The adapter matches only Sony `054C:0EBF` H5 on collection `FF04:0002`, and `054C:0FA8` H9 II on `FF04:0001` / `MI_05`, with 64-byte input and output reports. H3, H7, original H9, Buds, PS5 mode and firmware-update interfaces are outside this adapter's scope.

- Only battery GET queries are sent. Volume, ANC, microphone and firmware settings are not changed. Existing audio controls in the main window and tray continue to use Windows audio APIs.
- Replies must match the report ID, fixed battery payload length, HCI direction, Sony marker, headset-to-PC address, event, transaction and checksum. Transaction IDs 0/1 are reserved; old replies and unsolicited notifications cannot satisfy a current query.
- Values 0–100 are device-reported percentages without ≈. Charger bytes 0/1 mean not charging / charging; other values remain unknown. Full charge does not imply charging. Battery `FF` explicitly marks offline and clears prior numbers; 101–254 are invalid.
- Background polling waits at most ten seconds. Failed queries retain the original sample time, with numbers hidden after 120 seconds and restored by a valid response. Identical receiver models retain separate Windows interface identities.

Protocol tests cover a published H5 notification, transaction wraparound, echoes, unrelated events, checksum errors, valid zero, offline and wake. UI-state regressions cover expiry and recovery. Neither headset is attached locally; USB communication, power recovery, receiver mode changes and INZONE Hub coexistence await hardware acceptance.

## HyperX Cloud Alpha 2 Wireless

Only the `03F0:08BE` control station's `FF13:FF00` collection with 64-byte input/output reports is matched. The `03F0:0ABE` audio function, `FF13:0001` sibling, original Alpha, and Cloud II / III protocols are excluded. Only connection and battery queries are sent; audio settings are not written.

- `52 01` / `53 01` queries connection; `50 02` / `51 02` reads percentage, independent charging state and little-endian voltage. Percentages 0–100 remain exact device reports. Unknown charger bytes and 0 / `FFFF` voltage stay unknown, without deriving charging from voltage bits or full charge.
- Length, report and subcommand must match. Echoes, ACKs, audio replies and unsolicited notifications are ignored; a matching NACK stops the query. Only connection values 0/1 are accepted.
- Connection is checked before and after battery. A power-off during the read cannot publish station-cached charge. Failed reconfirmation cannot renew the prior sample; explicit offline clears it. Polling waits at most ten seconds, cached samples expire after 120 seconds, and identical stations keep separate interface identities.
- The protocol has no transaction number or checksum. Same-command replies from another process cannot be reliably distinguished. Filtering and bounded queue draining do not replace NGENUITY coexistence testing; cross-mode USB deduplication is not claimed.

Protocol regressions use four published HeadsetControl / MyFinder frames plus synthetic faults and query transcripts, covering charge, voltage, rejection, power-off and expiry. No matching headset is attached locally; USB transport, power recovery and NGENUITY coexistence await hardware acceptance.

## CHERRY MX 2.0S WL Pro

The adapter matches receiver `046A:01E0` / `MX 2.0S Pro Dongle` and USB `046A:01DE` / `MX 2.0S WL Pro`, requiring `MI_01`, `FF1C:0092` and 64-byte input/output. Eligible management collections must be unique within the Windows USB interface instance. Both button and value capabilities must establish input/output Report `04` only; otherwise no query is sent. Full physical-parent resolution across USB interfaces is not implemented.

Only the zero-padded `04 20 00 1A 06` battery query is sent, without keymap, lighting, pairing or firmware operations. A valid reply must match the full length and five-byte prefix, status byte 7 equal to 0, percentage byte 8 in 1–100, and charger byte 9 equal to 0/1. Device percentages are retained without adjusting them to the vendor UI.

The hardware meaning of 0% is unverified and zero payloads cannot be reliably distinguished from request echoes, so this adapter does not publish 0%. Unknown states, charging enums, truncation and echoes never renew a sample. Failed queries do not establish disconnection. Background polling waits at most ten seconds; prior samples expire after 120 seconds and fresh responses restore them.

USB and receiver paths retain separate identities; identical keyboards are not merged by name or serial. A keyboard connected through both transports may show two cards, with independent tray visibility. Bluetooth `0687:01DE` and other Cherry products such as `046A:01AC` are excluded from this private adapter.

Evidence includes the original project's fixed query and MyFinder issue #482's identity, report-descriptor and reply-prefix records. Regression padding is synthetic. No matching keyboard is attached locally; low/full battery, stopped charging, long sleep recovery, multiple units and Cherry Utility coexistence await acceptance. Upstream reported a temporary 3–4 percentage-point difference after changing connections; its cause remains unresolved despite later agreement.

## Software DPI presets

- The main window and tray share 1–4 enabled presets per device, sorted by value. The count selector only changes the software list and retains hidden values; actual device stages are never reordered or removed.
- Presets use reviewed numerical DPI write capabilities on Logitech, CompX and Razer connections. Read-only connections, onboard-only Logitech mode and independent axes that prohibit scalar writes remain restricted.
- Apply saves the local preset. Use writes the device and verifies readback. CompX modifies the active onboard stage; Razer storage behavior follows the reviewed connection data. Stage changes while queued are rejected or reported during readback verification.
- Hardware acceptance and power-cycle persistence remain pending. Rapoo preset writes are not enabled.

## Actual Rapoo nRF54L stages

Reads the active profile, enabled count, up to seven X/Y stages and active index. The main window and tray switch existing device stages without changing values or stage count. Full configuration fingerprints and telemetry gate writes; full readback and a newer status report confirm the result. Configuration failure does not replace valid battery data. Unknown or mismatched interfaces are not written.

Synthetic transcripts cover collection pairing, both ACK layouts, seven slots, independent axes, profile changes, lost ACKs, ignored writes, neighbor changes, expired status and backend command routing. Hardware stage switching and official-driver coexistence remain unverified.

## MCHOSE keyboards and Arctis Nova headsets

The keyboard adapter queries only `FF02:0002` control collections with 20-byte input and output reports. Ordinary key-input collections are excluded.

| Interface | Identification | Implementation |
| --- | --- | --- |
| `3554:FA09` | Shared G75 Pro / K99 V2 / GX87 receiver; a specific product string is required for a model name, otherwise MCHOSE Keyboard | Windows SET_REPORT `13:4A` query with header, payload length, checksum and percentage validation |
| `258A:01BB` | A `13:05` handshake verifies the six-byte Z75S identity on each read; the shared ID alone does not distinguish Z75 | Windows WriteFile handshake and read-only battery query |

Query echoes are skipped. The full flag does not imply charging, and unknown charging flags do not discard valid percentages. Identical receivers retain separate Windows interface identities. G87 V2 uses the independent GLW reader described above. VGN wireless keyboard reads share the strict frame validator, rejecting echoes with no battery payload.

The Arctis Nova adapter queries only `1038`, `FFC0:0001` control collections with 65-byte input/output descriptors. It sends `00 B0` using Windows WriteFile and validates status replies. Other generations and descriptors are excluded.

| Family | PIDs | Battery semantics |
| --- | --- | --- |
| Nova 5 / 5X | `2232` / `2253` | Actual percentage, separate connection and charging fields |
| Nova 3P / 3X Wireless | `2269` / `226D` | Nova 5 status layout, actual percentage |
| Original Nova 7 / 7X / 7P and selected special editions | `2202`, `2206`, `220A`, `223A`, `227A`, `22A4` | Five levels displayed as ≈0% / ≈25% / ≈50% / ≈75% / ≈100%, retaining the level label |
| Updated Nova 7 / Gen 2, 7X / 7P and selected special editions | `22A1`, `227E`, `2258`, `229E`, `22AD`, `22A5`, `22A9`, `22A7`, `2298` | Actual percentage; values 0–4 remain exact percentages |

Explicit offline replies invalidate live readings, and fresh reports restore them. An all-zero query echo cannot establish disconnection and is left to cache expiry. Timeouts do not invent battery or connection state. Coarse levels cannot trigger percentage notifications. Headsets retain Windows audio controls, with no new firmware-setting writes.

Protocol regressions cover malformed frames, identity handshakes, identical receiver instances, offline/wake transitions and aggregate refresh. Neither new family is currently attached locally. Windows hardware transport, sleep/wake and coexistence with official software remain unverified in this project.

Estimates carry ≈ throughout the main window, tray list, taskbar icon, tooltip and diagnostic report. HID++ uses Solaar's representative values 5/20/50/90; original Nova models use their own 0/25/50/75/100 mapping. Exact readings take priority. Estimates remain separate from measurements and cannot trigger exact-percentage notifications; missing levels and uncalibrated voltage-only readings remain unknown.

## CompX mice and polling rates

Candidates must expose vendor `3554` / `373B`, collection `FF02:0002`, and exact 17-byte input/output buffers. Replies must match report ID, command, status, address, length and checksum. Unknown runtime identities can supply valid battery readings but cannot authorize guessed sensor settings. All mouse entries below await hardware verification in this project.

| Scope | Layout evidence | DPI stages | Polling-rate boundary |
| --- | --- | --- | --- |
| VGN Dragonfly F2 Master+ | `3554:FB56/FB57` plus the control descriptor | Four-byte stages; 50–26000 DPI in steps of 50; writes also require valid CID/MID and profile replies | 125–8000 Hz for the reviewed transport, verified by readback |
| ATK F1 Ultimate 2.0 | CID/MID `1,8` | Ultra segmented encoding; independent X/Y reads and edits | Stored rate readable; writes currently restricted to 125–1000 Hz |
| ATK A9 Mini + | CID/MID `1,31` / `1,52` | Six-byte PAW3955 records at `0x1B00`, 50–40000 DPI in steps of 10 | Stored rate readable; writes currently restricted to 125–1000 Hz |
| ATK X1 Pro Max / A9 Plus Nearlink | CID/MID `2,39` / `2,83` | Four-byte records interpreted with the corresponding sensor limits | Stored rate readable; writes currently restricted to 125–1000 Hz |
| VXE R1 / R1 Pro Max | CID/MID `2,11` / `2,12` / `2,27` | Actual stage reads; validated writes for Pro Max on `3554:F58A/F58C` | F58A preserves the complete system row; writes restricted to 125–1000 Hz |
| VXE R1 SE+ | CID/MID `2,32` | Nonlinear PAW3395SE codec; stage writes only on `3554:F58F` | `373B:1085` / `3554:F58E` read the separate `0x0070` selector row; unknown values stay unknown and these receivers remain read-only |
| VXE Star Flash / ATK control-report receivers | `373B:10C9/1278` plus the control descriptor | Windows SET_REPORT transport; configuration decoded only for known CID/MID layouts | Read-only on these connections, even with a known sensor; query echoes are not battery readings |

Stage counts, active indices and each X/Y record are validated. Switching a stage and editing its values are separate operations. F58A edits preserve adjacent stages and unknown system-row bytes. Writes recheck identity, profile and original bytes; failed readback or changed identity/profile is an error. CID/MID is a model identity, not a unique unit ID: replacing a mouse with another unit of the same model on the same receiver cannot currently be distinguished.

Logitech `0x8060` reads the advertised interval bitmap, including intervals that are not powers of two. `0x8061` queries the USB/wireless link-specific list and current value; an unknown connection is not assumed to be USB. The setter sends only the rate code and verifies the same link afterward. Onboard mode and failed mode queries leave valid readings visible but disable writes; the app does not switch modes or rewrite onboard profiles.

The performance page provides separate X/Y inputs per stage, Apply/Use actions and an independent polling-rate control. Drafts retain their original device/profile/axis values; changed configuration rejects a stale edit. Cached stage layouts remain visible without write permission. Complete Logitech and CompX operations are serialized within this process; other applications and diagnostic processes are not covered by that lock.

Simulated exchanges cover CompX responses, Logitech rates, Razer writes and configuration changes, page-command reads and headset battery data. These mouse and headset capabilities still await hardware verification in this project.

## Razer battery and settings

Candidates require vendor `1532` and a 91-byte Feature Report. Reviewed connections in the table below can also query that fixed-length report on a Generic Desktop Mouse collection with no declared Feature length. Collections are probed individually; readings require matching transaction ID, class, command, length, status and XOR checksum. Windows interface instances and a hash of a valid device serial distinguish identities. Empty, placeholder or malformed serials cannot authorize settings; raw serials are not exported in reports.

- Battery `07:80` consistently scales raw 0–255 values; charging uses the separate `07:84` command. A failed charging query does not erase valid battery data.
- `04:85` reads actual X/Y DPI; `04:86` reads up to five complete stage records, preserving duplicates and independent axes. Captured one-based record IDs, the one-based active index, and zero storage in replies are handled separately from the confirmed zero-based write format. Storage is not a profile ID; edit drafts retain a fingerprint of the complete table.
- `00:C0` and `00:85` rate formats are decoded separately. Reviewed connections select the command from capability data; unknown products try the legacy query only after an explicit unsupported response to the newer query. A current rate is not a supported-rate list.
- Busy, echo and previous-command reports are polled within a bounded deadline. Silence does not create a permanent unsupported cache. Other brands continue polling independently.
- Transaction IDs are confirmed per command. A setter uses the matching getter's confirmed ID and sends once; lost acknowledgements lead to readback, without resending the write or blindly rolling back. Identity, normal device mode and configuration are checked before and after writes.
- A successful control collection skips its siblings; failure still allows fallback to another collection. Identity confirmation across siblings preserves preferences. A temporary identity failure retains the latest device card with writes disabled, without reactivating a replaced device.

The following connection data shares one protocol implementation. Every entry awaits hardware validation in this project. Unlisted connections remain read-only; shared receivers do not inherit guessed capabilities from a brand or appearance.

| Model / connection | PID | DPI range | Current DPI / stage storage | Rate encoding and allowed values |
| --- | --- | --- | --- | --- |
| Viper wired | `0078` | 100–16000 | 0 / 1 | Legacy; 125, 500, 1000 Hz |
| DeathAdder V2 | `0084` | 100–20000 | 0 / 1 | Legacy; 125, 500, 1000 Hz |
| Viper V2 Pro wired | `00A5` | 100–30000 | 1 / 1 | Legacy; 125, 500, 1000 Hz |
| Viper V2 Pro stock receiver | `00A6` | 100–30000 | 1 / 1 | Extended; 125, 500, 1000 Hz |
| DeathAdder V3 Pro wired / stock receiver; Viper V3 HyperSpeed | `00B6/00B7/00B8` | 100–30000 | 1 / 1 | Legacy; 125, 500, 1000 Hz |
| Viper V3 Pro wired | `00C0` | 100–35000 | 1 / 1 | Legacy; 125, 500, 1000 Hz |
| Viper V3 Pro receiver | `00C1` | 100–35000 | 1 / 1 | Extended; 125, 500, 1000, 2000, 4000, 8000 Hz; separate persistence commit |

Scalar DPI writes require originally equal X/Y and verify both axes. Stage edits preserve all neighboring values, reserved bytes and unused slots. All active records must fit the reviewed range so that invalid neighboring values cannot be rewritten. Both switching and editing recheck the full original table, read back the full result, and verify current axes. The `00C1` rate setter confirms the live change before committing with selector 1 and transaction `1F`; persistence across power cycles still needs hardware testing. Older byte-scaled DPI and unconfirmed generations remain unknown.

## Lamzu / WLMouse page-command

This family is separate from 17-byte EEPROM16. It uses vendors `373E/37B0/36A7`, a vendor Usage Page and a 65-byte Feature Report, matching target, page, command, status and length. Bootloaders and the Report 4 WLMouse Beast X 4K `36A7:A887` are excluded; similar descriptors do not establish protocol compatibility.

- Page 0 / `83` reads percentage and charging. An unknown charging flag does not erase valid battery data.
- Reviewed layouts for Lamzu Inca / Paro / Maya X, CRDRAKO, Attack Shark R5/R6/R8, RAWM V4, Vancer and WLMouse page-command connections can read the actual profile, complete X/Y stages, active stage and rate. Identification conditions remain in the adapter and do not imply complete support for those brands.
- Shared `37B0:0001/0003` receivers first query the paired model. Unconfirmed models and the different Luna Fury layout remain battery-only.
- Profile is rechecked after settings reads. Changed profiles or incomplete tables discard settings while preserving independent battery data. Counts and axes are validated without default stages.

DPI, profile and rate writes are unavailable in this family. Sibling-collection status notifications are not yet integrated. All entries await local hardware verification.

## Shared ATK / MCHOSE headset protocol

ATK N9 V2 Ultimate `373B:1262` and MCHOSE V9 Turbo+ `3837:600A` share FF90, 64-byte input/output and the `55:65` query over Windows WriteFile. The adapter restricts control collections and rejects echoes, truncation, invalid percentages and unknown states. State fields are decoded per model; see ATK N9 V2 ULTRA / MCHOSE V9 headsets above for their distinct charging and full-state meanings.

These headsets use the existing audio-control page; the adapter only queries battery. Identification and response evidence come from Voltide sources and upstream hardware/community comments. Validation uses simulated exchanges; neither headset has been tested locally.

Additional backends share descriptor scans for 500 milliseconds without extending their lifetime. Scan failure remains an error, never an invented empty device list or fresh evidence from an expired scan.

## Core protocol capabilities

| Brand / protocol | Model / scope | Battery | DPI | Status |
| --- | --- | --- | --- | --- |
| ROG | HARPE ACE MINI, USB / OMNI | Verified | Verified; existing detailed settings retained | Stable release |
| Logitech HID++ 1.0 | Devices exposing battery registers 0x0D or 0x07 | Exact charge or coarse levels implemented | No writes yet | Hardware validation pending |
| Logitech HID++ 2.0 | Devices exposing 0x1000 or 0x1004 | Protocol implemented | 0x2201 numerical control; actual stage reads and switching for supported 0x8100 onboard profiles | Hardware validation pending |
| Standard HID | Generic Device Controls / Battery Strength (06:20) in the descriptor | Read using descriptor limits | No standard DPI write operation | Hardware validation pending |
| Rapoo nRF54L | 77 official driver model entries, covering specific VT0 / VT1 / VT2 / VT3 / VT4 / VT6 / VT7 / VT8 / VT9 variants and ESM models | Status report reads implemented | See Actual Rapoo nRF54L stages above for configuration reads and controls | Per-model hardware validation pending |
| VGN Beiying | V98 Pro V3, USB / 2.4G | Wired feature and receiver status queries implemented | No DPI for keyboards | USB read verified; receiver and wake recovery validation pending |

Voltage and coarse HID++ battery levels are not shown as exact percentages. Charging remains unknown when standard HID does not expose it. Standard HID also distinguishes native 0–100 percentages from other descriptor ranges: values scaled from 0–4, 0–255 and other ranges carry ≈, retain their precision through caching and cannot trigger exact-percentage notifications.

HID++ 1.0 receivers that expose pairing information provide per-slot names and device types. Device identities are hashed before they are used for preferences. A temporary identity query failure retains the existing card; first-time identity confirmation migrates custom names, types, tray selection and order without treating a replacement device as the original one. If HID++ 2.0 unified battery reports only a coarse level, other battery features are queried for an exact percentage supplied by the device.

The standard HID adapter reserves listed ROG devices and receivers for detailed ROG support, and checks other devices for battery capabilities in their descriptors. Hardware compatibility still needs validation for each model.

Each backend has its own polling thread and command queue, so a timeout or DPI write does not block polling for other brands. Coarse or missing battery readings remain coarse or unknown instead of becoming 0%. Logitech DPI writes validate advertised values and verify readback; failed writes attempt to restore and verify the original value.

Adjustable DPI lists are read across the device's reported pages, including values and step ranges spanning report boundaries. Transport padding is excluded from parsing. Incomplete lists, invalid report lengths and failed subsequent queries do not authorize DPI writes from partial data.

Logitech's actual onboard stages are separate from the hardware's allowed DPI values. Supported mice in onboard mode expose five DPI slots through `0x8100`; disabled slots do not renumber other slots, and duplicate values are retained. Length, format and CRC are validated before switching is enabled. Switching changes only the current DPI slot, without changing the whole profile, G HUB mode or Flash. Identity and profile are checked again before switching, and readback is verified afterward. A failed switch attempts to restore the previous stage only while the original profile remains active.

G HUB host mode does not guarantee that the computer's configured stages are available over HID++. The interface then retains validated numerical control through `0x2201` and common-value buttons, without labeling these shortcuts as configured stages. Failed onboard queries or unsupported formats retain confirmed current DPI readings but disable numerical writes. Onboard reads currently support known profile formats 1–5 with 256-byte sectors; other formats need additional protocol and hardware validation.

Rapoo matches the 77 model entries explicitly assigned to `VT_nrf54L` by the official web driver. See the [Rapoo model table](RAPOO_MODELS.md) for model and USB/receiver PIDs. Battery reads use status report `0x07` on the `FF00:0002` control collection; other controllers and unknown PIDs are not probed with this protocol. Missing valid reports remain unknown and require a user wake test. See Actual Rapoo nRF54L stages above for configuration reads and controls; protocol inference is separate from hardware verification.

Wireless battery and current DPI reads are hardware-verified on Rapoo VT1 `24AE:1415`. Its status report arrives about every three seconds. A retained read-only listener publishes observations at an independent interval of at most two seconds, within the eight-second sample lifetime even when the active polling interval is sixty seconds. Silence and recovery are separately covered by synthetic tests. Malformed or truncated reports cannot replace the last valid sample or extend its lifetime. Other Rapoo models and stage switching still require hardware validation.

VGN currently matches V98 Pro V3 `258A:024E/024F/027E/027F` and queries only the confirmed vendor control interface. Wired battery reads use feature report `0x09`; receivers use command `0x4A` in report `0x13`, with length and checksum validation. Unknown IDs and unrelated vendor collections are not probed with this protocol. USB hardware validation covers a 100% reading and charging status; other battery levels, receivers and wake recovery still need testing.

Open **Settings → About → Device feedback**, choose a device and issue category, and describe the problem. You can preview or disable **Include diagnostics** and attach screenshots. The optional summary contains app and system versions, the original model, type, connection, battery, VID/PID, usage, report lengths, backend and the last 24 query summaries matching the backend and VID/PID. It excludes payloads, serial numbers, MAC addresses, interface paths, usernames, device aliases, DPI setting values and raw logs.

Builds with a configured feedback service can submit in the app and use **My feedback** to read maintainer replies or add information. Otherwise, copy the content and open GitHub, adding screenshots manually; browser submissions do not create in-app history. Descriptions, selected diagnostics and screenshots are public. Network access occurs only when you submit, add information or check replies. Diagnostic logs and the command-line reports below remain available. See the [privacy policy](../PRIVACY.md).

For remote adaptation, run `x-battery.exe device-report --probe`. It performs read-only queries and then generates a sanitized report from the same process, including returned battery, charging status, current DPI, available onboard stages and the current stage. It changes no settings and uploads nothing. Profile names, button settings and raw memory contents are excluded. Users can compare readings with the official driver; DPI write tests are separate. Local hardware ownership is not required for user-assisted testing, although unknown private protocols may still need driver analysis or targeted communication captures.

Other brands and models listed by FerrisPulse and similar projects have not all been integrated into X-Battery. Generic catalogues, shared receiver identifiers and validated protocols require separate checks and are not a statement of compatibility with this app.

Protocol references: [Solaar](https://github.com/pwr-Solaar/Solaar), [libratbag](https://github.com/libratbag/libratbag), [USB HID Usage Tables](https://usb.org/document-library/hid-usage-tables-15), [Rapoo A HUB](https://hub.rapoo.cn/), [VGN web driver](https://drive.vgnlab.com.cn/) and [FerrisPulse protocol research](https://github.com/Ferris-echo/FerrisPulse). The implementation does not copy driver or open source project code.

## DELUX M900 Pro

Adds passive battery reception only for `1D57:FA65 / 000A:0000`, Input5/Output0/Feature0, with a runtime descriptor proving sole input report 03. Ambiguous sibling collections are rejected. The shared VID/PID does not identify Inphic ZHEN: its usage and report prefix differ.

Only read-only handles are opened. Exact `03 30 40 <flag> <percent>` frames are accepted. Flag 01 yields 0–100% when settled; flag 03 exposes charging but no potentially decorated percentage. Unrecorded flag 02 exposes neither full-charge nor percentage. Initial connection, failed reads, long silence and leaving charging require at least 8 seconds and a fresh discharging report. Samples expire after 90 seconds without refresh. State is isolated by Windows path and serial metadata and reset on removal. Reads are bounded to 5 seconds/32 reports per receiver; backend polling interval is capped at 10 seconds. Multiple receivers are read sequentially.

Evidence is pinned VigilWatt contributor documentation and the recorded 89% discharging/charging vectors. No local hardware validation; upstream also notes no maintainer retest and defaults the plugin off. Verify descriptors, 0%, sleep, dock transitions, multiple receivers, Inphic coexistence and official-software coexistence on hardware. Existing M900 Ultra/GH8K CompX support is unchanged.

## HECATE Wireless Receiver (35BB:D3FB)

- Scope: 2.4 GHz receiver `35BB:D3FB`, `MI_01/Col05`, usage `FF02:0002`, 17-byte input/output; runtime admission also requires sole report `08` in both directions. This does not establish support for D00B, wired D3FA/D3FC/D3FD or the older G3M PRO.
- Reads percentage, voltage and connection state. Charging remains unknown; no DPI, polling-rate or configuration writes. Online status is checked before and after power; failed queries do not refresh samples, which expire after 120 seconds.
- Upstream observed this receiver with a G3M Pro upgraded model, but D3FB is not proven exclusive to it. The UI retains the receiver product name without claiming an exact mouse identity.
- Complete public STATUS / 50%, 3848 mV POWER fixtures and synthetic boundary, multi-receiver, disconnect and expiry regressions are covered. No local hardware acceptance yet: full descriptors, firmware variants, actual zero, charging, sleep caching and HECATE Connect coexistence remain unverified. See [protocol notes](PROTOCOLS.en.md#hecate-wireless-receiver-35bbd3fb).
