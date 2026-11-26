# 适配证据索引 / Adaptation evidence

本表覆盖近期跟进的协议，不代表全项目型号清单。完整兼容范围见 [兼容性](COMPATIBILITY.md) / [compatibility](COMPATIBILITY.en.md)，固定来源及边界见 [协议研究](PROTOCOLS.md) / [protocol evidence](PROTOCOLS.en.md)。实现、合成测试、上游硬件记录和本项目真机验收是不同证据。

This index covers recently followed protocols, not the complete device catalog. Implementation, synthetic tests, upstream hardware records and local hardware validation are separate evidence levels.

VigilWatt 来源固定为 `c97adc389b60b0e945a62cddacbaaace39887723`；HyperX 的交叉来源与固定提交见协议研究。固件版本未明确记录时，不能推断所有固件兼容。许可见 [第三方声明](THIRD_PARTY_NOTICES.md)。

| 型号与模式 / Device and mode | 状态 / Status | 上游硬件证据 / Upstream hardware | 本项目验证 / Local validation | 未确认 / Outstanding |
| --- | --- | --- | --- | --- |
| 高斯 GANSS GS3104T-Pro，接收器 `05AC:024F` | 已实现，未发布 / Implemented, unreleased | VigilWatt 97% 完整捕获，Linux 提交说明中的精确产品串 / Full 97% capture and independently recorded receiver identity | 捕获回放、合成损坏/边界、同型号隔离 / Replay, synthetic corruption/bounds and instance isolation | 本项目真机、完整描述符、1%、休眠、其他固件及驱动共存 / Local hardware, descriptors, 1%, sleep, firmware, coexistence |
| HyperX Cloud Alpha Wireless，接收器 `03F0:098D` | 已实现，未发布 / Implemented, unreleased | 多个 Windows 开源实现的接口、容量和命令证据 / Windows implementations establish interface, capacities and commands | 合成回复、回显、旧型号及多设备回归 / Synthetic replies, echoes, existing models and multi-device regressions | 本项目真机、逐报告描述符、0%、充电冻结、关机缓存及驱动共存 / Local hardware, per-report descriptor, zero, charging freeze, off-state caching, coexistence |
| 狼途 LANGTU T98，接收器 `1A2C:8FFF` | 已实现，未发布 / Implemented, unreleased | 单台 Windows 接口观测、78/87/FF 前缀及休眠观察 / One-unit Windows interface, prefixes and sleep observations | 前缀重建、合成解析及生命周期回归 / Reconstructed prefixes, synthetic parser and lifecycle regressions | 本项目真机、描述符、其他固件、0%、同型号双实例及驱动共存 / Local hardware, descriptor, firmware, zero, dual instances, coexistence |
| CHERRY PIXIU75，USB `046A:01E2` / 2.4G `046A:01E4` | 已实现，未发布 / Implemented, unreleased | Windows 接口与官方软件对照 / Windows interface and official-app comparison | 前轮 10 项解析、通道及旧 MX Pro 回归；前缀重建 / 10 earlier regressions, reconstructed prefixes | 本项目真机、0%、长休眠、多实例、充电及跨模式身份 / Local hardware, zero, sleep, instances, charging, cross-mode identity |
| 雷神 ML903 NearLink Se，接收器 `369B:0234` | 已实现，未发布 / Implemented, unreleased | 35% 前缀；27%、关机静默及恢复观察 / 35% prefix, 27%, power-cycle observations | 14 项实帧前缀回放与合成回归 / 14 prefix replay and synthetic tests | 实际描述符、不同固件、充电、双实例及官方驱动共存 / Descriptor, firmware, charging, dual instances, coexistence |
| AKG N9 Hybrid，接收器 `0ECB:20D9` | 已实现，未发布 / Implemented, unreleased | 完整 62 字节 50% 帧及六轮自然轮询 100% / Full 50% frame, six natural 100% polls | 19 项实帧、分片、ACK、旧缓存及实例回归 / 19 frame, fragmentation, ACK, stale-tail and instance tests | 本项目真机、持续放电、关机、官方软件并发；无事务号 / Local hardware, discharge, power-off, coexistence; no transaction ID |
| 北通鲲鹏20，`20BC:5159/5158` | 暂缓，未启用 / Deferred, disabled | 无线 39% 前缀、有线 78% 前缀 / Wireless 39% and wired 78% prefixes | 独立研究测试，不编入产品 / Isolated research tests, no production module | 输入集合边界、关机后缓存广播、XInput 槽位与物理实例对应 / Input isolation, off-state broadcasts, XInput slot-to-instance association |

## 本地采集的使用边界 / Local capture scope

新增的 [分步骤采集](docs/guided-capture.md) 旁路记录已发现设备的既有通信，可帮助用户按正常使用、充电、关机、恢复提供带时间标记的证据；不主动探测未适配设备。有数据时默认附带到用户提交的反馈，提交前可关闭，公开议题提供下载链接。采集成功不等于协议已经解释正确。

[Guided capture](docs/guided-capture.md) observes existing communication for a discovered device and adds action markers. It does not probe unsupported devices. Nonempty recordings are attached by default when the user submits feedback; this can be disabled before submission. Public issues provide a download link. Capturing a frame does not establish its meaning.

## 后续准入条件 / Acceptance criteria

1. 固定上游版本与具体源码，分别记录连接模式、接口指纹及已验证字段，不仅依据 VID/PID 列表。Pin source and record mode, interface fingerprint and verified fields; a PID catalog is insufficient.
2. 每个实帧明确来源；补零、改写、假通道均标为合成测试。Identify recorded data; padding, mutations and fake transports remain synthetic.
3. 对异形帧、未知字段、静默和断连分别测试，不能把失败显示为 0%，不能用旧采样刷新时间。Test malformed data, unknown fields, silence and disconnect separately; never turn failure into zero or refresh stale samples.
4. 同型号双设备、跨品牌、插拔、休眠和官方软件共存需实机验证。Hardware checks must include identical instances, mixed brands, reconnect, sleep and official software.
5. 北通需先补无线集合描述符、完整开关机序列及可靠 XInput 关联；不按枚举顺序、型号或电量猜测槽位，不屏蔽无关手柄。Betop requires collection evidence, lifecycle captures and reliable slot linkage; do not infer mappings from order/model/battery or hide unrelated controllers.
