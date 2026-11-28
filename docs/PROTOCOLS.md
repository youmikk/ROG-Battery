# 多品牌协议研究与接入依据

[English](PROTOCOLS.en.md) · [已实现兼容性](COMPATIBILITY.md)

## 适配证据与验证

实现、合成测试、上游硬件记录和本项目真机验收是不同证据。下表归纳部分协议的来源与待验证点，不代表完整型号清单；兼容范围见[兼容性](COMPATIBILITY.md)，具体命令、接口和固定源码见各型号章节。固件版本未明确记录时，不能推断所有固件兼容。

VigilWatt 来源固定为 [`c97adc389b60b0e945a62cddacbaaace39887723`](https://github.com/evren-oct/vigil-watt/tree/c97adc389b60b0e945a62cddacbaaace39887723)；HyperX 的交叉来源与固定提交见下方型号章节。许可与归属见[第三方声明](../THIRD_PARTY_NOTICES.md)。

| 型号与模式 | 上游证据 | 本项目验证与待验证点 |
| --- | --- | --- |
| [高斯 GANSS GS3104T-Pro](#高斯-ganss-gs3104t-pro)，接收器 `05AC:024F` | VigilWatt 完整 97% 捕获；Linux 提交独立记录精确产品串 | 捕获回放、合成损坏与边界、同型号隔离；尚无真机验收，完整描述符、1%、休眠、其他固件和驱动共存待验 |
| [HyperX Cloud Alpha Wireless](#hyperx-cloud-alpha-wireless)，接收器 `03F0:098D` | 多个 Windows 开源实现提供接口、容量与命令证据 | 合成回复、回显、旧型号与多设备回归；尚无真机验收，逐报告描述符、0%、充电百分比冻结、关机缓存和驱动共存待验 |
| [狼途 LANGTU T98](#狼途-langtu-t98)，接收器 `1A2C:8FFF` | 单台 Windows 接口观测、78/87/FF 前缀及休眠记录 | 前缀重建、合成解析与生命周期回归；尚无真机验收，描述符、其他固件、0%、同型号双实例和驱动共存待验 |
| [CHERRY PIXIU75](#cherry-pixiu75)，USB `046A:01E2` / 2.4G `046A:01E4` | Windows 接口及与官方软件的读数对照 | 前缀重建、解析、通道与原 MX Pro 回归；尚无真机验收，0%、长休眠、多实例、充电和跨模式身份待验 |
| [雷神 ML903 NearLink Se](#雷神-ml903-nearlink-se)，接收器 `369B:0234` | 35% 原始前缀；27%、关机静默及恢复观察 | 前缀回放与合成回归；尚无真机验收，实际描述符、不同固件、充电、双实例和官方驱动共存待验 |
| [AKG N9 Hybrid](#akg-n9-hybrid)，接收器 `0ECB:20D9` | 完整 62 字节 50% 帧及六轮自然轮询 100% | 实帧、分片、ACK、旧缓存与实例回归；尚无真机验收，持续放电、关机和官方软件并发待验；协议无事务号 |
| 北通鲲鹏20，`20BC:5159/5158` | VigilWatt 无线 39% 前缀、有线 78% 前缀 | 暂缓、未启用；仅独立研究测试，未编入产品。输入集合边界、关机后缓存广播及 XInput 槽位与物理实例对应尚未确认 |

后续准入要求：

1. 固定上游版本与具体源码，分别记录连接模式、接口指纹及已验证字段；VID/PID 目录不足以证明兼容。
2. 每个实帧注明来源；补零、改写及模拟通道均标为合成测试。
3. 分别检查异形帧、未知字段、静默和断连；失败不能显示为 0%，旧采样不能刷新时间。
4. 同型号双设备、跨品牌、插拔、休眠和官方软件共存须经实机验证。
5. 北通须先补无线集合描述符、完整开关机序列及可靠 XInput 关联；不按枚举顺序、型号或电量猜测槽位，不屏蔽无关手柄。

[分步骤采集](guided-capture.md) 旁路记录已发现设备的既有通信，并为正常使用、充电、关机、恢复添加时间标记，不主动探测未适配设备。有数据时默认附带到用户提交的反馈，提交前可关闭，公开议题提供下载链接。采集成功不等于协议已经解释正确。

## 英菲克 IN9

来源为 VigilWatt 固定提交 `c97adc389b60b0e945a62cddacbaaace39887723` 的 [英菲克硬件记录 §2](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/brands/inphic.md)、`InphicIn9Codec.cs`、`InphicIn9Plugin.cs` 和 `InphicIn9CodecTests.cs`，另核对[贡献者公开采集记录 #40](https://github.com/evren-oct/vigil-watt/issues/40)。公开记录包含 `MI_01 Col04 / FF00:0001 / in=0 out=0 feat=33` 和 `SET_REPORT wValue=030C wIndex=1 wLength=33`；完整 pcapng 由上游本机留存，未公开原始完整描述符。唯一 Feature ID `0C` 是运行时准入要求，尚待本项目实机确认。

固定刷新帧为 `0C 01 20 00 <序号>`，补零到33字节。序号1～255循环，重复序号可能使接收器返回空寄存器，因此每轮先写、等待30ms再读；无有效帧时等待500ms换号重试一次，随后只等待80/160ms重读。失败写入不回读缓存，最多两写四读；单次传输上限400ms，整轮预算3.2秒，超时结果不发布。打开句柄、枚举及内核取消收尾不作硬实时保证。有效轮次间隔至少10秒，连续失败按10/20/40/60秒退避。

上游完整放电帧 `0C012000010110004D3930393401002514FF5100FF010000B7B76238FFC767096D` 对应81%；充电帧 `0C012000030110004D3930393401002546FF6401FF01000032963238FFC767096D` 的 `[18]=100、[19]=1、[21]=1`，百分比是占位值。解析要求完整33字节、头 `0C012000` 和 `[8..13]=M9094`；`[19]=1` 仅报充电，`[19]=0/[21]=1` 才采用1～100%的放电电量，其余状态清旧样本。不单独解释 `[21]=3`，也不扫描其他有线设备推测充电。未知状态及非100充电百分比的测试向量为合成数据，不能视为已记录的固件行为。

序号 `[4]` 只是写入寄存器，不是无线回复的事务凭证。上游明确记录关机后接收器可能仍回旧值，本实现无法据此判断鼠标关机，也不能保证每个格式正确的返回值都是新测量。每设备序号/退避独立，发现接口消失时清理调度状态；正常查询间隔内可重复发布上次成功采样，保留原测量时间，真实查询失败即停止重发缓存。前后校验接口、按路径与序列号核对目标，描述符失败不发送命令，沿用原始通信采集与通信摘要。

测试中的上述两帧来自上游；损坏、未回填、零电量、重试、超时、多设备及界面状态为合成回归。实机验收需覆盖冷启动、反复启动与序号重复、双接收器、充电/拔线、睡眠/唤醒、同路径重插、固件差异与官方驱动同时运行。

## 墨将烈风 2（MOJHON GALE2）

来源为 VigilWatt 固定提交 `c97adc389b60b0e945a62cddacbaaace39887723` 的 [Mojhon 硬件记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/brands/mojhon.md)、`MojhonCodec.cs`、`MojhonPlugin.cs` 及 `captures/mojhon-vendor-frames.jsonl`。上游提供 Windows 接口、报告描述符、USBPcap 记录及与官方助手的充放电对照；本项目独立接入现有传输和生命周期。

只匹配 `E2B7:122C / MI_02 / FFAB:0001`、无编号 65 字节双向报告。固定读请求为 `00 A5 04 <寄存器> <校验>` 并补齐容量；`E2` 查询身份，`04` 查询电量。校验为协议帧除末字节外的累加低字节，首个 Windows ID 槽和帧外填充不参与。`C4_SERVER` / `C4_DONGLE` 来自设备身份及官方目录，不能由通用产品名 `XBOX 360 For Windows` 推断型号。

原始完整电量帧包括放电 `A5 06 04 12 00 C1`（18%）、`A5 06 04 24 00 D3`（36%）和充电 `A5 06 04 24 01 D4`（同为36%）。同电量拔线对照支持 flag 0/1 的语义；未知 flag 不推断满电。测试补上 Windows 零 ID 槽和报告填充，其他边界、损坏、路径与多设备场景为合成数据。

仅发送上述两个状态查询，电量前后确认手柄身份，空接收器清除读数。整轮预算 3.2 秒，单次交换上限 1.2 秒；处理最多32帧旧队列或候选回复，排空未完成则不发送查询。句柄打开、系统枚举和驱动取消收尾不作硬实时保证，超过预算的结果不发布。日志区分无回复、错误数据和空接收器离线，沿用原始通信采集。

身份是型号判据，不作为唯一序列号。协议没有事务号，前后核对不能排除其他程序或之前请求的迟到回复。旧句柄、同路径重新枚举、真实0%、官方助手共存、有线与无线切换仍需本项目实机验收。

## 英菲克 ZHEN

来源为 VigilWatt 固定提交 `c97adc389b60b0e945a62cddacbaaace39887723` 的 [英菲克硬件记录 §1](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/brands/inphic.md)、`InphicCodec.cs` 和 `InphicPlugin.cs`。上游记录 `1D57:FA65`、产品串 `Inphic KP 8K`，电量集合 `FF00:0001`、输入 5 字节、Report03；同设备的 `FF01:0002` 64 字节输出集合不使用。没有原始完整描述符或确定的 MI/Col 编号，运行时检查报告 ID、长度及集合，未固定猜测的接口号。

上游载荷 `76 41 01 55` / `76 41 03 54` 分别为 85% 放电、84% 充电；测试按记录的 Report03 前置得到完整 Windows 输入帧。解析只接受 5 字节和 `03 76 41` 前缀，byte4 为 0–100，byte3 仅 01/03 有明确充电含义。其他边界、损坏、并发和拔插是合成测试；未采集满电特殊编码。

只读句柄监听，5 秒 / 32 帧上限；发现只查描述符，前后重新校验描述符，按当前实例和序列号核对目标。歧义电量集合不读取，同型号共享序列号不合并。不缓存并重新发布旧报文，静默和坏帧分别记录，失败不更新采样时间；有效期 90 秒、发布间隔上限 10 秒，沿用公共后台任务、原始采集和反馈诊断。IN9 `248A:5B2F` 的刷新写入协议未复用。首帧等待、硬件缓存、真实零电量、快速拔插及厂商软件共存仍待本项目实机验证。

## Pulsar X2 V2

来源：VigilWatt 固定提交 `c97adc389b60b0e945a62cddacbaaace39887723` 的 [Pulsar 硬件记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/brands/pulsar.md)、`PulsarCodec.cs`、`PulsarPlugin.cs`、`PulsarCodecTests.cs`；交叉核对 [python-pulsar-mouse-tool `587f3a996a32aaea0f40e671756808f2f37e7fe2`](https://github.com/andrewrabert/python-pulsar-mouse-tool/blob/587f3a996a32aaea0f40e671756808f2f37e7fe2/pulsar.py) 的接口、POWER 字段及校验公式。仅使用后者的协议事实，不移植设置实现。

仅允许 `3554:F508 / MI_01/Col05 / FF02:0002`、双向唯一 `08/17`。STATUS `08 03 00…4A` 与 POWER `08 04 00…49` 均走中断 OUT；末字节等于 `55 - sum(bytes[0..16])` 的低字节。STATUS byte6 只接受 0/1，POWER byte6 为 0–100，byte7 只将 0/1 解释为放电/充电，其余未知；byte8/9 为大端毫伏，0/FFFF 不发布电压。请求回显不产生电量或离线状态。

完整上游记录：离线 STATUS `080300000001006506710000000000006D`；20% 放电、3570 mV POWER `08040000000214000DF200000000000034`。在线、其他百分比、充电及并发场景由这些记录改写并重算校验，属于合成测试。每轮 STATUS→POWER→STATUS，末次断开则丢弃电量；整轮协议等待预算 3.2 秒，单次 1.2 秒，句柄打开、排空和取消收尾不作硬实时保证。复用实例通道锁、枚举缓存和采集日志，不增加常驻线程。协议没有事务号，无法排除跨进程迟到回复；同型号双机、休眠缓存、真实 0%、实际描述符与厂商软件共存待实机验证。

## 高斯 GANSS GS3104T-Pro

来源为 MIT 许可的 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723`：[GANSS 硬件记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/brands/ganss.md)、[解析器](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Ganss/GanssCodec.cs)。上游 luze258 在 PR #15 提供官方软件捕获，维护者未复测。精确接收器产品串另由 Linux [提交说明 `0eda44edbc17d4f821690d1dc7765f14a28e4550`](https://github.com/torvalds/linux/commit/0eda44edbc17d4f821690d1dc7765f14a28e4550) 交叉确认；只使用身份事实，没有移植该驱动实现。

准入为 `GS3104T-PRO 2.4G`、`05AC:024F / MI_03 / FF60:0061`、输入输出 33 字节和两方向唯一 Report0。只发固定电量读取请求 `00 20 01 00…21`，750ms / 32帧；复用原 F108 通道锁和轮询生命周期。纯 payload 必须32字节、头 `20 01 00`、byte3为2–100且末字节等于前31字节和的低字节。GANSS校验与原F108的1–100及不要求响应校验和的规则分开。

上游完整97% payload 为 `2001006100000000000000000000000000000000000000000000000000000082`。其他边界、损坏及双实例向量为合成测试。无响应、错误校验、0/1、越界或短帧不产生新电量；充电、电压保持未知。产品串和捕获来自不同上游设备记录，仍需本项目真机联合验证描述符、低电量、休眠与驱动共存；不由同VID/PID推断其他模式或型号。

## HyperX Cloud Alpha Wireless

固定来源交叉核对：

- [HeadsetControl `32facec14215d3fb6c4b1db0d3b28e0b8d20aa5a`](https://github.com/Sapd/HeadsetControl/blob/32facec14215d3fb6c4b1db0d3b28e0b8d20aa5a/lib/devices/hyperx_cloud_alpha_wireless.hpp)：型号 `03F0:098D`，31 字节中断请求、命令 03/0B/0C 与 byte 3。该来源充电时跳过电量，单独不足以证明 Windows 集合或容量。
- [HyperXBatteryMonitor `9d01847913f4947de77e56bb05347dffc19937cd`](https://github.com/davidsantanaalves/HyperXBatteryMonitor/blob/9d01847913f4947de77e56bb05347dffc19937cd/Devices/CloudAlphaWirelessDevice.cs)：`MI_03/Col01`、Report21/markerBB、直接回复 0B/0C 与通知 25/26 的区别；同版本 `Hid/HidConnection.cs` 和 `Devices/HyperXBatteryDeviceBase.cs` 证明 Windows 中断收发与独立电量、充电查询。
- [HaloBattery `15a73ac4d88435a771b0060b28171733a43dab06`](https://github.com/HeyOkay/HaloBattery/blob/15a73ac4d88435a771b0060b28171733a43dab06/providers/hyperx_alpha_wireless.py)：集合 `FF43:0202`、interface3、按本次 selector 匹配直接回复，充电时继续查询百分比。
- [MyFinder `3d8958f734a67f8598ae39687517c2e9182830bf`](https://github.com/wushuai0802/myfinder-device-battery/blob/3d8958f734a67f8598ae39687517c2e9182830bf/scripts/HeadsetBattery.cs)：Windows 精确输入输出容量 31、Feature0、集合匹配及连接值 1/2；同版本 `docs/compatibility.md` 记有上游实机条目。
- [HyperHeadset `3652a5de38c100842e4b2f882af0e04897804638`](https://github.com/LennardKittner/HyperHeadset/blob/3652a5de38c100842e4b2f882af0e04897804638/src/devices/cloud_alpha_wireless.rs)：补证连接在线值 2 及直接回复/通知差异；其较大传输缓冲不作为 Windows 报告容量。

本项目独立编码公开协议事实，复用自有传输、描述符、锁及状态层，没有复制 GPL/AGPL 实现。运行时逐报告校验比上游集合容量检查更严格，仍需真实描述符验收。测试全部为合成向量，没有原始 Windows 抓包。

仅发布 1–100%；0 与请求回显相同，缺少独立有效标志，因此不把保护解释为离线编码。充电回复与请求完全相同时忽略并继续等待，不因本轮已有有效百分比而推断未充电；这也会保守丢弃同样字节的合法未充电回复。没有后续有效回复则保持未知，充电1可构成不含百分比的新样本。保留实际电量采样时间，后续查询不推迟时间；发布前再次确认在线。失败、通知、错误命令和旧结果重复发布不能刷新旧采样时间。

Cloud III 暂缓：同固定 HeadsetControl 的 `hyperx_cloud_3.hpp` 是有线 `089D/03CC` 的侧音设置，不是无线电量。HyperHeadset 另有 `05B7/0C9D` 的 `66` 协议，但各 PID 的 Windows Feature 后备传输和描述符证据仍未在本轮确认，不从其他型号外推。

## 狼途 LANGTU T98

固定 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723`：[硬件记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/devices/langtu.md)、[插件](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Langtu/LangtuT98Plugin.cs)、[解析器](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Langtu/LangtuT98Codec.cs)。上游贡献者 YyingnLL / haloneko，MIT 许可见 THIRD_PARTY_NOTICES.md。依据协议事实独立实现，未运行上游程序或硬件脚本。

已知接收器 `1A2C:8FFF` 的 `MI_00 / FF02:0002` 输入输出均 65 字节，约每 5.5 秒被动推送输入 Report 0。记录前缀分别为 `00 AA 55 CC 33 0E 4E 01`（78）、`...57 01`（87）和 `...FF 01`（无效）；文档说明其后补零。本地完整测试向量按此前缀重建，不称为完整抓包。运行时要求唯一输入报告，不依赖产品泛化名称推断其他型号，也不触碰 `MI_03` 通知和系统键集合。

仅被动读取，固定长度、头部、命令及零尾部全部校验。0 尚无实测，因此本实现比上游收窄为 1–100；FF 及其他越界值不产生测量。byte 7 已在插拔充电线时证实恒定，充电保持未知。电压换算含义来自上游变化曲线观察，作为估算显示，不声称库仑计精度。

每目标共用 7 秒截止时间、最多 32 帧；延迟样本使用实际读取时间，空读不续期。休眠可连续四分钟静音，故不把超时等同关机，样本独立在 120 秒过期。接收器拔出由枚举判定，恢复收到新帧后更新。不同固件、真实零电量、同型号多实例及官方软件共存仍需真机验证。

## AKG N9 Hybrid

固定 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723`：[AKG 源码](https://github.com/evren-oct/vigil-watt/tree/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Akg)、[硬件记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/devices/akg.md)、[完整 50% 样本及测试](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/src/tests/VigilWatt.Core.Tests/AkgN9Tests.cs)。原作者 endermiteDOTcn / PR44，MIT 许可见 THIRD_PARTY_NOTICES.md。未运行上游程序或硬件脚本。

62 字节固定请求 `06 07 80 05 5A 03 00 D6 0C 00`，后补零。控制输入为 `07 <有效长度0–59> 80 <片段> <旧尾部>`；RACE 消息 `05 <5B/5C/5D> <长度LE16> <命令LE16> <参数>`，总长 6–1024。成功 ACK 为 `05 5B 03 00 D6 0C 00`；结果 `05 5D 05 00 D6 0C 00 00 <百分比>`。要求当前阶段 ACK、状态0、角色0，不把 ACK 或尾部缓存读成电量。

共享控制读取实现支持复用句柄；AKG 整次空闲检测、写入、响应读取共用自身句柄/事件，避免每 50ms 重开。普通 `query_report` 包装层仍保持原有生命周期。分片和 ACK 状态每次查询重置，重复快照只比较有效字节；没有事务号，连续相同分片和迟到结果存在固有限制。公开样本证明特定硬件的行为，不将 GET_INPUT_REPORT 普遍当消息队列。

## 雷神 ML903 NearLink Se

固定来源 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723`：[实现与标签解析](https://github.com/evren-oct/vigil-watt/tree/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Thunderobot)、[硬件记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/devices/ak870-ml903.md)。MIT 归属和许可保存在 THIRD_PARTY_NOTICES.md 并通过 `licenses` 内嵌到程序。

只用 64 字节中断 OUT `A0 02`。已公开输入前缀 `A1 02 04 01 11 02 11 E3 01 9B 36 02 13 31 32 33 34 35 36 02 25 23 02 26 01`，余下零填充：`0204/0211/0213/0225/0226` 长度分别 2/4/6/1/1；`0225` 为百分比，`0226` 不是充电。按字段边界解析，拒绝未知或重复标签、非零尾部及越界，不强制样本的排列顺序。标签内出现相同字节不作为新标签。

上游插件声明 `FFA0:2`、64/64、输出 `A0`，实帧证明输入 `A1`；`MI01/Col04` 与 Feature0 只见合成测试，不将其作为硬件必需条件。加入独立运行时报告证明；尚无本项目真机验证。正常回复无事务号，有限排空和本进程锁不能保证与其他驱动并发时回复归属。测试分别标明公开帧和合成边界。

## CHERRY PIXIU75

固定核对 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723` 的 [CHERRY 实测记录](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/docs/hardware/brands/cherry.md)、[CherryCodec.cs](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Cherry/CherryCodec.cs) 与 [CherryPlugin.cs](https://github.com/evren-oct/vigil-watt/blob/c97adc389b60b0e945a62cddacbaaace39887723/src/VigilWatt.Core/Plugins/Cherry/CherryPlugin.cs)。仓库为 MIT；本次只使用协议事实、公开报文前缀和本项目既有传输，不引入上游实现。

两种 PID 的实测接口都是 `MI_01/Col04 / FF1C:0092`，编号 Report `04`、输入输出各 64 字节。固定令牌 0、块号 `1A`、长度 4、偏移 0；上游确认 WriteFile 中断 OUT 能回包，控制端点 SetOutputReport 无回包。信息块 `1D` 在会话建立后可能长期回零，不作为新增查询或识别条件。

公开前缀 `04 00 00 1A 04 00 00 00 30 00 00 00` 对应 48% 放电，后续 `31/32/33` 配合充电值 1 对应 49/50/51%。测试将前缀重建为 64 字节，尾部零填充和 100% 用例不是原始捕获。相较上游宽松解析，本实现要求完整头部、精确长度和百分比范围；充电高位未验证，保留未知。0 与请求回显有歧义，本轮不发布，不把它解释成离线；超时也不作为断连证据。原 MX Pro 的 `20` 令牌、长度 6 和解析路径保持不变，两种回复不可互用。

## HyperX Cloud II Wireless

核对 [HeadsetControl `32facec14215d3fb6c4b1db0d3b28e0b8d20aa5a`](https://github.com/Sapd/HeadsetControl/blob/32facec14215d3fb6c4b1db0d3b28e0b8d20aa5a/lib/devices/hyperx_cloud_2_wireless.hpp) 的两个 HP PID、接口 0、`FF90:0303`、Report `06`、52 字节请求及 20 字节回复、命令和字段。该实现的缓冲区长度不是本项目的 Windows 描述符抓取，因此运行时另用逐报告描述符证明精确容量；不兼容的集合不发送查询。

[HyperHeadset `3652a5de38c100842e4b2f882af0e04897804638`](https://github.com/LennardKittner/HyperHeadset/blob/3652a5de38c100842e4b2f882af0e04897804638/src/devices/cloud_ii_wireless_dts.rs) 交叉核对 `02/03` 查询，并明确记录 `01` 连接查询会重置静音等状态，故排除该命令。充电 0/1/2 的含义来自同提交的 [devices/mod.rs](https://github.com/LennardKittner/HyperHeadset/blob/3652a5de38c100842e4b2f882af0e04897804638/src/devices/mod.rs)。未知值不推断为未充电，百分比回复不成功时独立的明确充电状态仍可显示。

所查源码分别为 GPL-3.0 与 MIT。本次复用本项目 HID 传输、状态和诊断层，独立编码协议事实，没有引入上游实现。合成测试不证明 Windows 接收器实机兼容；零百分比的非占位电压门禁是本项目对零填充歧义的保护，不是已验证的离线编码。旧 Alpha 2 路径保留，后端显示名扩展但持久键仍使用 `HyperX Alpha 2`。

## Corsair Bragi 接收器暂缓接入

复核 OpenMouse `5a4f6433da3d70865d008874c7055016b336c197` 的 [IRONCLAW 抓取说明](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/captures/corsair-ironclaw-rgb-wireless/README.md) 与 [SCIMITAR 抓取说明](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/captures/corsair-scimitar-rgb-elite/README.md)。`1B1C:1BDC` 的 Windows 抓取只有运行 iCUE 时的 DPI 写入及确认，没有 GET 回复、HID 报告描述符或硬件模式电量证据；`1B66` 接收器没有该来源的流量记录。不能将型号目录当成已验证的电量通道。

有完整描述符的 `1BE3` 是有线 SCIMITAR：`FF42:0001`、无编号 128 字节协议报告，对应 Windows 容量 129 字节；电量属性返回不支持。其 input-only 通知集合也不是命令通道，均不能外推到无线接收器。

[ckb-next `e22eaf2fba7f93764b4c6d4413bb83eeca75d10a`](https://github.com/ckb-next/ckb-next/blob/e22eaf2fba7f93764b4c6d4413bb83eeca75d10a/src/daemon/device_bragi.c) 的接收器初始化主动切换软件模式，不能证明保持设备模式时也可读取电量。Bragi GET 回复不回显属性或事务号，进程内串行仍不能完全区分其他程序对同槽位的 GET 回复。本轮不增加后端或探测命令；后续需无线接收器描述符及不切模式的电量、充电、离线、槽位和多接收器实帧。所查上游分别为 AGPL-3.0 与 GPL-2.0，未引入其代码。

## 雷蛇接收器电量接口

核对 OpenRazer `ea55c1b5bda0b4112fcef91dd38395675151af33` 的 [razermouse_driver.h](https://github.com/openrazer/openrazer/blob/ea55c1b5bda0b4112fcef91dd38395675151af33/driver/razermouse_driver.h)、[razermouse_driver.c](https://github.com/openrazer/openrazer/blob/ea55c1b5bda0b4112fcef91dd38395675151af33/driver/razermouse_driver.c) 与 [razerchromacommon.c](https://github.com/openrazer/openrazer/blob/ea55c1b5bda0b4112fcef91dd38395675151af33/driver/razerchromacommon.c)。所查文件为 GPL-2.0-or-later；本次仅使用型号、命令和字段事实，复用项目现有 Feature 传输及解析器，未引入第三方实现。

四款接收器 `00B4/00B9/00C5/00CD` 的电量事务为 `1F`，命令类 `07`、电量命令 `80`，数据长度 2；百分比由第二数据字节按 0～255 换算。`00C5/00CD` 另查命令 `84`，充电值仅接受 0/1；`00B4/00B9` 不发送充电查询，不把未查询状态伪装为未充电。

型号协议事实与接口模式分开保存：仅 `Feature=0` 的 `MI_00` 鼠标集合进入新增的纯电量路径。已有 `Feature=91` 通用路径保持原有读取能力；型号表不授予额外写入权限。测试覆盖命令、CRC、事务、回显、忙状态、拒绝、短帧、同型号双接收器及旧路径兼容。本项目没有这些接收器的 Windows 实帧，不能由 Linux 上游支持推定本项目实机验证完成。

## Arctis 7+ 系列

固定核对 [HeadsetControl `bd56cf875b736ff91f69ebf17b519a79d0270564`](https://github.com/Sapd/HeadsetControl/blob/bd56cf875b736ff91f69ebf17b519a79d0270564/lib/devices/steelseries_arctis_7_plus.hpp) 的型号、接口 3、`FFC0:0001` 与五档电量，以及同提交的 `protocols/steelseries_protocol.hpp` 的 `00 B0` 查询。所查源码为 GPL-3.0，新增路径复用本项目既有 B0 通信和状态模型，未移植上游实现。

[Linux-Arctis-Manager `684f07c96bd93416d4f1dc22c67b089f2a08d2a0`](https://github.com/elegos/Linux-Arctis-Manager/blob/684f07c96bd93416d4f1dc22c67b089f2a08d2a0/src/linux_arctis_manager/devices/arctis_7_plus.yaml)（GPL-3.0）交叉核对去除 Windows ID 槽后的字段：byte 1 为 1 时离线，0/2/3 在线；byte 2 为电量档 0～4；byte 3 的 0/1 为未充电/充电。未知状态拒绝，未知充电值保留为空，不能套用 Nova 7 的充电字节语义。查询回显与全部零值歧义保持未知。

[OpenChatMix `2413361931f2a4e28389b2a818af646df30c5487`](https://github.com/Swiftlyx/OpenChatMix/blob/2413361931f2a4e28389b2a818af646df30c5487/src/chatmix_device.cpp)（MIT）提供 Windows 零 Report ID 槽及输入/输出长度分别处理的交叉依据。当前新增路径要求实际描述符证明唯一 `00/65`，不把其他长度猜成兼容；本项目没有三款接收器的实测描述符或回包。

## 迈从接收器增量

核对 MyFinder `a763adb5a0c44076f9331ab8fad63a899b5326c7` 的 [mchose.py](https://github.com/wushuai0802/myfinder-device-battery/blob/a763adb5a0c44076f9331ab8fad63a899b5326c7/engine/providers/mchose.py) 与 [test_mchose_v3.py](https://github.com/wushuai0802/myfinder-device-battery/blob/a763adb5a0c44076f9331ab8fad63a899b5326c7/engine/tests/test_mchose_v3.py)。所查仓库为 AGPL-3.0；本次使用协议和型号事实，复用本项目已有实现，没有引入上游代码或运行时依赖。

`3837:101D / MCHOSE V7 Pro` 使用 `FF01:0001`，报告总长 64 字节且包含 ID `4D`。状态请求为 `4D 01 01 00 00 09 00 00 08` 加零填充；回复核对命令、声明长度及可选 XOR，byte 20 为百分比，byte 19 为充电值。V7 的 0 是不发布的占位状态；忙回复 `[FF]` 与合法零电量分开处理，不沿用到已有 A7 的零电量规则。V7 无本项目实帧，适配依据为上游实现和合成测试。

`3837:1014` 的 `MCHOSE A7 V3 Ultra+` 有上游 2026-11-22 Windows 描述符变体成功记录；公开 `ULTRA_42` 前缀对应 42%、未充电，测试中的尾部零填充是重建内容。新增例外只扩展这一完整产品名和 `MI_02`，不把同 PID 的其他设备当成同型号。未知 V7 物理归属时拒绝读取；扫描失败只跳过新增 V7，保留原 A7 候选。

## ROG OMNI 配对表

`0B05:1ACE` 接收器 FF02 控制接口使用 Report 1 查询 `A0 00`。本项目 2026-11-25 实机核验的 64 字节回复前缀为 `01 A0 00 01 00 65 1B 03 05`，其余为零。以 Report ID 为字节 0，字节 3/4 是小端条目数，条目从字节 5 开始，每条 4 字节：PID（LE16）、Report ID、通知 ID。本例对应 PID `1B65`、Report 3、通知 ID 5。

解析必须验证报头和全部声明条目的长度，只有唯一匹配目标且 Report ID 非零时才建立对应链路。回归使用该实帧，并覆盖双接收器隔离、截断及歧义拒绝。这份回复只证明已测设备的格式，不代表同一接收器上任意多外设均已完成实机验收。

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
- 雷柏持续回报使用独立的两秒界面发布周期，不受默认六十秒主动查询间隔限制；本项目实机验证限于 VT1 的无线电量与当前 DPI 读取。
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

回归覆盖 Logitech 回报率、DPI 草稿与并发保护、CompX 回显、Razer 设置与读回、page-command 完整配置和耳机电量。这些鼠标与耳机能力仍待本项目实机验证，不能将模拟测试等同于硬件兼容。

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

## 接入与验证边界

电量与 DPI 独立验收；读取电量成功不授予设置写入权限。型号识别须同时核对描述符、运行时身份、传输、传感器及协议布局，没有证据的组合保持未知，不靠扩大 VID/PID 范围试写设置。键盘不能沿用鼠标设置命令。

设备真实档位与软件预设分别保存。只有已确认数值写入能力的设备开放预设切换，按实际范围和步进校验，写后读回确认；身份暂缺、配置变化或离线时不沿用写入授权。CompX 预设绑定点击时的激活档，写前后复核档位及 X/Y；交互与设置的逐型号实机验收仍待完成。

雷柏 nRF54L 官方驱动按配置地址区分最多 7 档，与部分参考实现的固定地址、6 档不同；按实际配置读取完整 X/Y 表，不套用旧 VT9Pro 协议。实机切档、休眠与官方驱动共存仍待验收。

## CompX 只读电量型号

复用 `3554` 的 Report `08` / 17 字节传输，只匹配 `FF02:0002`。新增型号不先发送 `0x10` 身份命令，也不读取配置或 DPI；每轮仅发送固定 `08 04` 加零参数和补偿校验。回复要求 Report `08`、命令 `04`、状态 `0`、声明长度、byte 6 电量 `0～100` 及全帧累加和为 `0x55`。byte 7 不用于这些新增型号的充电推断，保持未知；状态长度允许时 byte 8～9 解码为电压。

`3554:F503` 使用公开完整 95% 回包，byte 8～9 为 `0x10 05`；`F506` 使用 VGN HUB 对 Report `08` / 命令 `04` 的静态映射；`FB26` 使用 DELUX GH8K 的 90%、85% 回包；`FB3E` 使用 VGN 共享接收器的 40%、90%、85% 回包。`FB3E` 只显示中性名称，不从接收器编号推断 Y2 Pro、F2 Pro Max 或其他零售型号。

这些路径只开放电量，不识别 CID / MID、不读取配置且不开放 DPI。描述符不是 17 字节输入输出或集合不是 `FF02:0002` 时不查询；深睡 `08 03` 事件、静默、请求回显、错误校验和及越界电量均不生成新样本。测试使用公开帧和按字段构造的异常帧，本项目尚无对应硬件，冷启动、充满、休眠唤醒及 VGN HUB 共存仍待实测。

## 前行者 X23 Pro 被动状态

只匹配 `04F3:026F`、产品名 `X23 Pro Receiver`、`MI_01 / 000A:0000` 的 `Col04`。运行时必须解析到同一 USB 父设备，且描述符只声明一个 32 字节 Input 报告；报告 ID 仅接受公开路径验证过的 `0` 或 `4`。相同身份出现多个候选、父设备不唯一或描述符无法证明时不打开输入流。

每轮只打开只读输入句柄，最多等待 1.2 秒接收原生状态推送，不写 Output、不 GET/SET Feature、不执行配对或初始化。Windows ReadFile 缓冲保留 Report-ID 槽，因此编号与无编号报告的百分比都在 byte 4；全零帧、错误 ID、短帧、越界值和静默都不生成样本。充电未知。上游 Windows 实现约每 5 秒收到一帧并已与官方软件读数对照；本项目测试使用合成帧，实际节奏、冷启动、深睡、唤醒及厂商软件共存仍待实机验收。

## 黑爵 AK980 PRO 设备信息

有线命令集合固定为 `0C45:8039 / MI_02 / FF68:0061`，2.4G 为 `0C45:FEFE / MI_03 / FF60:0061`；产品全名和输入输出 65 字节都由运行时描述符再次确认。2.4G 的 `MI_04` 与有线的 `FF67:0061` Feature 大包通道不发送命令。

发送 65 字节 `Report 0`：`00 | AA 10 30 addrLo addrHi 00 01 | data`。`GET_DEVICE_INFO` 数据长 48；2.4G 可以只返回 32 字节，因此只依赖前 19 字节。接收输入槽经统一无编号报告处理后，帧必须以 `55 10` 开头、声明长度 19～48、保留完整声明数据，并回显键盘 VID/PID `0C45:8039`。payload byte 16 工作模式有线为 0、2.4G 为 2，byte 17 电量接受 0～100，byte 18 当前只有 0/2 两个样本，不映射为充电状态。

每轮只发送一次信息查询，命令通道按需打开并在本轮关闭，不保持常开。上游维护者已在两种连接下实测 100%，正常往返约 14～17 ms；测试仍以公开帧和异常合成帧为主，本项目没有对应硬件，0%、休眠、蓝牙档、多键盘与网页驱动共存仍待验收。

## Veekos Shine 60 状态页

仅匹配 `3151:5026`、产品名 `Veekos 2.4G Wireless Keyboard`、`MI_02 / FFFF:0002`。集合必须没有 Input/Output，只有 65 字节 Feature；描述符需证明唯一 Feature ID。公开固件优先使用 `0x83`，另一种无编号变体使用 `0x00`，同时出现多个报告 ID 时不读取。

只调用 GetFeature，不发送 Feature Set 或 Output。回复长度必须为 65，首字节回显请求 ID，byte 1 为 0，byte 2 为百分比，byte 3 固定为 100。公开用户记录包含 61% 和 48%，其中后一条前缀为 `83 00 30 64`；当前实现接受 0～100，拒绝 101 以上、错误回显、错误比例、短帧及静默。接受范围不代表真实 0% 语义已通过硬件验证。充电未知。实际 0%、冷启动、休眠唤醒、频率和多接收器仍待本项目实测。

## Logitech G PRO X 2 LIGHTSPEED 耳机

接收器 `046D:0AF7` 的 Usage Page `FFA0` 集合在 Report `0x51` 上交换 64 字节帧。请求固定为 `51 08 00 03 1A 00 03 00 04 0A` 加 54 个零；描述符必须证明输入输出只有 Report `0x51` 且各 64 字节。不发送 HID++、LED、音频端点或固件命令。

回复类型 byte 1 为 `0x0B`、tag byte 8 为 `0x04` 时，byte 10 为 `0～100` 电量，byte 12 等于 `0x02` 表示充电。类型 `0x05` 且 byte 6 为 0 表示耳机关闭，应清除旧电量；ACK 和 byte 6 非零的开机事件只是噪声。轮询周期为 120 秒，避免高频唤醒无线接收器。开源 Linux 驱动与独立 probe 均在目标 PID 上取得实机读数；测试使用按字段构造的完整帧和异常帧，本项目尚无硬件，冷启动、充电切换、休眠及 G HUB 共存仍待实测。

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

XInput 不提供型号或持久物理 ID，不使用 VID/PID 猜测槽位，也不推断虚拟控制器对应哪个 HID 设备。偏好按系统槽位保存，快速替换和跨协议重复项的边界已写入兼容性说明。本项目尚无对应手柄电量实测。

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

本项目尚无 Cherry 实机验收；协议测试不证明低电量、停止充电、长休眠和官方驱动共存行为。

## 1.4.7：洛斐 HYZEN

2026-11-21 直接核对 [Lofree HUB](https://hyzen.lofree.tech/) 当日 `assets/index-D0x9DTYM.js`，SHA256 `078b76071f7f4cdf7851611c3a2cd5fba90b6da18502b3a844845404a059dafa`。官方脚本没有发现开放源码许可，仅在本地分析协议事实，不分发脚本、资源或混淆代码。

官方列表含 `388D:0024` 有线、`0025` 2.4G、`0029` 单模；控制集合 `FF1C:0092`。电量调用向 `Dx` 传入 `AA` 或 `1A`、空数据和偏移 0，使用 Report `04`；无线载荷 31 字节，Windows 加报告 ID 为 32 字节。START / END 的载荷偏移 2 为 1/2；数据报文偏移 2 为命令，4/5 为地址，数据从偏移 7 开始。与 MyFinder `73d99e2` 的 `lofree.py` 交叉核对。MyFinder 测试注释的 issue #82 在本次查询时对应另一设备，未将该议题作为洛斐硬件证据。

实现只允许在线 / 电量命令，逐阶段限时、校验非空数据长度及零偏移，失败后尝试 END，并以结束确认作为发布条件。前后复核在线；充电保持未知，不使用官方 UI 对有线设备初始化的 100 作为实测电量。有线存在时抑制查询的规则暂按型号执行，配对身份尚未确认，可能影响另一把同型号键盘；详见兼容性边界。

本项目尚无洛斐硬件验收，Windows 集合、数据长度与休眠恢复仍待验证。

## 1.4.7：VGN / VXE 320F:5088 键盘

核对先前保存的 [VGN HUB](https://drive.vgnlab.com.cn/) `assets/index-BfN6tzNB.js`，SHA256 `2dc306331d30f874eec8a006f975ef5cac3ed2f18d3bef58e5dd089dc8d5cd5f`；设备目录与 `weisheng` 驱动使用 Report `04`，电量载荷 `00 00 1A 06 00 00 00`，回调在载荷字节 2 为 `1A` 时读取字节 7。仅研究协议事实，未复制或分发厂商代码。MyFinder `73d99e2` 的 `vgn_keyboard.py`、协议记录及公开回复前缀提供独立交叉依据。

实现严格匹配 `320F:5088 / FF1C:0092`、64 字节双向 Report `04` 和唯一管理集合，只读电量；不复用 Cherry 的 `04 20` 前缀。约 1.03 秒的公开接收器延迟使旧短超时不适用，新适配器采用 2.5 秒窗口并与 V98 Pro V3 分开调度。`FF` 休眠回报、查询回显和错误帧不刷新采样时间，120 秒后隐藏旧值，十秒以内的后台等待结束后再次查询。

共用编号和通用产品字符串无法排除 Rainy75，因此保留产品名，不凭 PID 命名 VGN 具体型号。不同接收器保留实例身份；充电、无事务号下的多进程共存、完整休眠周期及所有目录型号仍待实机验收。回归覆盖主界面与托盘共用状态的过期及后台恢复；上游局部记录不等同于本项目实测。

## 1.4.7：VGN / VXE 20 字节键盘接收器

交叉核对固定 VGN HUB 资源 `index-ZngoA_qG.js` 和 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `vgn_beiying.py`、`by_keyboard.py` 及 #190 实机记录。官方目录给五个无线身份统一使用 `FF02:0002`、Report `13`：`258A:024F/027F/027B`、`391D:2A01/2A02`。有线 `FF00:0001 / 09` 和共用 `3554:FA09` 不在本读取器范围；`3554:FA09` 继续由统一 MCHOSE 键盘后端独占。

完整请求为 `13 4A 01`、16 个零和末字节 `5E`；末字节是前 19 字节的累加和。回复必须恰为 20 字节，头为 `13 4A 01 00`，拒绝请求与身份查询回显，并重新计算同一累加校验。full-frame byte 5 为 0～100 的百分比；byte 4 是包元数据，不赋长度含义。只有 `258A:024F` 在 #190 的插拔对照中确认 byte 6 的 `0x10` 为外部供电，其他四个身份仍保持充电未知。没有把 M HUB 的 MCHOSE 高低半字节规则套到 VGN。

描述符必须证明同一接口组唯一的 `FF02:0002` 管理集合、输入输出都只含 20 字节 Report `13`。不使用产品名或型号查询来推断设备；精确 VID/PID 表已给出产品名，但仍不冒充完整硬件验证。单次查询窗口 2.5 秒，失败不刷新旧样本；120 秒后隐藏，十秒后台发布间隔与既有托盘状态一致。

新增五个身份仍待对应无线接收器实机验收；现有有线 `258A:024E` V98 Pro V3 通过原 Feature 路径读取，本项目实机读到 100% 与充电状态，不用于证明上述无线路径。描述符、0%、充电变化、休眠恢复及与 VGN HUB 并发读取需要后续硬件记录。

## 1.4.7：前行者 S9（M156）

依据 HaloBattery PR #352 在 MIT 许可下公开的报文事实和 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `eweadn.py`、测试，只独立实现 `089D:062F / FF01:0010` 的 M156 电量读取。仅接受 33 字节输出和 16～34 字节 Windows 输入容量；描述符必须证明双向都只有一个无编号 Report 0。有线 `088D:062E`、`FF00:0000`、M302 和其他产品名均被排除，也不使用被动 `D1` 通知。

固件输出请求为 `00 30` 加 31 个零：首字节是 Windows Report ID 槽，后续 32 字节载荷的 byte 0 为 `30`，其余为零；载荷校验范围 byte 5～31 因而得到 0。输入使用实际描述符的完整容量接收，再移除零报告编号，不固定假设 33 字节。

回复必须为 15～33 字节，以 `30 00` 开头，声明长度足以覆盖 byte 4～14 且不超过实际帧；byte 4～7 必须为 ASCII `M156`，byte 13 为 0～100 电量，byte 14 必须为观测到的在线值 `01`。没有已确认充电字段，充电保持未知；`30 02`、其他状态、错误身份、截断或请求回显都不会变成 0%。一次发送在 1 秒窗口内可以忽略无关输入并等待第一条有效 M156，不发送额外在线或配置命令。

回归覆盖 15～33 字节输入、全部百分比、严格身份和错误帧。没有本项目 S9 实机，实际 Windows caps、70% / 69% 等跨档比较、休眠、拔插、冷启动和官方驱动共存仍待验收。

## 1.4.7：前行者 S9 PRO 8K

依据固定 EWEADN HUB `vendor-bytech` 与 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `eweadn_s9pro.py` 及 #315 用户实机记录，独立实现 `372E:1026` 的 basics 电量读取。目录同一 VID/PID 还包含其他 S7/S9 型号，因此名称只是该接收器协议族的当前展示名，不回传型号查询。

官方目录把控制集合分到 `FF00:0001` 与 `FF06:0001`。实现要求同一接口组中只有一个集合能由运行时描述符证明 Feature Report `03` 为 64 字节；HidP 的集合最大长度单独不算报告 ID 证据，其他 ID 可拥有更大报告。已证明的 Feature 报告图允许 `03:64` 与 `09:128` 共存，但只传输 `03`。零个或多个已验证集合都 fail closed，不优先 FF00，也不在一次失败后尝试 FF06。

完整 Feature 请求为 `03 20 50 00 00 4F 81` 加 57 个零。校验 `20` 是零初始化 63 字节载荷的累加和，不含 Report ID。官方控制器先 SET_FEATURE，等待 100 ms，再 GET_FEATURE `03`，未建立状态变更握手前置条件。回复有两种已记录形状：

- 16 字节用户实机回复：`03 50 00 0B 00 01 <battery> ...`，要求存在字节 `01`，电量只接受 1～100。
- 64 字节描述符容量回复：要求 ID `03`、ACK `00`、封装索引 `00`、声明长度 7～58；电量接受 0～100。全零缓冲、请求回显、截断或超长、错误 ID/ACK/索引和越界值都拒绝。

两路均把数据字节低位七位作为百分比、最高位作为充电；不猜测回复校验和，也不把 TYPE/OFFSET 固定成请求值。单次查询只允许一帧请求和一次读取，不重试，不发送 UUID、在线、重置、DPI、回报率、宏、显示、配对或固件命令。

#315 记录了 Windows x64 下 16 字节回复、23%～98% 稳定读数和充电变化；本项目没有独立硬件。相同编号的其他型号、真实 64 字节回复、休眠缓存、唤醒、冷启动和官方驱动共存仍未验证。

## 1.4.7：飞智 APEX5

依据 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `flydigi.py`、三份批次协议资料与公开许可证边界，独立实现 `37D7:2501 / FFA0:0001` 的 GET_INFO 电量。描述符必须只含 32 字节 Input Report `04` 和 32 字节 Output Report `03`，同接口组只允许一个候选。

唯一请求为 `03 5A A5 01 02 03` 加 26 个零。回复必须为 32 字节并匹配 `04 5A A5 01 01 00 80`、byte 7 为模式 `01` 或 `02`。byte 12 的低四位是 `0..5` 五档电量，映射为 0/20/40/60/80/100%，界面统一显示 `≈` 估值；高四位只接受 0 或 1。高四位为 1 时是厂商充电哨兵，不解释成第六档或非法百分比，只报告充电中且电量未知。其他档位、充电枚举、请求回显和错误帧都拒绝。

回复校验和尚未确认，不将请求校验规则套到回复；只发送一次 GET_INFO，不读取 IMU，不发送震动、扳机、灯光、模式或配置报文。本项目没有 APEX5 实机；真实 caps、休眠、充电切换、0% / 100% 和桌面软件共存仍待验收。

## 1.4.7：盖世小鸡 Nova / Cyclone 2

协议事实根据 Wireless-Device-Battery-Monitor 的 MIT 驱动资料和 GameSir 客户端公开描述独立实现，不复制驱动代码。Nova 系列只匹配 `3537:1098/100F/1040` 与 `FF7A` 集合；描述符必须证明输入输出都只有一个无编号 Report 0，输入容量至少 19、输出至少 3。请求为 Windows Report ID 槽 `00` 加 `01 01`。回复可能先出现 `F4` 状态帧，程序在 800 ms 内等待首字节 `01` 的帧，并读取 byte 18：0～100 为精确电量，`FF` 为充电哨兵；其余值拒绝。

Cyclone 2 只匹配 `3537:100B / FFF0:0040`、64 字节输入输出和输出 Report `0F`、输入 Report `12`。固定请求为 `0F 03`，只让手柄开始发送状态；回复必须为 Report `12`，byte 35 只接受 0/1，byte 36 为 0～100 电量。其他 Report、输入长度、充电枚举和电量拒绝。

两条通道都没有设置写入入口，不发送震动、扳机、灯光、按键、模式或配置命令。回归覆盖精确身份、描述符、请求、0% / 100% 和错误帧。本项目没有对应实机；真实 caps、休眠、有线 / 2.4G 切换、多手柄实例与官方软件共存仍待验收。

## 1.4.7：ATK N9 V2 ULTRA / 迈从 V9 耳机

FF90 耳机后端以精确 `FF90:0001`、输入输出 Report `55` 和描述符容量控制三条身份。新增 `373B:12D6` ATK N9 V2 ULTRA，允许输入总长 22～64 字节；`373B:1262` N9 V2 Ultimate 使用同一协议。请求固定为 `55 65 01` 加 61 个零。ATK 回复状态 `02/03/04` 分别为放电、充电、充满，均要求 byte 2 为 0～100；状态 4 保留原始百分比。

`3837:600A` MCHOSE V9 Turbo+ 继续要求 64/64 字节。状态 `02` 读取电量、`00/03` 表示外部供电但百分比未知、`1A` 表示明确离线；其他状态拒绝。外部供电保留充电标记但不用 100% 或上一次电量替代；离线和静默分开处理。`3837:6008` 灯光接口不探测。

`291D:385D` V9 PRO 使用同一 FF90 电量协议，但增加固件门禁：同一物理接收器必须存在唯一的 `FF22:0001`，且描述符证明 Feature `AA/64`。只读请求 `AA 01 01` 加 61 个零后等待 300 ms，再读取 Feature `AA`；回复按 byte 2～5 的十进制字符串拼接得到版本。确认低于 `0008` 时阻止电量查询；版本未知、查询失败或描述符不唯一时按既有方式保守回退，不重复重试固件查询。

回归覆盖 ATK 22～64 字节输入、状态 2/3/4、迈从状态 0/2/3/1A、请求回显、固件十进制版本和描述符。没有上述耳机实机；真实 caps、充电 / 满电 / 拔线、休眠、固件版本和厂商软件共存仍待验收。

## 1.4.7：迈从 QHW 与 BY 键盘

2026-11-21 直接核对 [M HUB](https://www.mchose.com.cn/) 当日入口，应用 `assets/main-DIt5vAcV.js`（SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`），以及官方 manifest 指向的 `cizhou/CZ_SHARED_DATA/main.002bbdb0f17b5f7135ee.js`（SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3`）。这些厂商文件未发现开放源码许可，只在本地静态分析协议事实，不运行整包、不分发实现或资源。与 MyFinder `73d99e2` 的 QHW、BY 协议记录交叉核对。

QHW 目录严格对应 G98 V3 无线 `3837:1015 / MCHOSE G98 V3 2.4G / FF80:0001`。`getBattery` 调用 `_simpleGet(2308, [])`，Report `4D`、小端命令 `0904`，载荷 `01 03 00 04 09 00 ss cc`；序号低八位递增，输出校验为去掉 version 后各字节 XOR，即 `0E XOR ss`。回复匹配 version、flags、命令、bizCode、sequence，并要求 length=2。数据两字节为充电枚举与电量；枚举 2 表示充满而非正在充电。官方只提取回复 checkCode，没有确认其算法；本项目不猜测回复 XOR。

Windows 描述符分别证明输入输出 Report `4D` 及容量，最小 11 / 9 字节，写入按实际输出长度补零，回复必须符合实际输入长度。官方 `NORMAL_MAX_PACKET_LENGTH=31` 是代码上限，不能冒充已测得的 HID 报告长度。只发送电量查询，setup 中的本地锁、监听和其他设置流程不移植；冷启动与睡眠缓存的硬件行为待验收。

BY 的 `keyboardPwds` 是六字节型号表，不是登录凭据。`getPassWord` 发送 Report `13` / `05`，单包回复从完整帧字节 5 开始取六字节；不能移位寻找一个期望型号。官方 `handleCommandResponse` 检查总包数和序号，丢弃字节 4 及最后一字节；所以 FA09 / 新 `41E4:2001` 分支不强制长度元数据或沿用其他固件的回复校验。保留输入报告 ID、20 字节总长、命令和单包头检查，精确请求回显不能提供型号或电量。Z75S 与独立 VGN V98 Pro V3 的既有校验仍保留。

已识别型号还可接受查询窗口中的 `0A` / type `05` 电量通知；按照 M HUB 的 `batteryParser(false)` 跳过通知字节 2～4，从 6 / 7 读百分比与标志。只在高低半字节均为 0/1 时解释充电；高半字节为 1 且百分比低于 100 才显示充电，满电标志不改写百分比。通用 FA09 没有已确认型号时，只取 `4A` 的有效百分比，充电未知；其他已命名品牌不额外发送迈从身份查询。BY 无线 0% 与休眠占位难以区分，不刷新原读数。

沿用 FA09 已有 SET_REPORT 传输，Z75S / `41E4:2001` 使用中断 OUT，不自动切换传输重试。`41E4:2001` 暂限官方精确通用产品名、`FF04:0002` 和已确认型号码；没有实现跨 USB 接口的 FF02 兄弟配对。全部入口增加双向 Report `13` 描述符验证和同接口组唯一性检查，Feature-only 的 AULA/FA09 路径不发送命令。

回归覆盖 G98 V3 序号回绕、错误回复、独立容量，BY 未知型号、主动通知、休眠占位及后台恢复。MyFinder #476 的完整两帧保持原样：身份无法匹配 G75 Pro，电量可解析为中性 100%、充电未知；#312 仅使用已发布前缀，尾部为测试填充。本项目尚无这些键盘的硬件验收。

## 1.4.7：迈从 G87 V2 GLW 键盘

根据固定的 M HUB / CZ SDK 静态资料和 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `mchose_glw_keyboard.py`、`mchose_keyboard_hid.py` 交叉实现。`3837:3033` 与多款键盘共用，入口要求精确产品名 `MCHOSE G87 V2 2.4G`、集合 `0001:0000`、65 字节输入输出。描述符必须证明输入报告 ID 唯一（可为 0），输出报告 ID 唯一且为 0；同接口组有多个符合项时拒绝。

Windows `HIDP` 的输入容量包含 Report ID 槽，`ReadFile` 也必须使用完整容量。对于已证明的无编号输入，接收后移除零 ID，再解析 64 字节载荷。仅在指定集合上使用中断 OUT，命令只允许 64 字节载荷加一个前导 Report ID：

```text
55 04 00 SS 38 00 00 00 ...    // 读取 56 字节基础信息
55 05 00 SS 38 OO OO 00 ...    // 读取当前配置前 56 字节
55 05 00 SS 08 OO OO 00 ...    // 读取当前配置尾 8 字节
```

`SS` 是载荷 byte 4～63 的累加和，`OO OO` 是当前配置块偏移的小端值。基础数据 byte 0 为活动索引，byte 2～5 若是 `0..3` 的完整排列则映射为真实槽位，否则沿用旧式直接索引。两条 `05` 回复必须分别声明 56 和 8 字节并回显相同偏移，合并成 64 字节配置后读取 byte 32：低七位为 0～100 电量，高位为充电。

回复只需 `AA` 标记、命令、长度和偏移匹配；回复校验与其他头字段仍未确认，因此不强制或猜测。不发送基础修复、状态切换、回报率、DPI、灯光、宏、配对或固件命令。回归覆盖零 / 非零输入 ID、请求校验和、三种只读交换、槽位排列及异常帧。没有 G87 V2 实机，真实 caps、休眠、冷启动、配置切换和官方驱动共存仍待验收。

## 1.4.7：漫步者花再 Halo Space

依据 FerrisPulse `335fb15` 的 [公开协议记录](https://github.com/Ferris-echo/FerrisPulse/blob/335fb15ba98be94a85e13061dc0812a04b3aadf6/docs/EDIFIER_HALO_SPACE_PROTOCOL.md)、设备辅助类和报文测试，独立实现电量 `D0`。公开记录明确 `2D99:E049 / FF14:0001`、输入输出 64 字节，输入前缀 `2F BB EC`，输出 `2E AA EC`；`FF13` 为另一个集合，不因输出长度相近而探测。上游的接口回退和多种松散帧头兼容不沿用。

固定查询 `2E AA EC D0 00 00 66` 加零填充；首字节已是 Windows 输出报告 ID，不能再前置 `00`。回复大端载荷长度必须为 1，线上字节 1～6 的和与字节 7 比较，随后将字节 6 XOR `A5` 得到百分比。公开前缀 `2F BB EC D0 00 01 86 FE` 得到 35%。仅有效载荷参与解析，历史缓冲尾部不会影响电量或产生虚假充电状态。

描述符额外要求单输入 `2F` / 单输出 `2E`，这是保守接入条件，仍需实际 caps 验收。没有查询 C8 MAC、C9 名称或 C6 固件，也没有尝试上游记录未响应的 F2 状态命令。充电保持未知，不把上游界面的 false 当成已确认放电。跨进程同命令回复归属、接线充电导致的断连和接收器休眠缓存均待实测。

回归检查有效零/满电、错误校验、错误通道、回复长度、尾部残留、不同实例及统一主窗口/托盘状态过期恢复。上游实机声明仅作协议来源，本项目没有该耳机，未宣称硬件验收完成。

## 1.4.7：达尔优 Jm 鼠标

对照 [达尔优官方网页驱动](https://driver.dareu.com/allinone/) 1.3.26 的 `JmHidDevice`、配对目录及各型号 JSON，结合 OpenMouse `5a4f643` 的 [A950 PRO Mg 协议记录](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/dareu-a950-pro-mg.md) 和 Voltide `c1872c4` 的 `DareuA950MouseProvider` 独立实现。官方 `jmdevice.js` SHA256 为 `997fb0cb6331a68e730db1d59499cad11aaf0a976db70383f33aa476c1370427`，`global.js` 为 `cdb0d2db7f36bd53e87f906bc86749e498d45739dc6f815d6c033c441842accf`。厂商资源只作本地静态核对，不随源码或程序分发。

接入限 `260D:1114 / 1117 / 1175`、`FF02:0002`、双向 Report `08`、完整 17 字节报告，以及同 USB 接口组唯一控制集合。1114 / 1175 是共用接收器；不能固定命名成 A950 PRO Mg 或 A950 AIR V2。官方当前 `ParseDevices` 以 FF02:2 的 Report 08 判定 Jm；因此本实现直接验证这个集合，不按 VID/PID 借用另一接收器的 FF05 服务集合，也不猜跨 USB 接口的物理归属。

| 接口 PID | 运行时 CID / MID | 官方型号 | 本次能力 |
| --- | --- | --- | --- |
| 1114 | 10 / 1、10 / 3、10 / 4、10 / 39 | A950 PRO、A950 PRO Mg、A950G（两种目录型号） | 电量；PAW3395 实际档位读取与切换 |
| 1117 | 10 / 3 | A950 PRO Mg 有线 | 同上，但有线通道未实测 |
| 1175 | 10 / 9、4 / 9、10 / 25 | A950 AIR（两种目录型号）、A950 GM | 电量；PAW3395 实际档位读取与切换 |
| 1175 | 10 / 37 | LM180T | 电量与命名；PAW3950 编码未接入，不读写 DPI 配置 |
| 上述接收器 | 其他或查询失败 | 中性名称 | 有效电量；不开放设置写入 |

命令 03 确认在线，01 的完整帧字节 10 / 11 / 12 提供 CID、MID 与接收器类型；请求按官方流程携带四字节随机挑战，但尚无挑战回显的证据，不能将其宣称为事务编号。命令 04 只接受字节 5 为 2 的电量回复，有效百分比在字节 6。该长度条件来自 Voltide 的实际驱动记录，较官方网页解析更严格，1114 其他固件仍需验证。充电 0 / 1 分别表示未充电 / 充电；枚举 2 在官方当前 `energy.html` 与 `energyex.html` 显示充电，而 Voltide 解释为充满，暂保持未知。不显示未核实的电压，不要求尚未证明的回复校验和。

DPI 先读当前四选一配置号，不切换配置；地址 2 / 4 分别为档位数与零基当前档位及各自校验，实际读取 1～5 档，绝不填入目录默认档位。地址 `12 + 4 × index` 的每条记录按实际型号解码，PAW3395 高字节 `00 / 44 / 88` 对应每 256 级的 50 DPI 编码，范围分别限制为 100～26000、100～30000 或 50～26000。保留原始编号、重复值及 X/Y 值，记录和值必须为 `55`。

完整配置反向复读并校验身份和配置号，指纹覆盖身份、类型、当前配置、档位数、当前档及全部记录。用户点击后重新读取并比较指纹，只有一致才发一次命令 07，写地址 4 的两个字节；不改档位数量、DPI 数值表、按键、宏、回报率或其他配置。ACK 丢失不重发，必须再读完整表确认目标档位和其他记录未变才报告成功。没有协议事务 ID 或跨进程原子锁，不能彻底排除官方驱动在最后核对后的并发修改；同型号替换也缺少唯一身份，仍需实测。

回归用例为按公开字段构造的合成报文，覆盖共用接收器、未知传感器、错误长度与地址、休眠恢复、配置中途变化、写入未生效、ACK 丢失及主窗口/托盘命令队列。OpenMouse 的 1114 实机声明只作为协议来源；本项目没有达尔优实机，全部新增入口仍待硬件验收。

## 1.4.7：迈从 Feature 鼠标

从当前 M HUB `main-DIt5vAcV.js`（SHA256 见前文）核对 `mouseDeviceReadParser`、`mouseAllReadParser`、`mouseAllSetParser`、Feature 收发和型号表，并交叉检查 OpenMouse `5a4f643` 的 [MCHOSE 协议记录](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/mchose-protocol.md) 及 L7 Pro+ 公开报文。厂商和上游源码不进入发布包。

接入限制为 `3837:100B/1020/4018/4023/4019/4021/4015` 与 `5253:1020/1011`，`FF01:0001`，Feature 容量 65 字节。描述符必须包含 Feature `11/12`，允许同集合的 `14`，其他 Feature ID 拒绝；`FF0B` 固件集合、MagDock、QHW 键盘、A7 V3 的 `4D`、旧 A5 和蓝牙不探测。同接口组有多个管理集合时拒绝，重复序列号不会合并不同实例。

短查询为 Report 11 加取反后的 03 / 06 及 19 个参数零，剩余位置保留线上零填充；长配置查询为 Report 12 / 67。每次只发送一次查询，在 1.9 秒 / 最多 20 次读取中等待连续两份相同的有效载荷，间隔 90 ms；无关命令、失败和空回显会清除连续样本。Report 11 只解析 7 / 11 字节，接受公开抓包的 21 字节或 Windows 容量 65 字节；尾部不会变成型号、电量或设置。Report 12 必须返回 65 字节，并比较完整 63 字节配置。

06 的载荷 VID/PID 是鼠标身份，不是接收器 PID。官方解析库 `.endianness("little")` 的 bit 字段从低位开始，因此连接模式是 flags 的低三位，连接位是 bit 3；OpenMouse 所查版本的高位解析不沿用。公开 L7 Pro+ 有线帧得到 `3837:4015`、57%、充电、flags 00；无线帧得到同一鼠标、100%、未充电、flags 09。无线前后还用 03 核对接收器 VID/PID、配对和在线状态。充电只解释 0/1，其余未知；无线 0% 按官方占位行为不刷新电量，有线有效 0% 保留。明确离线清除旧数字，超时不延长旧样本寿命，后台最多等待 10 秒再查询，电量寿命 120 秒。

运行时型号目录包含 A7 V2 Pro `4018`、Pro+ `4023`、Ultra `4019`、Ultra+ `4021`、L7 Pro+ `4015`、L7 Pro `00B0` 及 A7 Ultra `1011`。仅目录存在不代表实机已验收；型号未知时仍可保留有效电量，但不读写设置。尤其旧 `5253:1020` A7 Ultra 有 06 一直为空的公开记录，本实现不会凭产品名捏造电量，也不会把该接收器一律当作 A7 Ultra。

配置只读当前三选一板载配置，byte 1 / 2 的低半字节分别为有线 / 无线档位，实际启用数在 byte 16。按启用数显示，保留编号、重复数值和 50 DPI 步进；不把六个已填槽都当成启用档。没有确定的 Y 轴启用字段，因此只显示已核对的 X 值，不冒充 X/Y 相同。A7 V2 Ultra+ / L7 Pro+ 有上游整块配置读写依据，开放切档；其他型号先只读。

设置指纹包含鼠标 VID/PID、固件、连接方式和完整 63 字节配置。点击后重新确认连接、身份和配置两次；只修改当前连接 byte 1 或 2 的低半字节，命令 57 回写原 63 字节，其余 62 字节及该字节回报率位原样保留。等待 400 ms 后完整读回，并确认身份、在线与所有字节；失去发送完成信号不重发、不盲目回滚。这里只切换已有档位，没有开放任意 DPI 数值或自定义预设写入。尚不能消除官方驱动的最后时刻竞争，也不能凭型号区分同型号新旧鼠标。

电量与配置解码使用公开 L7 Pro+ 原始帧，其他场景使用合成报告：旧命令穿插、半写入、尾部变化、未知型号、休眠、离线、完整配置保留、配置竞争、发送超时后成功读回、未生效与错写相邻字段。无线零电量占位不能启用设置。主窗口与托盘使用同一配置令牌和后台队列。本机无迈从鼠标，Windows Feature caps、接收器缓存、完整唤醒周期和切档效果仍待实机验收。

## 1.4.7：黑爵 AJ179 PRO / GearHub

结合 OpenMouse `5a4f643` 的 [AJ179 PRO 协议记录](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/ajazz-aj179-pro.md) 与公开原始回复，直接核对 [GearHub](https://qmk.top/) 当前脚本及其 [v4 驱动](https://qmk.top/v4/) 的型号目录和 Pan1080 控制类。仅静态研究协议事实，厂商代码、资源与研究工具不进入源码或发布包。

| 官方来源 | SHA256 |
| --- | --- |
| `js/index.Ba-4VG4I.js` | `c4fc6fdd77b58974804385bfe659928e0b8fabc7bcce8c94a5432f86b547c1f4` |
| `v4/js/index.58137dcb.js` | `7dfb0851189a6287c1a03b36dc785237adc905b1cf10ac136a751c12e6e2d0b1` |
| `v4/js/f1e16e07.js`，Pan1080 基类 | `43e9a29f48d16ee292aea1f229c0e17b984a039971cde54ad3917de2578e90cc` |
| `v4/js/1167ee1d.js`，AJ179 PRO 子类 | `af7392515ea3384e4db151de2d2b6c4c5165c9edd3ac5cb1a92424c9207a6106` |

官方两个目录均将设备 ID `1851` 映射 AJ179 PRO，8F 回复的载荷字节 1～4 为小端型号 ID。主机 `3151:402D/4026` 仅确定接收器 / USB，不确定品牌。控制集合严格为 `FFFF:0002`、Windows Feature 容量 65，Feature caps 只含 ID 0；公开描述符的报告体为 64 字节，Windows 前置零报告 ID。未编号报告读回沿用现有 Windows Feature 通道的返回长度处理，不接受截断帧或自行移位猜测。

无线流程依次 F6/05 选鼠标、F7 等待发送就绪、鼠标查询、F7 等待读取就绪、FC 通知读出、Feature GET。F7 的字节 0/5 为读取/发送就绪，字节 4 为鼠标离线标志，字节 2 为电量；字节 1 是键盘电量，不能混用。就绪标志和在线标志限 0/1，全零、请求回显、错误命令与越界百分比拒绝。每段就绪等待最多八次并有时间上限，单次 Feature 传输限时 150 ms。充电没有已证明字段，保持未知；USB 不使用接收器电量命令。

鼠标请求 byte 7 为前七字节和的反码，回复该字节不随返回数据重新校验，不能强加不存在的回复校验算法。查询均完整匹配命令并拒绝原样请求回显。协议没有事务号；前后身份检查与反向复读降低错误归属风险，但不能证明接收器内部回复一定新鲜或彻底排除其他进程的同命令结果。

官方 `getMouseOption0` 从 D3 回复 byte 8 读取当前配置，目录 `layer=8`；D4 查询必须携带这个配置号。上游驱动固定 `currentProfile=0`，本实现不沿用。D4 byte 2/3 分别为当前槽与存储槽数量，X 从 byte 8、Y 从 24 按小端双字节排列，颜色从 40 起每槽三字节。官方按各槽 X/Y 同为零判断禁用，允许中间空档；显示保留原编号与重复值。目录范围为 50～32000、步进 50，上游 AJ179 实机记录为 50～26000；更高有效值只读，任一启用轴超过 26000 时不开放整表写入。

切档令牌包含型号、完整 D3 与 D4；写前重新检查两次配置并确认型号。54 写入仅替换命令、当前索引和请求校验，所有槽位、禁用槽颜色及未知字节从原始 64 字节复制。等待 400 ms 后核对完整表、原配置及型号，发送完成信号丢失后只读确认，不重发、不盲目回滚。D3 不写，按键、宏、配置选择和回报率不修改。当前回报率只解析已确认编码，没有默认 1000 或 125 Hz。

原始公开帧解析为 AJ179 PRO、85%、当前第 3 档 1000 DPI，八个存储槽中五个启用。回归另覆盖非零配置、中间禁用槽、分离轴、较高 DPI 只读、配置竞争、失去完成信号、写入未生效、相邻字段变化、休眠与后台恢复。本项目没有对应硬件，Windows caps、无线缓存、断电保持和实际切档仍待验收；同型号替换及跨连接合并也未解决。

## 1.4.7：狼蛛 F75 / F87 PRO 键盘

2026-11-21 从 [狼蛛官网](https://www.aulastar.com/aula-hub/) 下载 AULA HUB 1.1.22，仅读取安装包内资源，未安装或运行厂商程序。官网列出 F75 / F75 MAX / F87 PRO V2 / F99 / F108 PRO / F108 PRO V2；本批只按精确型号接入 F75 与 F87 PRO，目录中的其他型号仍不代表已兼容。网页入口当日返回 403，改用官网提供的离线安装包进行静态核对。

安装包压缩文件 SHA256 为 `f18f4bfa2a6d66f24bed8d2c17b61eff8530ebc9ad0615344f72003f5bf9db08`。离线网页入口 `index-npIkQlTa.js` 为 `0b97fd16dc42ad77da883c318af8924318aeebd759270c944518c8afe7278090`，Bytech 控制代码 `index-BPTK-vUa.js` 为 `3090652c88abbaf61e68f1b643929ff4995068d309dbd6307f8aa6452e00d19b`。

F75 型号码 `03 00 00 00 00 CD` 与 F87 PRO 的 `03 00 00 00 03 E3` 接入现有 `Keyboard20` 共用接收器，不新建第二个监听同一 `3554:FA09` 通道的后端。接口必须满足既有的 `FF02:0002`、20 字节 Report `13` 和同接口组唯一管理集合。F87 的官方无线目录使用 `FF00:0001`，与这条接收器路径冲突，因此不识别；未知型号码不会仅凭接收器编号命名成狼蛛。

狼蛛身份帧为 Report `13`、命令 `05`、参数 `01`，补齐到 20 字节；累加和覆盖完整帧 byte 0～7 并写在其后的 byte 8。电量命令 `4A 01` 补齐到 20 字节，没有同一校验字节。回复按 20 字节完整帧、`13 command 01 00` 前缀和原样回显拒绝校验，型号从完整帧 byte 5～10 读取。电量 `1～100` 有效，高半字节 `1` 为充电、低半字节 `1` 为充满；任一标志不是 0/1 或电量回显为零时不发布新样本。按照狼蛛官方界面，100% 时高半字节仍可表示充电，不套用迈从“满电即不充电”的显示规则。

共用接收器先尝试已知缓存协议，未命中时先查狼蛛，再查迈从。成功协议按 Windows HID 实例缓存 120 秒，避免每轮都等待另一协议的 2.2 秒超时。狼蛛身份只有精确型号命中后才继续电量查询，并在电量后再核对相同型号；已被确认为狼蛛的通道即使休眠唤醒失败，也不把该回复改交给迈从解析。测试覆盖两种请求格式、两条型号码、错误型号、充电与满电半字节、未知标志、零占位、电量前后替换和设备实例隔离。

- F75 与 F87 Pro 无线目录使用 `3554:FA09 / FF02:0002`，按六字节型号 `03 00 00 00 00 CD` / `03 00 00 00 03 E3` 区分；有线为 `258A:010C / FF00:0001`。F87 的无线目录却标成 FF00:1，不能直接扩展到相同 FF02 接口。
- 无线类选择真实 input/output 报告 ID，发送 19 字节报告体，身份命令 `05`，电量 `4A`。身份查询把包含 report ID 的加和放在完整帧 byte 8，电量查询未附同样校验；与现有 MCHOSE 请求格式须分开处理，不能只追加型号名称。
- 解析器从 WebHID 的无报告 ID 载荷 byte 4 开始取六字节型号，电量也从 byte 4 开始，后一字节高半字节 1 表示充电、低半字节 1 表示充满。官方的主动状态处理存在同一通知按不同用途解释的情况，尚不作为独立在线或电量证据。
- 有线类发送 519 字节 Feature 报告体，身份 `82`、电量 `87`；剥除报告 ID 后跳过七字节头部，读取独立数据区。此流程不能证明共用接收器的 8 字节 Feature 06 回复 byte 3 是电量。MyFinder 的 #99 / #485 / #477 反馈已否定后者，继续保持该路径禁用。
- F108 Pro V2 的新固件使用 HFD 协议，不能套用上述 Bytech 类。F108ProV2C 已按独立官方目录和 56 字节 HFD 信息协议实现，见文末 MT560、R9 与狼蛛、达尔优章节；无 C 变体仍不由这条路径推断支持，实际 caps 和冷启动仍待验收。

测试帧均为按官方静态代码构造的合成数据；本项目没有 F75 / F87 PRO 实机，Windows SET_REPORT 传输、休眠缓存、同一接收器上的迈从 / 狼蛛切换及厂商程序共存仍待验收。

## 1.4.7：狼蛛 SC380 PRO

核对官网 SC380 PRO 安装包（SHA256 `99a0987b762d0dc6e4b3ecb23afc32c2c57debf547f9b0e9ed503830e096b9b5`）内 `Dev/ms/MS.ini`，明确有线 `258A:0136`、无线 `258A:0138`、型号签名 `04 00 00 00 00 2E`、PAW3311。结合 OpenMouse `5a4f643` 的 [SC380 PRO 协议记录](https://github.com/OpenMouse-Project/mouse-protocol/blob/5a4f6433da3d70865d008874c7055016b336c197/docs/aula-sc380-pro.md) 独立实现。仅开放有上游 Windows 记录的无线 `FF02:0002 / Feature 09 / 520`，同接口组须唯一管理集合，描述符须确认唯一 Feature 09；不以名称或其他相似集合代替。厂商资源和上游实现不随源码分发。

Windows 报告头为 `09 checksum command argument 01 00 lengthLE16`，载荷从 byte 8 开始，补零至 520。请求校验为 byte 2 起的累加和；短回复沿用请求头校验，`44` 配置回复则包含有效数据。匹配完整长度、命令、参数和数据长度，尾部残留不提供新鲜度证据。`05/01/6` 查询型号，`07/00/1` 查询在线，`09/01/1` 查询电量；在线仅接受 1，电量零载荷与请求相同，因此不当成实际 0% 或离线。充电没有已确认字段，保持未知。每次查询后等待 30 ms，单次 Feature 传输限时 150 ms，每段操作有 3.5 秒截止时间。

`44/00/400` 读完整配置，marker 为 `64`，sensor 低五位为 `18`。byte 3 低半字节为启用数 1～6，高半字节为一基当前档；X 从 byte 7、Y 从 27 按小端双字节排列，DPI 为 `(word + 1) × 100`，只接受 100～12000。byte 2 低半字节 1～4 分别对应 125/250/500/1000 Hz，只读不写。保留实际原序及重复值；官方安装包默认六档不作为读取结果。

配置读取要求两份完整一致的回复。切档令牌包含型号及完整 400 字节，写前再核对完整配置、型号与在线；`04/00/400` 只改变 byte 3 高半字节，其他字段原样保存。等待 250 ms 后完整读回两次并再次检查身份/在线。发送完成信号丢失只读确认，绝不重发或自动回滚。设备、配置竞争或相邻字段变化时不能报告成功；没有跨进程原子性或同型号设备唯一身份，仍有需要实测的边界。

回归覆盖畸形头、不同校验范围、零载荷回显、范围溢出、配置竞争、完整块保留、发送失败后的读回以及统一主窗口/托盘过期恢复。新增用例为按协议事实构造的合成报告，并非本项目实机抓包；无对应硬件，实际 caps、唤醒、物理切档和断电持久性待验收。

## 1.4.7：八位堂 Pro 3 手柄

核对 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的公开协议记录，以及 SDL 固定提交 `5e7b0bd9f1fbe7e661cf6bb0cdbf31d37be2cf50` 的 [8BitDo HID 驱动](https://github.com/libsdl-org/SDL/blob/5e7b0bd9f1fbe7e661cf6bb0cdbf31d37be2cf50/src/joystick/hidapi/SDL_hidapi_8bitdo.c)。SDL 文件 SHA256 为 `084a79270595e7a99c15b89c7e7b5f167fc1ae0156df8c51537e5c085fe8ed53`。只独立实现电量读取，不复制 SDL 源码或资源。

仅匹配 D-input 模式 `2DC8:6009`、标准 Gamepad `0001:0005`，描述符须证明所有 Input Report ID 均为 `04`，输入容量 15～64 字节。XInput 模式使用其他 PID，不进入这条解析器。程序打开只读句柄，最多接收 64 个报文、等待 400 ms；不发送 Feature、Output、模式、灯光、震动或初始化命令。SDL 对其他 8BitDo 型号会读取 Feature 06 来启用增强报文，而 Pro 3 已有用户在无 Steam、刚开机的 D-input 蓝牙模式确认 Input `04` 自带电量，因此这里不复制该初始化步骤。

完整 Input Report byte 14 的低七位是百分比，高位置 1 表示充电；0～100 保留，101～127 拒绝。按照 SDL 的 charged 映射，100% 作为已充满，`charging=false`，避免在托盘继续显示正在充电。每个报文提供新的采样时间；无有效报文不延长旧值，120 秒后隐藏，后台发布间隔上限 10 秒。

回归覆盖错误 VID/PID、错误 usage、报告 ID、短包、0%、101～127、充电位、100% 已充满、噪声跳过、边界数量及同型号实例隔离。测试帧为根据公开字段构造的合成数据；本项目没有 Pro 3 实机，实际 Windows 描述符、插拔充电、休眠恢复、多手柄及与 Steam / 官方工具共存仍待验收。

## 1.4.7：VGN Dragonfly3 SE

核对 VGN HUB 固定资源 `index-ZngoA_qG.js`（SHA256 `53adfab22f478249bdb9117bb26cedf8a7d3c6c561f2f2eb735e7d86e943be63`）中 `Yd.GetBattery=48`、`oc.getBattery` 和 `sendHidBuffer` 路径，并交叉核对 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `engine/providers/vgn_dragonfly3.py`（SHA256 `e91a2f755b2d6b84de3d088cfd7900d62300b3fb67a3762b88ccea9106c2e4bc`）。只独立实现协议事实，厂商脚本和上游实现不进入源码或发布包。

SE 路径匹配 2.4G 接收器 `391D:1A01 / FF01:0010`；描述符要求输入、输出都只有无编号 Report 0，Windows 容量各 65 字节，同接口组只有一个候选。Pro `1A03` 的独立匹配规则见后文；已诊断的 `FF05:0002`、有线 `1001` 及其他 PID 不探测。输出请求为 `00 55 30` 加 62 个零；按 Windows 65 字节容量接收后移除零 ID，协议解析使用 64 字节载荷。

回复只关联 byte 1 的 `30`。byte 9 为 1 时表示有线，不发布电量并明确清除旧百分比；byte 9 为 0 时，byte 8 接受 0～100。原样请求回显、短包、越界值和未知连接枚举不产生新样本。厂商路径没有确认充电字段，因此保持未知。单次查询最多等待 1 秒、接收 32 个报文，后台发布间隔上限 10 秒；失败不刷新时间，120 秒后旧值失效。

回归覆盖精确 VID/PID、usage、无编号 65 字节容量、请求布局、0/100、101～255、有线清除、请求回显、短包及设备实例隔离。测试帧为按公开字段构造的合成数据；本项目没有 Dragonfly3 SE 实机，实际 Windows caps、接收器缓存、休眠恢复、物理电量、0% 和官方软件共存仍待验收。

## 1.4.7：HECATE G3M PRO

核对 MyFinder `73d99e2087dd70ae8eef4199854e0c495a66f0bc` 的 `engine/providers/g3m.py`（SHA256 `520c397ec971eba8d8d49eb2720721c3b9bcf15bcf1e5f6426c3991ca848349b`）以及其引用的 BoBppy/g3mpro-macos `6271ac2` 协议事实。只独立实现电量读取，不复制上游代码或资源。

仅接入 `320F:706E / FF1C:0092`，描述符必须证明输入、输出都只有 64 字节 Report `04`，同接口组唯一候选。请求为 `04 20 00 1A 06` 加 59 个零；byte 1 的 `20` 等于请求 byte 3 起的累加和。回复要求完整 64 字节、Report `04`、命令 `1A`、byte 7 不为 `FE/FF`，百分比 byte 8 为 0～100。原样请求回显、短包、错误报告 ID/命令和错误状态均拒绝。

回复校验和与充电字段没有确认依据，不猜测；只读、不开启配置会话、不读写 DPI、灯光、在线握手或固件。单次查询 1 秒，后台发布间隔上限 10 秒，失败不刷新旧样本，120 秒后旧值隐藏。测试使用按公开字段构造的合成帧，本项目没有 G3M PRO 实机，实际 caps、休眠唤醒、0%、物理电量和官方软件共存仍待验收。

## 1.4.7：接收器、键盘与耳机协议

只独立实现已核对的报文事实，不引入外部项目源码、测试工具或厂商资源。对应代码、型号边界和实机状态见[兼容性说明](COMPATIBILITY.md)。

| 路径 | 核对依据 | 报文事实与限制 |
| --- | --- | --- |
| VGN YJX Pro / ryMs | 官方 VGN HUB 本地静态资源 `vgn-main.js`，SHA256 `2dc306331d30f874eec8a006f975ef5cac3ed2f18d3bef58e5dd089dc8d5cd5f`；FerrisPulse `a916c657c18034a30300dd08b33c6c192c86fb06` 的 `VgnDeviceHelper.cs`；MyFinder `aa0228abbc850b693620d466147b35dfe50c8dca` 的 `vgn_ry_ms.py` 与 PR #717 | `1A03` 与 SE 同 YJX 命令 `30`；官方解析将后一个字节作为有线状态，不照搬上游的充电推断。六组 ryMs 发 F7、前七字节和补码 08、Feature 0/65，完整帧 byte3 电量、byte5 休眠；不发送 F6 初始化或 FC 配置命令 |
| 迈从 4D | 官方 M HUB 本地静态 `main.js`，SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`；Voltide `c1872c4c93124d62c05bf5887fe89c32c6be8dd6` 的 K7 provider；MyFinder `aa0228a` 的协议说明与 #535 Windows 描述符 | 固定 `4D 01 01 00 00 09 00 00 08`，64 字节；版本、命令、业务码、序号、声明长度、可选 XOR、鼠标 VID、连接和电量共同校验。公开 Ultra+ 数据体 85% 充电和 42% 放电用于离线回放，传输补零重建，不标成本机抓包；未知充电标记保持未知 |
| 迈从 K99 V3 | [mchose-hid-bridge `ad74f3e75fd5ae099b2ea4af039678c54dea73ac`](https://github.com/arutkowski00/mchose-hid-bridge/tree/ad74f3e75fd5ae099b2ea4af039678c54dea73ac) 的完整40%回复、描述符和100%记录；M HUB SDK SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3` | Report0、64字节载荷，`55 03 00 38 38` 查询、`AA 03` 回复byte8百分比。G87 V2配置字段不能证明K99充电含义 |
| 水月雨 GM01 Pro | [MyFinder #127 回复](https://github.com/wushuai0802/myfinder-device-battery/issues/127#issuecomment-6687188223)、固定 `aa0228a` provider；已保存 BatteryHub `632f204c50c43355616bd32d17483c63b1bcac54` 的 `moondrop.cpp`（SHA256 `c6b0bf3bb9496740487f65fa64f3d17d982d99381d8218f41f1195afe617753d`） | 用户回复 `4B 80 27 06 00 00 00 00 01 32` 为50%；固件1.5.3记录四集合位于MI03，普通音量集合与电池集合分开。官方原始Hub脚本本次未独立取得，不声明已核对 |
| 雷柏 VT 1851 | [FerrisPulse #76](https://github.com/Ferris-echo/FerrisPulse/pull/76)、固定 `a916c657` helper SHA256 `1916585f63a15e2e1108fd5b643d65ad8d9a02f353d470de3c010085302cdee0` | 输出BA、GET_REPORT(Input)，不是ReadFile输入流；31/32字节及报告ID保留/替换布局分别解析。标记0配置回包拒绝，标记1/2放电/充电；只发查询，不开启EEPROM写入；接收器缓存仍待实机核验 |

Windows `ReadFile` 接收缓冲必须使用包含零 Report ID 槽的完整 HID caps 容量，收完后再从明确的无编号报告剥掉零字节；不能先把缓冲长度减一。共用中断收包路径适用于 VGN SE / Pro、K99 V3 及其他调用该接口的设备。GET_REPORT(Feature) 的返回长度处理不同，保持原有行为。

新路径均接入后台轮询、脱敏诊断和统一缓存。协议测试、公开回复回放与本项目实机验证分开记录；本批暂无新增型号本地硬件验收，不能声明睡眠缓存、充电转换或厂商软件共存已全部解决。

## 1.4.7：国内鼠标、键盘与耳机协议

以下协议分别实现，只查询状态。

| 路径 | 核对依据 | 实现与限制 |
| --- | --- | --- |
| 雷柏 MT760 `24AE:1870` | FerrisPulse `a916c657c18034a30300dd08b33c6c192c86fb06` 的 MT760 helper 与 PR #75；PyFlat/Device-Battery-Info PR #36 的完整 Windows 47% 回复；Linux 同族描述符 | SET_OUTPUT_REPORT 发固定 `BA A5 AA`，GET_INPUT_REPORT 指定 BA 却收到直接 status 布局；只接受完整 32 字节，不借用 1851 的另一个 ID 布局。状态 `02` 仅重复限时 GET，最多 24 次 / 700 ms；`01 02 7F` 表示充电但百分比未知。无事务回显，接收器缓存仍待验证。 |
| ATK / VXE NANO `373B:1216` | 官方 [ATK HUB 3.2.33](https://v3-hub.atkgear.com/) 的 `useRemoteConfig-DYQXy-Fo.js`，SHA256 `78a7c52ed2cbd859f47ac68d535a369fa26d7775817d0b20e8c020f1418f24d3`；HaloBattery PR #350 提交 `926bfc0affaf293d0c7af0bc949929568281d68f`；atk-tool PR #15 | BITMOUSE 固定 `08 7D 72 02 00 01 07 01`，只读 GET07。输入 ID 与容量独立从字节数组描述符确认，按 Windows Configuration Manager 的 USB 父节点检查候选唯一性。公开 90% 只提供前缀，其余测试填充为合成；序号不当充电或电压。 |
| 狼蛛 F87ProV2 | MyFinder `aa0228a` 的完整 48% 记录；官方 HFD SDK SHA256 `888b4493bf0bcd25aceaefe29f4a9b0a6ee84b486cab92bbd31d531be4394d45` | GET_DEVICE_INFO 页 0。payload byte 3 / 4 是页地址低 / 高字节，必须同时核对；byte 25 为百分比。官方目录 FF68 不据此覆盖公开实机 FF60，未知充电字段不解读。 |
| 狼蛛 F99 MAX | [MyFinder #858](https://github.com/wushuai0802/myfinder-device-battery/issues/858) 的完整 68% 两页回复及修正 Windows 报告长度后 71% 反馈；独立 F99MAX 1.0.1 SDK `index-CPx-FK1k.js` | 型号专用 contentSize=48，读取页 0 / 18，各自 32 字节载荷加零 ID。与通用 HFD 56 字节三页不同；先确认固件 `0C45:800A`、无线模式和电量才发送第二页。只核对充电字节为已知 0/1，不据此声明充电含义。 |
| 前行者 X87 | futao-awa/EWEADN_x87_Agreement `cea036b` 的 `hid.js`，SHA256 `f04e7dc9c28e9584da2572cd565bf358ee762a9ad944c5a1de80300311683495`；marcelowbr/eweadn-x87 `3316109` 的 `protocol.py`，SHA256 `2b7adb1e158fb8f2ebe1030166ffa674cddfac36b246ea9c6a306c59c38aab8c`；MyFinder #262 Windows 接口记录 | 一页 GET_INFO，manufacturer=2 / product=5 / mode=2 才命名为 X87，0 占位不发布；不使用对其他同编号键盘的推测。上游电量读取也未完成实机验收，本地测试为合成帧。 |
| 迈从 V9 PRO 2 | 本文 M HUB 完整 `main.js` 快照及其 MC codec；BatteryHub 已保存提交 `632f204c50c43355616bd32d17483c63b1bcac54` 的硬件记录；MyFinder `aa0228a` | `55 AA 01 00 01 00 01 0B 00` 查询；验证 MC 类型、子命令、声明长度及可选 XOR，接收 `0001/0B` 或 `0004/22`。body 首字节为休眠，下一字节低七位百分比、高位充电。上游仅确认电量，充电和休眠仍待本项目验收。 |
| 迈从 X9 | HeadsetControl `62e361b085d7716805a3bf524bc9cca3761e0e15` 的 `mchose_x9.hpp`，明确 USBPcap 对照 M HUB；本文官方 BLK codec | 独立 `6045 / MI00` 身份，复用 FF90 固定查询而不查询 V9 PRO 固件。`02` 为有效百分比，`03` 仅显示充电，`1A` 清旧值；`04` 不造 100%。测试 20% 是合成，不能写成 X9 实机百分比。 |
| 西伯利亚 K06S | [MyFinder #846 实机回复](https://github.com/wushuai0802/myfinder-device-battery/issues/846#issuecomment-7071675681) | 只启用成功的 FF90 / Col03，固定 `55 65 00 00`；FF82 失败路径排除。公开 64 字节回复只给出 `55 65 00 64` 前缀，测试尾零重建；0 与请求回显不可区分，充电未知。 |

共享状态模型支持仅充电回报：新鲜的充电状态完整替换旧样本，百分比清空，显示充电并按本次状态时间过期。它不会把旧数字复制进新样本。新增回归覆盖首次充电未知、已有数字转充电未知、持续失败过期和恢复百分比，主页面与托盘使用同一读数。

所有新型号本项目实机验收仍待完成。ATK NANO 的实际拓扑和描述符、各接收器关机缓存、热插拔及厂商软件共存不能由离线测试证明。后续章节分别记录 ATK 8K Nano、雷柏 MT560、迈从 K87S、狼蛛 F108 Pro / V2C 等独立实现；这些路径各有证据和门禁，不直接扩大本批 PID 表。

## 1.4.7：ATK 8K Nano、灵豹及键盘和耳机协议

以下五条路径均独立实现报文事实；上游实现、厂商资源、逆向工具和测试设备数据不进入发布包。

| 路径 | 核对依据 | 实现与限制 |
| --- | --- | --- |
| ATK 8K Nano `373B:1178` | 官方 [ATK HUB 3.2.33](https://v3-hub.atkgear.com/) `AtkRadioGroup-_tra2Wmk.js`，SHA256 `20497b7e1f847bf95474971c585a690a5e3d899a4278c20fe369075b52f8cc2b`；BatteryHub `632f204c50c43355616bd32d17483c63b1bcac54` 的 `atk_bitmouse.cpp` 交叉核对 | 独立 Holtek 路径：`FF10:0001`、双向 Report0/65、Feature0/9。先打开输入并限量清旧队列，再发送一次 `00 82 00 00 00 00 00 00 00`；最多 900ms / 32 帧。完整 65 字节回复带 Windows 零 ID，payload8 低七位为 1～100、高位充电；payload9 为 0 清旧值。真实 USB 父节点保证唯一集合，不读版本、不配对、不发设置命令。 |
| 灵豹 M1 Pro `088D:052F` | [MyFinder #171](https://github.com/wushuai0802/myfinder-device-battery/issues/171) 的两条完整 65% 报文；固定提交 `aa0228abbc850b693620d466147b35dfe50c8dca` 的 `lingbao.py`（SHA256 `6d243070991a115a03b468495bd03c0fd749129191e0d194de168a40c361c084`）与协议说明 | 完整产品名、MI01、FF00:0001、真实 USB 父节点及唯一 Feature0F/33 逐报告字节数组证明；一次 GET_FEATURE，没有任何写传输。33 字节回复的 byte7～10=`M536`、byte13～14=`LX`；只接受 byte19=0、byte18=1～100。flag1 全部舍弃，不以充电占位刷新历史；charging 未知，byte21 不解释。 |
| 迈从 K87S `3837:3033` | 官方 [M HUB SDK](https://www.mchose.com.cn/cizhou/CZ_SHARED_DATA/main.002bbdb0f17b5f7135ee.js)，SHA256 `d47d1d0e9e86efbbc1888d17f3bc5cd5caba73c06149a6f1fe26bb3d9386cbc3` 的 type130 / GLW 目录与 codec；官方 `audioTrayDevices-O2LjYPNA.js`，SHA256 `0ccd97c9d36a216c7c3c0c29f76fd772a25a0f4b0d7964e1a577fcd4cef60d5c` | 只接受完整 `MCHOSE K87S 2.4G`。命令04读base、05读当前配置56+8字节、再读base确认前六字节和槽位未变化，共四次只读请求、总计3秒、每页最多32帧。config32低七位为电量，但型号没有1%精度标记，按官方十个百分点档显示≈；0和越界拒绝，充电未知，不修复配置。 |
| 原版狼蛛 F108 Pro `05AC:024F` | [AULA F108Pro 官方驱动](https://cdn.aulacn.com/keyboard/AULA%20F108Pro%20Driver-1.0.0.3.exe)，安装包SHA256 `62babdfdca614e5c27919d26278c845c549f3943bf2076cef0b7d56ae7e2afe8`，内部DeviceDriver.exe SHA256 `c5dda120a840de278076c405c5faf3d1137bc440de882087c77f69d61e1a3ad7`；MyFinder #187 / #862 接口清单 | 仅静态提取与反汇编，未运行厂商程序。原生0x4357D0电量函数通过0x44F4E0事务发送33字节 `00 20 01 00…21`，最后索引32为累加校验。回复去零ID后32字节，前三字节20/01/00，byte3只接受1～100；充电未知。完整产品名、MI03、FF60:0061与双向Report0/33门禁；600ms / 32帧；不误用F99 MAX的HFD协议。 |
| FIFINE HT6 `3142:00C3` | [HeadsetControl `62e361b085d7716805a3bf524bc9cca3761e0e15`](https://github.com/Sapd/HeadsetControl/blob/62e361b085d7716805a3bf524bc9cca3761e0e15/lib/devices/fifine_ht6.hpp) 记录 SoundLink 抓包、接收器1.8与耳机4.2固件；`test_fifine.cpp` 的80%前缀 | 精确MI00、FF90:0001、双向唯一55/64；一次 `55 65 01 00`、1秒 / 32帧。`55 65 percent 02/03`分别为放电/充电，percent0～100；命令0A为关机，65回复中的状态1A为断链并清旧值。其他命令的1A不能触发断链，不移植RGB、睡眠设置或Flash操作。 |

灵豹接收器成功返回 Feature 不能独立证明鼠标 RF 在线或物理测量新鲜度。当前统一快照只有布尔连接字段，有效读数表示本轮接收器查询成功；不能据此声称已解决接收器缓存或休眠状态。两条完整65%回包之外的68%/66%及充电100/flag1场景仅有表格观察，测试变体明确作为合成数据。ATK 8K Nano、K87S、F108 Pro的回归数据也为合成帧；HT6的公开前缀与填充重建分开记录。

回归覆盖统一静默过期与明确断链恢复。样本120秒过期，F108 Pro后台发布间隔上限15秒，其余10秒；无回复或灵豹充电占位不延长旧样本。五款设备均无本项目硬件验收。MT560 与 F108ProV2C 的独立实现见下方章节，实际描述符与完整实机回报仍待验收；原版 F108 Pro 协议不能外推给 V2C。

<a id="domestic-147"></a>

## 1.4.7：MT560、R9 与狼蛛、达尔优设备

本节包含五条独立协议路径，覆盖 MT560、V9Pro+、R9、EM901 / CM610 Pro 及三款狼蛛 HFD 机械键盘，对应实机验收仍待完成。下列公开用户观察与合成用例分别说明，不以静态分析或上游成功替代硬件验收。上游实现、驱动资源和研究文件不随发布源码分发。

### 雷柏 MT560：持续监听 Telink8273 BB

官方 [A HUB 脚本](https://hid.rapoo.cn/AHUB_WEB/dist/assets/js/index-cgGQw9wn.js)，SHA256 `09a7906e669d75275a77ab394207bb6ab7264efa6eb6998fea3c792a31a36ecf`，将 MT560 分配到 Telink8273_Mouse：有线 `24AE:4504`、无线 `24AE:1872`，主集合 `FF00:000E`。[MyFinder #1060](https://github.com/wushuai0802/myfinder-device-battery/issues/1060) 提供无线身份和集合清单，没有完整 BB 报文或逐报告描述符。只启用 `1872`，不使用 MT760 或旧 VT 的 BA 查询。

官方处理器直接接收 Report `BB`、载荷头 `B0` 和至少六字节载荷；Windows 完整帧 byte5=1 时 byte6 为 0～100%，byte5=0 是无效电量，不能推断离线或充电。运行时要求真实 USB 父节点下唯一管理集合，输入字节数组只含 BA/BB，且单独证明 BB 长度至少七字节。原生输入允许 BB 自身长度或已证明的集合最大容量填充，其他长度不接受。仅打开只读输入句柄，不发送任何 Output / Feature 操作。

监听跨轮询保留，每批最多处理 32 帧；队列排空后发布该队列中最后有效样本，保留其原始主机接收时间。没有新有效报告不会续期，120 秒过期，后台发布上限两秒。30 秒静默或立即读取失败会触发重新打开，重开退避两秒，因此稀疏报告可能落入短暂监听空窗；该恢复策略尚需实机节奏验证。原生 I/O 取消仍需等待 Windows 完成，异常驱动下不能承诺停止时间严格有界。以上限制不影响“没有设备写入”的边界，也不能证明接收器缓存就是新的物理测量。

### 狼蛛 V9Pro+：04/1A 电量

来源为固定 [MyFinder `aa0228a` 的读取器](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/providers/aula_mouse.py) 及[协议记录](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/upstream-protocols.md#aula-v9pro-320f22ba-24-ghz-receiver)。其公开记录在 2026-11-18 分三组各九次观察拔线 / 充电 / 再拔线，百分比 80→82→82，充电字段 0→1→0；这是上游贡献者硬件证据。

精确 `320F:22BA`、`AULA-V9Pro+`、`MI_01 / FF1C:0092`，双向唯一 `04/64` 字节数组和真实 USB 父节点唯一集合。固定请求 `04 00 00 1A` 补零至 64 字节；完整同头回复 byte8 为 0～100，byte9 只接受 0/1。参数 byte4 不作为事务回显，也不猜回复校验和。0 且未充电的状态与查询回显无法区分，拒绝；0 且充电可保留。每轮一次 GET、最多一秒 / 32 帧，失败不换接口重试。查询锁内重新枚举，无法读取的同型号集合不能被当作不存在。

### 迈从 R9：独立拓扑约束的 MC 电量

官方 [M HUB `main-DIt5vAcV.js`](https://www.mchose.com.cn/assets/main-DIt5vAcV.js)，SHA256 `cb10c9d3d29505bef2e2d3207dbe223601ff84c46d772b4a0f1e93ef02391c1b`，明确 `3837:6013 / MCHOSE R9` 使用 MC profile 和 Report `55`。公开 [#373](https://github.com/wushuai0802/myfinder-device-battery/issues/373)、[#495](https://github.com/wushuai0802/myfinder-device-battery/issues/495)、[#496](https://github.com/wushuai0802/myfinder-device-battery/issues/496)、[#497](https://github.com/wushuai0802/myfinder-device-battery/issues/497) 补充 `FF07:0222`，没有完整 R9 回包或逐报告描述符。THX 音频的 MI00 不作为电量接口依据，也不借用 V9 PRO 2 的 MI03。

运行时解析真实 USB 父节点，要求唯一 `FF07:0222`、完整产品名和双向 `55/64` 字节数组。固定 `55 AA 01 00 01 00 01 0B 00` 补零至 64 字节；GET 类型 `0001/0B`、通知类型 `0004/22` 分别校验帧头、版本、标志、声明长度和可选 XOR。未启用校验位时校验字节仍须为零。body0=0 清除旧电量，其余状态使用 body1 低七位 0～100 与高位充电；查询回显不构成样本。

每轮在物理父节点锁内重新检查枚举和报告，未知拓扑或无法读取的同型号集合阻止查询。限量清旧队列后只查询一次，最多一秒 / 32 帧，无握手或设置。测试帧与接口号是合成数据；官方帧定义不等于 R9 冷启动、充电、睡眠或厂商程序共存已验收。

### 达尔优 EM901 / CM610 Pro：Feature08 与 Input09

固定 [MyFinder `aa0228a` 读取器](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/providers/dareu.py)、[协议记录](https://github.com/wushuai0802/myfinder-device-battery/blob/aa0228abbc850b693620d466147b35dfe50c8dca/engine/upstream-protocols.md#dareu-em901-260d1074) 与 [#81](https://github.com/wushuai0802/myfinder-device-battery/issues/81) 记录 EM901 查询；所分析驱动快照 SHA256 为 `afac433fbd34b67994886a079a917c832c7f5e12cd22189e5fd84b303f56052e`。[#725 的 CM610 Pro 用户记录](https://github.com/wushuai0802/myfinder-device-battery/issues/725#issuecomment-6969007615) 及其[读取脚本](https://github.com/user-attachments/files/35007352/dareu.py) 补充分离集合与回复累加校验。

仅 `260D:1074` 和产品名 `2.4G Wireless Receiver`。Feature `08/17` 请求为 `08 04`、十四个零和尾字节 `49`；输入必须为 17 字节 `09 04 00`，byte6 为 0～100，byte7 非零表示充电。同一 `FF02:0002` 集合若声明 Input09，必须在该集合证明长度并读取，不因无回复而改用其他接口。只有未声明同集合 Input09 时，才允许同实际 USB 父节点下唯一 `FF01:0000` 输入，两端厂商字符串必须都是 `Compx`。分离路径另外要求完整回复加和模 256 为 `55`；同集合路径没有已确认回复校验，不强加该规则。

接收句柄先打开，限量清理旧队列，单次 Feature 查询后最多等待一秒 / 32 帧；发送本身限时 250 ms，失败不重试。锁内重新枚举和验证报告；同型号不可读集合阻止查询。没有 Output、GET_FEATURE、握手、配置、DPI 或固件操作。显示名保留 EM901 / CM610 Pro，因为共享 PID 和通用产品名不能证明某一个零售型号；PID1090 未启用。用户分离通道成功是上游证据，本项目的更严格 Windows 描述符路径仍待验证。

### 狼蛛 F108 Pro V2C、F87 Pro V2C 与 NOVA98：HFD56

官方 [AULA HUB 下载入口](https://www.aulastar.com/aula-hub/) 指向 [AULA HUB.zip](https://www.aulastar.com/AULA%20HUB.zip)，归档 SHA256 `f18f4bfa2a6d66f24bed8d2c17b61eff8530ebc9ad0615344f72003f5bf9db08`。静态读取其中 Hub 1.1.22 的 `index-npIkQlTa.js`（SHA256 `0b97fd16dc42ad77da883c318af8924318aeebd759270c944518c8afe7278090`）、`info-DZvc7M4P.js`（`536e776b9dd8daff26a0be434bf47bf9960d550714fffb0b2ec206556bfc79a8`）与 `hfd-sdk.es-CwIM3CGY.js`（`888b4493bf0bcd25aceaefe29f4a9b0a6ee84b486cab92bbd31d531be4394d45`）；未执行厂商程序或脚本。

| 完整接收器名，共享 `0C45:FEFC` | 集合 | 信息中的固件身份限制 |
| --- | --- | --- |
| `AULA F108ProV2C Dongle` | `FF68:0061` | `38A6:273A/2749` |
| `AULA F87ProV2C Dongle` | `FF68:0061` | `38A6:2712/2738/2747` |
| `AULA NOVA98 Dongle` | `FF80:0071` | `38A6:273B/275D` |

上述目录分别走 Pi / qH，并共用同一 Info 类和 `getDeviceInfo` 56 字节读取器。FF67、FF70 是权限请求别名或辅助集合，不能当作查询回退。每个物理 USB 父节点只允许一个匹配管理集合，运行时证明双向唯一 Report0 和 33/33 或 65/65 字节容量；不硬编码未捕获的 MI。发现及读取均锁内重新枚举、拒绝同 PID 未读集合后证明目标报告。

完整 Windows 请求头为 `00 AA 10 len addrLo addrHi 00 last 00`；33 字节容量按偏移 0/24/48 读取 24/24/8 字节，65 字节容量一次读取 56 字节。剥除零 ID 后，回复必须完整匹配 `55 10` 和两字节地址，数据从 byte8 开始。SDK 把 byte2 命名为 lenOrType，未证明统一回复校验或尾标志，因此不猜测。首段 data4/5 与 data6/7 必须匹配上表固件身份，再继续余页；这是目录与字段的保守一致性门槛，尚无完整硬件捕获证实该映射。

data17 仅接受 1～100；data16 工作模式和 data18 充电状态没有足够型号证据，保持未知。样本时间取首段信息接收时间，后续页面完成不续期；后台发布上限三十秒，120 秒过期；最多三页、每页 600 ms / 32 帧、整轮两秒。原版 F108 Pro、非 C 的 F108ProV2、F99 MAX 和其他共享 PID 不进入此读取器。不发初始化、校准、灯光、配置或固件命令。

本节用例覆盖身份冲突、报告长度、错误回包、状态过期和恢复。V9Pro+ 与 CM610 Pro 的公开用户记录需与 MT560、R9、HFD 键盘的合成数据分开；没有本项目对应实机验收，不声明完整品牌兼容。
