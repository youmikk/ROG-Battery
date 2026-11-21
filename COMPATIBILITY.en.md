# Device compatibility

[简体中文](COMPATIBILITY.md)

Version 1.4.5 adds `0x2202` extended DPI reads and validated scalar writes, with simulated protocol coverage and Logitech hardware validation pending. Battery observations now distinguish live, cached and stale readings; failed queries alone do not establish disconnection. See [protocol research](PROTOCOLS.en.md) for upstream catalogs and capability boundaries.

This document lists additions in 1.4.6 and 1.4.5 alongside existing 1.4.4 capabilities with their hardware validation status. Discovery, model recognition and parser tests do not verify hardware compatibility.

## In development: DualSense gamepads

The adapter matches Sony `054C:0CE6` DualSense and `054C:0DF2` DualSense Edge, only on `0001:0005` Game Pad collections with 64-byte USB or 78-byte Bluetooth input descriptors. This work follows the 1.4.6 tag.

- USB Report `01` carries battery status at offset 53; Bluetooth Report `31` uses offset 54 and requires CRC32 with prefix `A1`. A padded Bluetooth minimal Report `01` is never parsed as USB battery data.
- Bands 0–9 display ≈5%–≈95%; band 10 or the fully charged state displays ≈100%. Charging stays separate. Voltage, temperature and charging errors never become 0%, and invalid bands are not clamped to full.
- Input handles request read access only. Observed Bluetooth minimal mode prompts a calibration Feature `05` GET to request extended reports, with its `A3` CRC checked. Battery still comes from independently validated input reports; no rumble, lighting or settings output is sent.
- The background polling wait is capped at two seconds. Timeouts and faults do not renew the last sample; numbers disappear after eight seconds and return on a fresh valid report. Identical controllers retain separate Windows interface identities. Cross-transport deduplication of one controller connected over USB and Bluetooth is not implemented.
- Diagnostics retain protocol identifiers, lengths, timing and outcomes, without button, stick, touch or unique identity data. Estimates cannot trigger exact-percentage notifications.

On 2026-11-21, all 187 local regressions passed and the Windows x64 build completed without warnings. Coverage includes USB / Bluetooth layouts, CRC, invalid and truncated reports, minimal mode, identical models, expiry and recovery. Neither controller is currently attached locally; hardware transport, sleep recovery and Steam / DS4Windows coexistence remain unverified. The read-only hardware probe still returned valid ROG and VGN keyboard battery readings.

## In development: XInput gamepads

Windows' system XInput 1.4 provides four slots through `XInputGetState` and `XInputGetBatteryInformation`. Support depends on the current Xbox-compatible driver and connection returning battery data, not on a whole-brand claim. The adapter does not fabricate HID paths, VID/PID or physical model names, and sends no vibration or settings commands.

Only known wireless battery types supply usable levels. Empty / Low / Medium / Full display as ≈0% / ≈20% / ≈50% / ≈100%. These are application display representatives: Windows defines no percentage boundaries or charging flag. Wired and unknown battery types clear previous numbers immediately. Failed queries retain the original measurement timestamp, with expiry after eight seconds. A confirmed disconnect clears that slot's battery before another controller can reuse it.

Names, tray selection and order are saved by slot. XInput exposes no stable physical identity, so preferences follow the slot when controllers change or Windows reassigns it; a replacement between polls without an observed disconnect cannot be identified. Virtual XInput controllers can appear alongside their physical HID device, and reliable cross-protocol deduplication is not implemented. Users can select which cards appear in the tray.

On 2026-11-21 all 192 local regressions passed and the Windows x64 build completed without warnings. The system API loaded successfully with all four slots disconnected. Battery grades, wireless / wired transitions, sleep recovery and different drivers still need hardware acceptance. The read-only probe returned valid VGN keyboard charge; the ROG receiver supplied no valid device reading in this run.

## In development: Sony INZONE headsets

The adapter matches only Sony `054C:0EBF` H5 on collection `FF04:0002`, and `054C:0FA8` H9 II on `FF04:0001` / `MI_05`, with 64-byte input and output reports. H3, H7, original H9, Buds, PS5 mode and firmware-update interfaces are outside this adapter's scope.

- Only battery GET queries are sent. Volume, ANC, microphone and firmware settings are not changed. Existing audio controls in the main window and tray continue to use Windows audio APIs.
- Replies must match the report ID, fixed battery payload length, HCI direction, Sony marker, headset-to-PC address, event, transaction and checksum. Transaction IDs 0/1 are reserved; old replies and unsolicited notifications cannot satisfy a current query.
- Values 0–100 are device-reported percentages without ≈. Charger bytes 0/1 mean not charging / charging; other values remain unknown. Full charge does not imply charging. Battery `FF` explicitly marks offline and clears prior numbers; 101–254 are invalid.
- Background polling waits at most ten seconds. Failed queries retain the original sample time, with numbers hidden after 120 seconds and restored by a valid response. Identical receiver models retain separate Windows interface identities.

Protocol tests cover a published H5 notification, transaction wraparound, echoes, unrelated events, checksum errors, valid zero, offline and wake. UI-state regressions cover expiry and recovery. Neither headset is attached locally; USB communication, power recovery, receiver mode changes and INZONE Hub coexistence await hardware acceptance. This feature is not part of the 1.4.6 tag.

## In development: HyperX Cloud Alpha 2 Wireless

Only the `03F0:08BE` control station's `FF13:FF00` collection with 64-byte input/output reports is matched. The `03F0:0ABE` audio function, `FF13:0001` sibling, original Alpha, and Cloud II / III protocols are excluded. Only connection and battery queries are sent; audio settings are not written.

- `52 01` / `53 01` queries connection; `50 02` / `51 02` reads percentage, independent charging state and little-endian voltage. Percentages 0–100 remain exact device reports. Unknown charger bytes and 0 / `FFFF` voltage stay unknown, without deriving charging from voltage bits or full charge.
- Length, report and subcommand must match. Echoes, ACKs, audio replies and unsolicited notifications are ignored; a matching NACK stops the query. Only connection values 0/1 are accepted.
- Connection is checked before and after battery. A power-off during the read cannot publish station-cached charge. Failed reconfirmation cannot renew the prior sample; explicit offline clears it. Polling waits at most ten seconds, cached samples expire after 120 seconds, and identical stations keep separate interface identities.
- The protocol has no transaction number or checksum. Same-command replies from another process cannot be reliably distinguished. Filtering and bounded queue draining do not replace NGENUITY coexistence testing; cross-mode USB deduplication is not claimed.

Protocol regressions use four published HeadsetControl / MyFinder frames plus synthetic faults and query transcripts, covering charge, voltage, rejection, power-off and expiry. No matching headset is attached locally; USB transport, power recovery and NGENUITY coexistence await hardware acceptance. This feature is not part of the 1.4.6 tag.

## Version 1.4.6: software DPI presets

- The main window and tray share 1–4 enabled presets per device, sorted by value. The count selector only changes the software list and retains hidden values; actual device stages are never reordered or removed.
- Presets use reviewed numerical DPI write capabilities on Logitech, CompX and Razer connections. Read-only connections, onboard-only Logitech mode and independent axes that prohibit scalar writes remain restricted.
- Apply saves the local preset. Use writes the device and verifies readback. CompX modifies the active onboard stage; Razer storage behavior follows the reviewed connection data. Stage changes while queued are rejected or reported during readback verification.
- Local configuration, ordering, restoration, tray state and protocol regressions passed. Hardware acceptance and power-cycle persistence remain pending. Rapoo preset writes are not enabled.

## Version 1.4.6: actual Rapoo nRF54L stages

Reads the active profile, enabled count, up to seven X/Y stages and active index. The main window and tray switch existing device stages without changing values or stage count. Full configuration fingerprints and telemetry gate writes; full readback and a newer status report confirm the result. Configuration failure does not replace valid battery data. Unknown or mismatched interfaces are not written.

Local transcripts cover collection pairing, both ACK layouts, seven slots, independent axes, profile changes, lost ACK, ignored writes, neighbor changes, expired status and backend command routing. Hardware switching and official-driver coexistence remain unverified; no supported Rapoo interface is currently available locally. The read-only scope below describes released version 1.4.5.

## Version 1.4.6: MCHOSE keyboards and Arctis Nova headsets

The keyboard adapter queries only `FF02:0002` control collections with 20-byte input and output reports. Ordinary key-input collections are excluded.

| Interface | Identification | Implementation |
| --- | --- | --- |
| `3554:FA09` | Shared G75 Pro / K99 V2 / GX87 receiver; a specific product string is required for a model name, otherwise MCHOSE Keyboard | Windows SET_REPORT `13:4A` query with header, payload length, checksum and percentage validation |
| `258A:01BB` | A `13:05` handshake verifies the six-byte Z75S identity on each read; the shared ID alone does not distinguish Z75 | Windows WriteFile handshake and read-only battery query |

Query echoes are skipped. The full flag does not imply charging, and unknown charging flags do not discard valid percentages. Identical receivers retain separate Windows interface identities. G87 V2 uses a different, unimplemented protocol. VGN wireless keyboard reads share the strict frame validator, rejecting echoes with no battery payload.

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

Version 1.4.6 feature verification (2026-11-17): 180 local regressions passed and the Windows x64 release build completed without warnings. Render checks cover Chinese/English, light/dark themes at 100% / 125% scaling and 16/20/24/32-pixel tray icons, including exact, estimated, unknown and stale readings. The read-only probe still returned 100% and charging for VGN V98 Pro V3.

## Version 1.4.5: CompX mice and polling rates

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

The performance page adds separate X/Y inputs per stage, Apply/Use actions and an independent polling-rate control. Drafts retain their original device/profile/axis values; changed configuration rejects a stale edit. Cached stage layouts remain visible without write permission. Complete Logitech and CompX operations are serialized within this process; other applications and diagnostic processes are not covered by that lock.

Verification (2026-11-17): all 148 local regression tests passed and the Windows x64 release build completed without compiler warnings. Simulated exchanges cover invalid CompX responses, Logitech rates, Razer writes and configuration changes, page-command reads, and headset battery data. The executable's read-only probe returned 100% and charging for VGN V98 Pro V3. No supported Rapoo, Logitech, CompX, Razer, page-command or FF90 headset interface was enumerated in this run; the ROG receiver supplied no valid device reading. New mouse and headset capabilities still await hardware verification in this project.

## Version 1.4.5: Razer battery and settings

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

## Version 1.4.5: Lamzu / WLMouse page-command

This family is separate from 17-byte EEPROM16. It uses vendors `373E/37B0/36A7`, a vendor Usage Page and a 65-byte Feature Report, matching target, page, command, status and length. Bootloaders and the Report 4 WLMouse Beast X 4K `36A7:A887` are excluded; similar descriptors do not establish protocol compatibility.

- Page 0 / `83` reads percentage and charging. An unknown charging flag does not erase valid battery data.
- Reviewed layouts for Lamzu Inca / Paro / Maya X, CRDRAKO, Attack Shark R5/R6/R8, RAWM V4, Vancer and WLMouse page-command connections can read the actual profile, complete X/Y stages, active stage and rate. Identification conditions remain in the adapter and do not imply complete support for those brands.
- Shared `37B0:0001/0003` receivers first query the paired model. Unconfirmed models and the different Luna Fury layout remain battery-only.
- Profile is rechecked after settings reads. Changed profiles or incomplete tables discard settings while preserving independent battery data. Counts and axes are validated without default stages.

DPI, profile and rate writes are unavailable in this family. Sibling-collection status notifications are not yet integrated. All entries await local hardware verification.

## Version 1.4.5: shared ATK / MCHOSE headset protocol

ATK N9 V2 Ultimate `373B:1262` and MCHOSE V9 Turbo+ `3837:600A` share FF90, 64-byte input/output and the `55:65` query over Windows WriteFile. The adapter restricts control collections and rejects echoes, truncation, invalid percentages and unknown states. States 2 / 3 / 4 mean discharging, charging and full; the full flag does not force the measured percentage to 100%.

These headsets use the existing audio-control page; the adapter only queries battery. Identification and response evidence come from Voltide sources and upstream hardware/community comments. Simulated protocol tests pass, but neither headset has been tested locally.

Additional backends share descriptor scans for 500 milliseconds without extending their lifetime. Scan failure remains an error, never an invented empty device list or fresh evidence from an expired scan.

## Existing 1.4.4 capabilities

| Brand / protocol | Model / scope | Battery | DPI | Status |
| --- | --- | --- | --- | --- |
| ROG | HARPE ACE MINI, USB / OMNI | Verified | Verified; existing detailed settings retained | Stable release |
| Logitech HID++ 1.0 | Devices exposing battery registers 0x0D or 0x07 | Exact charge or coarse levels implemented | No writes yet | 1.4.4; hardware validation pending |
| Logitech HID++ 2.0 | Devices exposing 0x1000 or 0x1004 | Protocol implemented | 0x2201 numerical control; actual stage reads and switching for supported 0x8100 onboard profiles | 1.4.4; hardware validation pending |
| Standard HID | Generic Device Controls / Battery Strength (06:20) in the descriptor | Read using descriptor limits | No standard DPI write operation | 1.4.4; hardware validation pending |
| Rapoo nRF54L | 77 official driver model entries, covering specific VT0 / VT1 / VT2 / VT3 / VT4 / VT6 / VT7 / VT8 / VT9 variants and ESM models | Status report reads implemented | Current DPI read-only; independent X/Y is not shown as one DPI | 1.4.4; per-model hardware validation pending |
| VGN Beiying | V98 Pro V3, USB / 2.4G | Wired feature and receiver status queries implemented | No DPI for keyboards | USB read verified; receiver and wake recovery validation pending |

Voltage and coarse HID++ battery levels are not shown as exact percentages. Charging remains unknown when standard HID does not expose it.

HID++ 1.0 receivers that expose pairing information provide per-slot names and device types. Device identities are hashed before they are used for preferences. A temporary identity query failure retains the existing card; first-time identity confirmation migrates custom names, types, tray selection and order without treating a replacement device as the original one. If HID++ 2.0 unified battery reports only a coarse level, other battery features are queried for an exact percentage supplied by the device.

The standard HID adapter reserves listed ROG devices and receivers for detailed ROG support, and checks other devices for battery capabilities in their descriptors. Hardware compatibility still needs validation for each model.

Each backend has its own polling thread and command queue, so a timeout or DPI write does not block polling for other brands. Coarse or missing battery readings remain coarse or unknown instead of becoming 0%. Logitech DPI writes validate advertised values and verify readback; failed writes attempt to restore and verify the original value.

Adjustable DPI lists are read across the device's reported pages, including values and step ranges spanning report boundaries. Transport padding is excluded from parsing. Incomplete lists, invalid report lengths and failed subsequent queries do not authorize DPI writes from partial data.

Logitech's actual onboard stages are separate from the hardware's allowed DPI values. Supported mice in onboard mode expose five DPI slots through `0x8100`; disabled slots do not renumber other slots, and duplicate values are retained. Length, format and CRC are validated before switching is enabled. Switching changes only the current DPI slot, without changing the whole profile, G HUB mode or Flash. Identity and profile are checked again before switching, and readback is verified afterward. A failed switch attempts to restore the previous stage only while the original profile remains active.

G HUB host mode does not guarantee that the computer's configured stages are available over HID++. The interface then retains validated numerical control through `0x2201` and common-value buttons, without labeling these shortcuts as configured stages. Failed onboard queries or unsupported formats retain confirmed current DPI readings but disable numerical writes. Onboard reads currently support known profile formats 1–5 with 256-byte sectors; other formats need additional protocol and hardware validation.

Rapoo now matches the 77 model entries explicitly assigned to `VT_nrf54L` by the official web driver. See the [Rapoo model table](RAPOO_MODELS.md) for model and USB/receiver PIDs. Only status report `0x07` on the `FF00:0002` control collection is read; other controllers and unknown PIDs are not probed with this protocol. Missing valid reports remain unknown and require a user wake test. EEPROM writes are not enabled; protocol inference is not hardware verification.

Version 1.4.5 has verified wireless battery and current DPI reads on Rapoo VT1 `24AE:1415`. Hardware sends a status report about every three seconds, which version 1.4.4's short listening window can miss. The revision keeps the reader open and publishes observations at an independent interval of at most two seconds, avoiding a conflict between the eight-second sample lifetime and the default sixty-second active polling interval. Continuous read-only validation exceeded eighty-three seconds, with valid battery readings in all thirty-nine UI state merges; simulated coverage separately verifies silence and recovery. Malformed or truncated reports cannot replace the last valid sample or extend its lifetime. Other Rapoo models still require hardware validation.

VGN currently matches V98 Pro V3 `258A:024E/024F/027E/027F` and queries only the confirmed vendor control interface. Wired battery reads use feature report `0x09`; receivers use command `0x4A` in report `0x13`, with length and checksum validation. Unknown IDs and unrelated vendor collections are not probed with this protocol. A USB device returned valid 100% battery and charging status; other battery levels, receivers and wake recovery still need testing.

Settings → About → Device compatibility opens a GitHub form and exports factory model, VID/PID, usage, report lengths, and a summary of the last 128 protocol queries in the current session: feature identifiers, durations and results. The report excludes payloads, serial numbers, MAC addresses, interface paths, usernames, custom device names, DPI setting values and raw logs.

For remote adaptation, run `rog-mouse-ctl device-report --probe` using version 1.4.4 or later. It performs read-only queries and then generates a sanitized report from the same process, including returned battery, charging status, current DPI, available onboard stages and the current stage. It changes no settings and uploads nothing. Profile names, button settings and raw memory contents are excluded. Users can compare readings with the official driver; DPI write tests are separate. Local hardware ownership is not required for user-assisted testing, although unknown private protocols may still need driver analysis or targeted communication captures.

Other brands and models listed by FerrisPulse and similar projects have not all been integrated into R Battery. Generic catalogues, shared receiver identifiers and validated protocols require separate checks and are not a statement of compatibility with this app.

Protocol references: [Solaar](https://github.com/pwr-Solaar/Solaar), [libratbag](https://github.com/libratbag/libratbag), [USB HID Usage Tables](https://usb.org/document-library/hid-usage-tables-15), [Rapoo A HUB](https://hub.rapoo.cn/), [VGN web driver](https://drive.vgnlab.com.cn/) and [FerrisPulse protocol research](https://github.com/Ferris-echo/FerrisPulse). The implementation does not copy driver or open source project code.
