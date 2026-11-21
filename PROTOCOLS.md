# 多品牌协议研究与接入依据

[English](PROTOCOLS.en.md) · [已实现兼容性](COMPATIBILITY.md)

核对日期：2026-11-21。本文记录上游源码中的协议、型号识别和状态处理依据。上游目录项、上游测试、协议实现和本项目实机验证是不同证据，不能合并成“已兼容全部型号”。

## 源码范围与许可证

| 项目 | 核对版本 | 已检查的主要源码 | 许可证 |
| --- | --- | --- | --- |
| [OpenMouse mouse-protocol](https://github.com/OpenMouse-Project/mouse-protocol/tree/5a4f6433da3d70865d008874c7055016b336c197) | `5a4f643` | `src/drivers/registry.ts`、`drivers/mouse-types.ts`、`drivers/logitech/hidpp.ts`、`drivers/vgn/hid.ts`、`drivers/atk/hid.ts`、`drivers/atk/products.ts`、`drivers/atk/device-catalog.generated.ts`、`drivers/lamzu/hid.ts`、`drivers/wlmouse/hid.ts`、`drivers/razer/hid.ts`、`drivers/corsair/bragi-hid.ts`、`drivers/steelseries/aerox3-wireless-hid.ts`；`src/vgn/index.ts`、`src/compx/codec.ts`、`src/lamzu/index.ts`、`src/wlmouse/index.ts`、`src/razer/codec.ts`、`src/razer/devices.ts`、`docs/razer-testing.md` 及相应协议测试 | 根目录 LICENSE 为 AGPL-3.0 |
| [OpenMouse](https://github.com/OpenMouse-Project/openmouse/tree/28f08ddf61ce0bc97a4eaa780b98bfe31a9f39fc) | `28f08dd` | `src/device/device-clients.ts`、`driver-classes.ts`、`mouse-receivers.ts`；核对界面如何选择控制接口及接收器角色 | 根目录 LICENSE 为 AGPL-3.0 |
| [FerrisPulse](https://github.com/Ferris-echo/FerrisPulse/tree/335fb15ba98be94a85e13061dc0812a04b3aadf6) | `335fb15` | 完整源码目录；`Hardware/BuiltInDrivers.cs`、`HardwareDriverRegistry.cs`、`HardwareCommandBus.cs`、`HidTransport.cs`、`LogitechDeviceHelper.cs`、`VgnDeviceHelper.cs`、`RapooDeviceHelper.cs`、`Models/MouseBatteryInfo.cs` | GPL-3.0 |
| [MyFinder Device Battery](https://github.com/wushuai0802/myfinder-device-battery/tree/73d99e2087dd70ae8eef4199854e0c495a66f0bc) | `73d99e2` | `engine/providers/` 下的 `base.py`、`logitech.py`、`rapoo.py`、`razer.py`、`vgn_dragonfly3.py`、`fa6x_passive.py`、`profiles.py`；`engine/battery_hid.py`、`engine/server.py`、兼容性记录及 provider 目录 | AGPL-3.0 |
| [Solaar](https://github.com/pwr-Solaar/Solaar/tree/e7304c4c451cc9bb4f206a914844525e67856a28) | `e7304c4` | `lib/logitech_receiver/settings_templates.py`、`base.py`、`notifications.py`；DPI、回报率、查询超时及通知分流 | 所查文件 GPL-2.0-or-later |
| [libratbag](https://github.com/libratbag/libratbag/tree/8235b5bb6032dea15901ab58a8218956f4494d08) | `8235b5b` | `src/hidpp20.c`；功能查询、板载配置和 DPI 分辨率模型 | 所查文件 MIT |
| [OpenRazer](https://github.com/openrazer/openrazer/tree/dfe987c8b2053f1c111117ea20163cdb2a07f893) | `dfe987c` | `driver/razermouse_driver.c`、`razerchromacommon.c`；DPI、档位、电量、充电及回报率命令 | 所查文件 GPL-2.0-or-later |
| [Non-Synapse-Mouse](https://github.com/Damnedapa/Non-Synapse-Mouse/tree/267cf595507a9ca1617019885a99ee601b792d06) | `267cf59` | `Non_Synapse_Mouse.py`；Windows 功能报告传输、transaction ID 探测、busy 重试、DPI 读写及界面缓存 | 源文件声明 GPL-3.0-or-later |
| [orpheus](https://github.com/lindestad/orpheus/tree/693538c8dc885340eff0b4844ed82f37de48b76d) | `693538c` | `src/protocols.rs`、`Cargo.toml`；Feature64、EEPROM16、IPI Pix、HID++、Razer 的传输与能力边界 | MIT |
| [Voltide](https://github.com/sdparklab/Voltide/tree/c1872c4c93124d62c05bf5887fe89c32c6be8dd6) | `c1872c4` | `Core/DeviceManager.cs`、`Core/Models.cs`、`Interop/HidNative.cs`；`Providers/` 中 Vxe、ATKA9Airadd、Rapoo、Razer、Mchose、AsusHeadset、ATKN9V2Ultimate、MchoseV9Headset、DualSense、XInput 实现 | MIT，Copyright (c) 2026 Joko |
| [HeadsetControl](https://github.com/Sapd/HeadsetControl/tree/62e361b085d7716805a3bf524bc9cca3761e0e15) | `62e361b` | `lib/devices/steelseries_arctis_nova_5.hpp`、`steelseries_arctis_nova_7.hpp`、`steelseries_arctis_nova_7p.hpp`、`steelseries_arctis_nova_3p_wireless.hpp`、`protocols/steelseries_protocol.hpp`；Nova 型号、状态布局和电量代际 | GPL-3.0 |
| [Linux hid-playstation](https://github.com/torvalds/linux/blob/4dd10fbbeb9c33709edc89916e19c03c780a0751/drivers/hid/hid-playstation.c) | `4dd10fb` | DualSense USB / 蓝牙输入结构、状态位、CRC 前缀及校准 Feature 报告 | 所查文件 GPL-2.0-or-later |
| [sony-h5-tray](https://github.com/splitbrain/sony-h5-tray/tree/1d282db4ec641bfdefab1df14c3e6d05eb767706) | `1d282db` | `src/proto.rs`、`src/hid.rs`；H5 HCI 报文、校验和和公开抓取样例，与 HeadsetControl 的 INZONE 协议及 H5 / H9 II 接口表交叉核对 | MIT，Copyright (c) 2026 Andreas Gohr |
| [cherry-battery-state](https://github.com/zcmk123/cherry-battery-state/tree/af82d4d98cbad872a586302ba44e6a6b75ba65f4) | `af82d4d` | `cherry_battery.py` 的固定电量查询及字段；与 MyFinder `cherry.py`、[议题 #482](https://github.com/wushuai0802/myfinder-device-battery/issues/482) 的 Pro 型号和描述符证据交叉核对 | README 声明 MIT；所查树未含独立 LICENSE，不引入代码或资源 |

Logitech 官方 `Logitech/Logitech-HIDPP` 仓库在本次访问时返回 404，未将其内容作为已核对证据。HID++ 命令交叉核对 Solaar、libratbag 和 OpenMouse 的实际实现。

研究目录和上游完整源码仅用于本地核对，不进入发布包。协议按报文事实独立实现；直接引入上游代码、生成目录或资源前，需要单独确认对应文件的许可证和分发要求。

## 品牌目录不是一份通用协议

OpenMouse 的驱动注册表含 51 个不同的品牌标签：

AJAZZ、Amkette、ASUS、ATK、Attack Shark、AULA、Cooler Master、Corsair、Dareu、Delux、Endgame Gear、Fantech、Fater、Finalmouse、G-Wolves、Glorious、GravaStar、Hitscan、HyperX、Incott、IPI、K-snake、Keychron、Lamzu、Lingbao、Logitech、MCHOSE、Microsoft、moddoMOUSE、Motospeed、Ninjutso、Noir Gear、Orbital、Pulsar、Rapoo、RAWM、Razer、Redgear、Redragon、ROCCAT、Ryunix、Scyrox、SteelSeries、Teevolution、Thermaltake、VAXEE、VGN、WALLHACK、WLMouse、Wooting、Zaunkoenig。

这是注册表清点结果，不是所有驱动已经逐项完成深入验证。同一品牌存在多个控制器或协议代际，动态识别还可能返回注册表以外的品牌标签。例如 ATK 的 CID/MID 目录同时覆盖 VXE；部分 Lamzu 协议设备返回其他品牌身份。

FerrisPulse 还覆盖耳机、键盘和手柄，实际 Helper 包括 NuPhy、MCHOSE、Flydigi、GameSir、Sony/Inzone、HyperX、SteelSeries、Edifier 等。MyFinder provider 目录还包括 Audeze、Astro、JBL、NZXT、Lofree、Cherry、Moondrop、Bluetooth、XInput 等。电量 provider 的存在不证明它提供 DPI、回报率或设备设置写入。

## 已深入核对的协议家族

| 家族 / 上游实例 | 身份与通道 | 电量与状态 | DPI / 档位 / 回报率 | 接入边界 |
| --- | --- | --- | --- | --- |
| Logitech HID++；LIGHTSPEED、Unifying、Bolt 的不同设备 | 查询 Feature Discovery；接收器实例 + 配对槽位 + unit ID 区分身份；短报告 `0x10`、长报告 `0x11` | `0x1004`、`0x1000`、旧寄存器 `0x0D/0x07`；查询回复和 SoftwareID=0 的通知分流 | `0x2201` / `0x2202` 当前 DPI 与数值范围；`0x8100` 板载配置及当前 DPI 档；`0x8060` / `0x8061` 回报率 | 一套 feature 查询可覆盖多型号；新板载格式和某些耳机协议仍须分别适配 |
| CompX/Beken EEPROM16；VGN F2 Master+，ATK F1、X1、A9，VXE R1 部分型号 | Report `0x08`，16 字节载荷；VGN 示例 `3554:FB56/FB57`，ATK/VXE 还涉及 `373B`；描述符及运行时 CID/MID 必须匹配 | 命令 `0x04` 电量；握手、online 与配置读取分开；接收器需节流 | 命令 `0x08` 读地址、`0x07` 写地址；档位数量 `0x02`、当前档 `0x04`、旧四字节档位表 `0x0C`；回报率标量 `0x00` | 能复用传输、校验和及标量操作；不能把旧传感器编码套到所有型号 |
| PAW3955 等新布局；ATK 新型号 | 仍可能沿用 EEPROM16 传输，按传感器和运行时身份选布局 | 可共享传输层，字段解释另行校验 | OpenMouse 的 PAW3955 档位表使用 `0x1B00`；官方 VGN 编码也区分四字节和六字节记录 | 共用 VID、报文长度或品牌不足以选择 DPI 编码 |
| YJX；FerrisPulse VGN/Zaopin 路径 | 另一套 65 字节控制帧；请求 `00 55`、回复 `00 AA`，控制集合与 EEPROM16 不同 | 查询 performance / profile 后解析状态 | 先读取原配置，再针对指定字段修改；不能用 EEPROM16 写法 | 同一品牌的另一协议，不与前述家族混用 |
| Razer 常见功能报告；Viper、DeathAdder、Basilisk、Naga、Orochi 的部分代际 | 通常 90 字节载荷，Windows API 缓冲区额外含 report ID；transaction ID、命令类、命令 ID、长度及校验需匹配 | 电量 `07:80` 原始值为 0–255；充电 `07:84`；busy 使用有上限的重试 | 当前 XY `04:85`，写入 `04:05`；档位表查询 `04:86`、写入 `04:06`；旧单字节 DPI 和新回报率命令另有变体 | 批量覆盖已确认命令代际；V4 等新代际不自动等同于旧协议 |
| Lamzu / WLMouse 的部分 CompX page-command 型号 | 区分 mouse、dongle、pairing target；page、command、状态及长度共同匹配 | 电量 page 0、command `0x83`；Lamzu 部分设备在另一个 input 集合发通知，去抖后触发刷新 | 读取档位表；当前 DPI、某一档位数值、档位数量和回报率使用各自命令 | WLMouse Beast X 4K `36A7:A887` 在上游明确使用另一套 Report 4 协议；相似描述符不代表同协议 |
| Corsair Bragi；IRONCLAW RGB WIRELESS、SCIMITAR RGB ELITE 上游实例 | `FF42` 带 output 的控制集合；64/128 字节报文；接收器槽位和命令匹配 | 用同一个串行队列接收回复；input-only 的通知集合不是控制通道 | 上游所查驱动明确只读；某些配置可读取，但持久性和硬件模式的写入行为不一致 | 不将已编码的写命令当作可发布的设置能力 |
| SteelSeries Aerox 3 Wireless 上游实例 | `FFC0` 控制集合；无线命令与有线命令有标志差异 | 电量可读回，需区别异步通知 | 所查实现的部分 DPI、档位和回报率来自缓存默认值及写入后缓存，并非设备读回 | 可借鉴电量命令；不能把默认值宣称为实际板载档位 |
| orpheus 协议抽象 | 按 Feature64、EEPROM16、IPI Pix、HID++、Razer 分类；传输为可替换接口 | 不同协议分别实现电量 | 所查版本只有 EEPROM16 和 IPI Pix 提供 DPI 读写；HID++ / Razer 部分回报率读取也明确未实现 | “可识别 / 能写某设置”与“能读回实际设置”必须独立声明 |

## DPI 的数据与操作

| 数据 / 操作 | 含义 | 本项目处理要求 |
| --- | --- | --- |
| 允许的 DPI 数值、范围和步进 | 传感器可接受的输入，不是用户保存的档位 | 设备已声明的数值才可写；不把 400/800/1600 的快捷按钮称为板载档位 |
| 当前 X/Y DPI | 正在生效的分辨率；可能由默认值字段表示 | 保存 X/Y；CompX 逐档独立编辑，Logitech 单值界面遇到不同轴值保持只读 |
| 实际 DPI 档位表 | 配置中保存的各档位；允许禁用档和重复值 | 保留协议原始编号，不能排序、去重后当成档位表 |
| 当前档位 | 当前激活的原始索引 | 切换索引与修改某档数值分开处理 |
| 修改档位数值 | 修改一个配置记录；有时写入 EEPROM / Flash | 先读原配置，保留其他字段；校验 ACK，再读回；失败不得报告成功 |
| 回报率 | 当前传输的频率和允许值 | 区分 USB / 2.4G / Bluetooth 的限制；例如 HID++ `0x8061` 的能力列表带连接类型参数 |

Non-Synapse-Mouse 的部分“Profile”通过当前 DPI 与应用内预设比较推断。SteelSeries 所查驱动的部分设置来自默认值缓存。这些方案可用于它们的交互，但不能作为本项目“已读到真实档位”的证据。

## 多品牌状态回报

1. **身份、连接、电量、DPI 分开保存。** MyFinder 的 `cached`、`source_quality`、`measured_at_utc`、`queried_at_utc`、`connection_known` 明确区分读数和连接证据。无线设备没有应答可能是休眠、超时或被官方驱动占用。
2. **主动查询和被动通知共存。** Solaar 将 HID++ SoftwareID=0 的通知送到通知处理器，只有匹配地址、feature 和软件 ID 的回帧才能完成请求。Lamzu 的部分状态通知来自兄弟集合；通知应触发局部刷新，不能当作另一个设备。
3. **持续回报协议保持监听。** Rapoo 的状态不是每次打开接口后立即返回，监听必须跨刷新周期。保存最新有效采样及其测量时间；解析失败不能伪造新电量。
4. **同一物理通道内串行，不同设备独立。** FerrisPulse 的统一命令总线可避免查询与设置抢包，但 VID:PID 作为锁键会合并两台同型号设备。本项目后续通道所有权应按接收器实例及物理控制接口分组，配对槽位也需参与命令匹配。
5. **按协议设超时、节流和恢复。** 所查实现使用数百毫秒至数秒的不同超时，接收器连续读之间可能需留间隔。统一 160ms 不适合首次无线握手。重试必须有上限；静态身份和能力不必每轮重新查询。
6. **缓存不能授权写入或触发新的低电量告警。** 缓存只供带明确标签的展示；过期隐藏数字。写入前重新确认设备身份、模式及当前配置，不能因为卡片仍在就沿用旧授权。

## 1.4.5 已实现的部分

- 共用轮询结果保留本轮接口枚举证据；读不到电量或枚举失败不再等同于设备拔出。
- 枚举中暂时无法打开的接口仍作为存在证据；标准 HID 缓存使用真实采样时间，重复发布不延长其寿命。
- 雷柏持续回报使用独立的两秒界面发布周期，不受默认六十秒主动查询间隔限制；实机连续验证超过八十三秒，三十九次状态合并未出现电量未知。
- 新增电量状态的 Live / Cached / Stale / Unknown 及测量、查询时间；短暂失败保留上次有效值，超过各适配器的期限隐藏数字。
- 待确认状态关闭 DPI 数值和档位写入，缓存电量不触发低电量通知；替换到同一接收器槽位的新设备不继承旧设备在线状态。
- Logitech 同一 Windows HID 接口组优先选择长报告集合，并保留不同接收器实例；首次 ping 与普通命令分别设置超时。
- 新增 `0x2202` 当前 DPI / Y 轴和支持列表读取，以及受校验的单值写入；保留抬升距离，未提供 Y 轴时写零，写后读回及失败恢复均验证。
- 保留原有 `0x2201` 和已支持 `0x8100` 板载格式 1–5 的实际档位功能；未开放新板载格式的 Flash 写入。
- 新增 CompX Report 8 / EEPROM16 鼠标适配器，按控制集合、运行时 CID/MID 和传感器布局读取电量、实际 X/Y 档位及当前配置；范围与传输例外见[兼容性说明](COMPATIBILITY.md)。
- CompX 写入分别处理切档、修改档位记录和回报率；F58A 保留完整系统行及相邻档位，操作前复核原值，操作后读回并再次确认配置。
- 新增独立 `PollingRate` 能力和主窗口控件。Logitech `0x8060` 按毫秒位图解析，`0x8061` 读取带连接类型、设置仅传回报率编码；不自动改变板载模式。
- CompX / Logitech 的本进程通信按物理控制通道串行化；编辑草稿与设备、配置、原始 X/Y 数值绑定。进程外竞争仍通过原值复核与写后读回检测，不能保证和第三方软件之间的原子写入。
- Razer 的读写共用功能报告实现，连接能力单独存表。通过序列号散列、正常模式、当前配置和读回确认授权设置；每个命令分别确认事务编号，不能因 DPI 查询成功就沿用同一编号查询档位。
- 统一 `StageContext` 区分真实 Profile 与完整配置指纹；Razer 没有 Profile 编号时也能防止提交旧草稿，不再把存储选择字段伪装成 Profile。上游实测档位回复的一起始记录、激活索引、零存储字段与零起始写入格式分别处理。
- Lamzu / WLMouse page-command 家族已接入只读查询，已核对布局可读实际 Profile、完整 X/Y 档位、当前档位及回报率。共用接收器先查配对型号；不同布局、未知身份保持电量只读，Beast X 4K 与升级模式排除。
- ATK N9 V2 Ultimate / MCHOSE V9 Turbo+ 共用一个 FF90 耳机电量适配器，拒绝回显和未知状态，不因充满标志补成 100%。

2026-11-17 本地 148 项回归测试通过，Windows x64 发布构建完成且无编译警告，覆盖 Logitech 回报率、DPI 草稿与并发保护、CompX 回显、Razer 设置与读回、page-command 完整配置和耳机电量。新增鼠标与耳机能力仍待本项目实机验证，不能将模拟测试等同于硬件兼容。总体目标尚未完成，后续缺口见本文末尾。

## Voltide 对照与落实

Voltide 的价值主要在 Windows 传输例外和社区回帧，DPI、档位、回报率仍以 OpenMouse、Solaar、libratbag、OpenRazer 的专门实现交叉核对。所查版本注册 23 个 provider；这不是 23 个品牌完整支持或本项目硬件验证的证明。

| 实际源码发现 | 与此前项目的关系 | 本项目处理 |
| --- | --- | --- |
| `VxeProvider.cs`：`373B:10C9` 的 17 字节集合要求 SetOutputReport，先回显请求再给电量；20 字节兄弟集合不能替代 | 补充 OpenMouse 的 EEPROM16 协议在 Windows 上的具体传输条件 | 为 `10C9` 与 ATK `1278` 增加限时、可取消的控制报告传输；校验并跳过回显，继续等有效回复；这些连接保持只读 |
| `RapooProvider.cs`：VT3S `24AE:1411` 持续保持只读句柄，报告 7；复用旧采样时保留时间 | 与 MyFinder、FerrisPulse 及本项目 VT1 的持续监听方向一致 | 保留本项目按接口监听、独立发布及采样过期机制；不照搬单个全局 session 或把旧采样刷新成实时 |
| `DeviceManager.cs`：一次枚举供各 provider 使用；其约 400ms 的说明是上游环境记录 | 减少重复设备树扫描，与品牌通信并行不冲突 | 共享 500ms 描述符快照，含暂时打不开的接口；枚举失败仍为错误。设备查询继续独立，不采用全局串行刷新 |
| ATK N9 V2 与 MCHOSE V9 Turbo+：`FF90`、64 字节、`55:65`，WriteFile，目标耳机 `01` | 相同协议可跨品牌；ATK 鼠标控制报告经验不能直接套用耳机 | 已用共用适配器接入并通过模拟报文测试；上游注释提供实机/社区依据，本项目实测待完成 |
| DualSense USB/蓝牙报告布局不同；XInput 返回四档等级 | 扩展需要协议之外的数据精度模型 | DualSense 已交叉核对 Linux；XInput 已核对微软 API 文档并以独立系统通道接入，不把有线模式当作充电或满电 |

所查 Voltide 的 Razer 换算使用 `raw <= 100 ? raw : raw / 255 * 100`：原始 100 显示 100%，101 却约 40%。本项目依据 OpenRazer 与 Non-Synapse 始终按 0–255 换算，并另查充电命令。还检查 transaction ID、命令类、命令、数据长度与 XOR，避免只匹配事务号就接收旧报告。部分 Voltide provider 按 PID 去重，会合并同型号接收器；本项目保留 Windows 实例身份。所查仓库未发现独立测试文件，因此注释中的实机记录仍按上游证据标注。

Razer 适配接入统一状态和能力模型，支持有效电量、独立充电查询、当前 X/Y、完整档位以及两代回报率解析。新增设置按已核对的连接数据开放，不从读取成功推导传感器上限、Profile 或可写回报率列表。例如 `00A6` 原配 1K 接收器使用扩展回报率命令，而 `00B7/00B8` 使用旧格式；不能按“无线”或“高回报率”猜测代际。`00C1` 的持久化需要额外 selector 1 / transaction `1F`，完成实时读回和身份复核后才提交。静默不永久记为不支持，断电持久性仍待实机测试。

## 后续接入顺序

优先完成多品牌电量和基础 DPI，ROG 保留深度适配；其他品牌的高级设置暂不优先于键盘、耳机电量覆盖。

- [x] 按设备保存 1～4 档自定义 DPI 预设，编辑后按数值升序排列；选择启用档数，收起最高档并保留数值，主窗口与托盘共用启用列表。
- [x] 区分设备真实档位和软件预设；只有已确认数值写入能力的设备开放预设切换，按实际范围和步进校验，写后读回确认。
- [x] 明确提示会修改板载 DPI 的协议；预设不能绕过尚未实现的 DPI 写入，身份暂缺、配置变化和离线时不沿用写入授权。
- [x] 接入雷柏 nRF54L 当前配置的完整 X/Y 档位读取与基础切换，协议回归通过；实机切档、休眠与官方驱动共存验收待完成。
- [ ] 优先接入上游有明确实现的多品牌电量协议，分别记录鼠标、键盘、耳机和手柄的覆盖及验证状态。
- [x] 接入迈从 20 字节键盘状态与 Z75S 型号握手，Arctis Nova 百分比 / 等级状态；协议与刷新回归通过，对应硬件验收待完成。
- [x] 接入 DualSense / Edge USB、蓝牙电量，区间数字标注 ≈，检查报告类型、长度及 CRC；协议回归通过，对应硬件验收待完成。
- [x] 接入 XInput 系统槽位、有效无线电池等级与非 HID 连接证据，明确无电池时清除旧数字；回归通过，对应硬件验收待完成。
- [x] 接入 Sony INZONE H5 / H9 II 接收器电量，按型号限制接口，核对事务、来源、校验和与离线回报；对应硬件验收待完成。
- [x] 接入 HyperX Cloud Alpha 2 基站电量，分开解析充电与电压，前后确认连接并过滤无关消息；对应硬件验收待完成。
- [x] 接入 CHERRY MX 2.0S WL Pro 的固定 USB / 接收器电量查询，增加按钮及数值 caps 的报告 ID 验证；对应硬件验收待完成。

协议与验证方面继续推进：

1. 继续 CompX / Logitech / Razer 实机验证，覆盖修改、休眠恢复、换设备及并行官方驱动；现有本地构建和报文回归已通过，硬件缺席时不宣称完成。
2. 扩展 CompX/Beken 的运行时身份与传输数据，Logitech 新板载布局、静态 feature / 身份缓存与持续通知分流；键盘协议保持独立。
3. 扩充 Razer 连接能力数据与代际识别；共用高频接收器需确认配对鼠标和传输能力，不能套用已有 PID 的设置范围。
4. 在 Lamzu / WLMouse page-command 读取基础上补足状态通知与受校验写入；Beast X 4K 等协议例外单独处理。
5. Corsair、SteelSeries 及更多耳机 / 手柄电量家族，根据实际可读回能力接入，不用目录默认值填充未知设置。
6. 完善可分发的兼容性数据、无需 GitHub 账号的反馈服务和 Beta 更新机制；现有入口仍是用户自行提交的 GitHub 表单与脱敏报告。

型号目录逐步改为描述符、运行时身份、传输、传感器及协议布局的数据表；没有证据的组合保持未知，不靠扩大 VID/PID 范围试写设置。

1.4.6 自定义预设：本地回归及中英文界面渲染检查已通过；新交互与设置的逐型号实机验收仍待完成。CompX 预设绑定点击时的激活档，写前后复核档位及 X/Y。雷柏 nRF54L 官方现版驱动按配置地址区分最多 7 档，与部分参考实现的固定地址、6 档不同；现已按实际配置读取完整 X/Y 表，未套用旧 VT9Pro 协议。

## 1.4.6：雷柏 nRF54L 档位

依据 [Rapoo A HUB](https://hub.rapoo.cn/) 当前网页驱动的地址表和读写逻辑，独立实现配置寄存器访问。配置号来自 0x010C；各配置基址为 0x0600 + 配置号 × 0x0400，X/Y 表偏移分别为 648/712，启用数为 662（存储值加一），激活档为 664。表含最多七档，保留独立轴、重复值、未启用项和保留字节。无线选择 A5、有线选择 FF；旧 VT9Pro 的 BA 协议不复用。

FF00:000E Output 33 字节和 FF00:000F Feature 33 字节按 Windows 实例与 USB 接口配对，同型号接收器不合并。Output 使用写权限，Feature 使用零访问权限，均限时且等待取消完成。状态接口继续由独立线程持续读取。

公开实现未确认 ACK 的地址或事务回显，因此读取前后检查配置号，逆序重复读取两轴，并与状态报告的档位及两轴相符后才显示可切换。操作绑定完整配置指纹；只写一个激活索引，失败不重发或盲目回滚，读回整表后再等新的状态报告。查询失败保留独立电量；两秒通信预算限制单次配置读取。未开放雷柏数值编辑或软件预设写入。

此实现仍需实机验证。协议模拟不能证明驱动并发时绝无竞态，也不能证明各固件都使用相同布局。当前官方驱动是协议事实依据，未分发其脚本。

## 1.4.6：键盘与耳机电量

迈从键盘结合 Voltide `MchoseProvider.cs` 的 Windows SET_REPORT 与抓包字段，以及 FerrisPulse `MchoseDeviceHelper.cs` 的 Z75S 型号握手。两者都使用 20 字节 Report 13，但 FA09 的实测控制传输与 Z75S 的中断 OUT 分开处理，不在超时后盲目换端点重发。电量应答必须为 `13 4A 01 00 02` 开头，末字节为前 19 字节之和；第 5 字节为百分比，第 6 字节高半字节为充电标志。请求回显载荷长为零，不是 0% 读数。共用 PID 不足以确认具体键盘型号。

Arctis Nova 的型号与状态偏移由 HeadsetControl 交叉核对，Windows 65 字节传输和 Nova 5 报文由 FerrisPulse 提供补充证据。去掉 Windows report ID 0 后，B0 命令的 Nova 5 / 3 Wireless 布局分别在偏移 1、3、4 提供连接、百分比和充电；Nova 7 在偏移 2、3 提供电量和状态。Nova 7 的等级 / 百分比语义按 PID 决定，不能用原始值小于等于 4 来猜测。旧型号的 0～4 保留原始等级，并以 0/25/50/75/100 作为带 ≈ 的显示估值；不写入精确测量字段。未确认状态拒绝或保留未知，不把任意非离线值当作有效连接。

这批实现只读电量，沿用每接口串行通信、采样时间、缓存过期和脱敏诊断。不引入上游代码或资源。范围、传输限制和未完成实机验证见兼容性文档。

后续手柄研究发现 Voltide 的 XInput 适配把有线连接显示为 100% 充电，并把四级电量映射成未标注的百分比；有线模式不证明满电或充电，等级换算必须带 ≈，不当作精确测量。

## 1.4.7：DualSense 电量

已交叉核对 Voltide 的 Windows 通道行为与 Linux `hid-playstation.c` 的报告结构、状态解析和 CRC。USB 输入为 Report `01` / 64 字节，状态偏移 53；蓝牙完整输入为 Report `31` / 78 字节，状态偏移 54，末四字节保存小端 CRC32。计算先放入前缀 `A1`，再放入前 74 字节；初始化全 1、最终取反。精简蓝牙 Report `01` 不含电量，即使 Windows 补齐缓冲长度也不能按 USB 偏移读取。

状态低半字节 0～9 对应十个百分点区间，取中间值 5～95；10 为 100。上半字节 0 放电、1 充电、2 已充满；A/B/F 表示故障，不借用 Linux 的容量 0 作为有效电量。所有区间值放入估算字段，数字显示始终带 ≈。无效值或损坏报文不替换有效采样、不更新其时间。

输入句柄只读，打开后先等输入；仅观察到蓝牙精简报告时查询 41 字节校准 Feature `05`，检查独立前缀 `A3` 的 CRC，随后等待完整输入。功能报告本身不提供电量，也不授权设置。没有 Output / SetFeature、灯光、震动或按键设置操作。单次输入接收同时限制总时长与报文数，避免无效报告流阻塞其他设备。

适配器轮询等待上限为两秒，电量有效期八秒，超时保留原测量时间并按期隐藏；新报告恢复数字。Windows 实例隔离同型号设备；跨 USB / 蓝牙的同一设备去重尚未接入。诊断不保留输入载荷。本机没有对应手柄，实机传输、休眠及 Steam / DS4Windows 共存验收待完成。

## 1.4.7：XInput 系统电量

依据微软 [XInputGetBatteryInformation](https://learn.microsoft.com/en-us/windows/win32/api/xinput/nf-xinput-xinputgetbatteryinformation) 与 [XINPUT_BATTERY_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/xinput/ns-xinput-xinput_battery_information) 文档核对 Voltide 的实现。电池类型 0 为断开、1 为有线、2 为碱性、3 为镍氢、FF 为未知。只有已知无线电池类型的等级有效；0～3 分别为空、低、中、满，没有百分比阈值或充电状态。

适配器只从 System32 加载系统 XInput 1.4；支持的 Windows 10/11 自带此 API，不安装额外库，也不加载应用目录中的游戏包装 DLL。读取状态只确认连接，按键和摇杆数据立即丢弃；随后查询手柄电池，不查询耳机槽或调用震动设置。四个槽位分别读取，一个槽位的其他错误不能把它误判为拔出，也不影响其他槽位。

设备地址现在区分 HID 接口与 XInput 槽位；现有 HID 识别键和品牌协议沿用原逻辑。XInput 的连接证据来自 API 返回值，不来自 HID 枚举。新增明确无电池标志，与暂时没读到电量分开，有线或未知电池类型清除旧采样；普通失败不会延长采样寿命。显示代表值为 0/20/50/100，始终带 ≈，不进入精确百分比通知。

XInput 不提供型号或持久物理 ID，不使用 VID/PID 猜测槽位，也不推断虚拟控制器对应哪个 HID 设备。偏好按系统槽位保存，快速替换和跨协议重复项的边界已写入兼容性说明。本地 192 项回归及构建通过；本机系统 API 正常，暂无已连接手柄可做电量实测。

## 1.4.7：Sony INZONE 电量

HeadsetControl 的 `sony_inzone_protocol.hpp`、H5 / H9 II 型号定义和接口选择测试，与 sony-h5-tray 的公开 H5 报文交叉核对。H5 使用 `FF04:0002`，H9 II 使用 `FF04:0001` / `MI_05`。FerrisPulse 的 H5 路径会在 usage 1 未找到时退回任意 64 字节输出集合，本项目不采用这一宽泛回退，也不访问 `FF13` 升级接口。

64 字节报告 `02` 内封装 Sony HCI，GET 电量请求事件 `04`、类型 `01`、地址 `41`。请求校验和为字节 6～12 的和；回复长度为 14，地址 `14`，事件 `04`，返回类型 `10` 或相同事务的 `A0`，校验和为字节 5～14 的和。查询编号跳过 0 和主动通知使用的 1；HCI 的参数长度字段在上游按信息字段处理，外围固定长度、来源、事务及校验必须同时通过。

载荷依次是充电标志与百分比。0～100 保留设备百分比；`FF` 标记离线 / 无缓存值；101～254 拒绝。充电只认 0/1，不将未知非零字节或满电当成充电。超时不刷新采样时间，明确离线清除数字，后续有效报告恢复。Buds 有多个电量组件，不能套用这两字节结构，因此未加入此适配器。

本次独立实现仅含状态查询，复用每接口串行化和现有生命周期。协议与状态回归通过，实机验收待完成；公开报文与合成异常测试不是本机硬件抓取。

## 1.4.7：HyperX Cloud Alpha 2 电量

核对 HeadsetControl `62e361b` 的 `hyperx_cloud_alpha_2_wireless.hpp` 与对应测试，以及 MyFinder `73d99e2` 的 `hyperx_alpha2.py` 与测试。两者公开报文共同确认控制基站 `03F0:08BE`、`FF13:FF00`、64 字节和 `50 02` / `51 02`，不能读取 `0ABE` 音频端，也不能用恒定的 `61 02` 音频设置当成电量。

发现 MyFinder 将字节 6 的 bit 7 当成充电状态；HeadsetControl 带独立充电实测的报文表明字节 3 才是充电状态，6/7 是小端电压。公开 51% / 67% 报文中的 `00 0F` / `86 0F` 对应 3840 / 3974 mV，不能据此推断充电；91% 的充电报文则在字节 3 明确返回 1。实现保留这些原始报文事实为回归输入，充电未知值不猜测。

连接请求 `52 01` 返回 `53 01` 加 0/1；确认包 `FF 01` 和拒绝包 `FF 02` 在偏移 14/15 回显命令。只把匹配的拒绝作为当前查询失败；ACK、`FB` 通知和其他命令继续过滤。电量前后均查询连接，防止关机期间发布基站旧值。此协议缺少事务号与校验和，无法保证其他进程同命令回复的归属，必须通过 NGENUITY 共存验收确认实际行为。

NuPhy 原厂键盘的接入暂未扩大：FerrisPulse 的 `55` / `AA` 路径会用默认模式和缓存设置补全未知值；另查 Slone123c/nuphy-air75-v2-web-panel `02c54f3094afc655a84271159053a19abd83f1fb`，其 README 明确为自定义固件，不能用来证明 Air V2 原厂固件支持同一电量命令。后续须补齐型号、描述符及有效回复证据。

## 1.4.7：CHERRY MX 2.0S WL Pro

原项目 `af82d4d` 仅用于核对固定输出命令及字节含义；实际 Pro 型号的 USB / 接收器身份、`MI_01` / `FF1C:0092` 和双向 Report `04` 依据 MyFinder `73d99e2` 与议题 #482。型号目录没有扩展成整个 Cherry VID；原项目所述 `01AC` 与蓝牙 `0687:01DE` 均不沿用 Pro 私有查询。

描述符读取通过 access=0 句柄完成，同时枚举按钮和数值 caps。任一方向失败、报告 ID 混合、长度不符或候选集合不唯一时不发送请求。管理集合检查按 Windows USB 接口组进行，尚未引入跨接口的物理 parent 查询；不同实例的同型号设备分别保存身份。

有效回复限制为 64 字节、固定前缀、状态 0、1～100 百分比和 0/1 充电字节。0% 与全零回显无法可靠区分，保持未知且不刷新时间。公开贡献者记录曾在切换连接后与官方 UI 短时相差 3～4 个百分点；程序不将设备读数修正成界面截图，硬件长期稳定性仍待验证。

2026-11-21 本地 211 项回归通过，Windows x64 构建无警告；新增 caps API 正常加载，既有 ROG 鼠标和 VGN 键盘仍返回有效读数。没有 Cherry 实机，本次测试不证明低电量、停止充电、长休眠和官方驱动共存行为。
