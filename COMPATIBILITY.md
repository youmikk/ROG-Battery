# 设备兼容性

[English](COMPATIBILITY.en.md)

本表区分正式版的实机适配与多品牌开发分支的协议实现。枚举到设备、识别型号或通过解析测试，都不能代表该型号已验证兼容。

| 品牌 / 协议 | 型号 / 范围 | 电量 | DPI | 状态 |
| --- | --- | --- | --- | --- |
| ROG | HARPE ACE MINI，USB / OMNI | 已验证 | 已验证，保留现有深度设置 | 正式版 |
| Logitech HID++ 2.0 | 提供 0x1000 或 0x1004 功能的设备 | 协议已实现 | 提供 0x2201 功能时读取、校验后写入 | 开发分支，待实机验证 |
| 标准 HID | 描述符提供 Generic Device Controls / Battery Strength (06:20) 的设备 | 按描述符范围读取 | 此标准没有通用 DPI 写入 | 开发分支，待实机验证 |
| 雷柏 | VT1 | 待研究 | 待研究 | 尚未验证 |

HID++ 仅返回电压或粗略等级时，不会把它们作为精确百分比显示。标准 HID 不提供充电状态时，充电状态保持未知。

新增适配反馈可在设置 → 关于 → 设备适配中打开 GitHub 表单，并导出报告。报告只含出厂型号、VID/PID、Usage 和报文长度，不含序列号、MAC、接口路径、用户名、设备自定义名称或原始日志。

协议资料：[Solaar](https://github.com/pwr-Solaar/Solaar)、[USB HID Usage Tables](https://usb.org/document-library/hid-usage-tables-15)。协议逻辑独立实现，未复制驱动或开源项目代码。
