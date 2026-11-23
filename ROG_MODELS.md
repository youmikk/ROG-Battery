# ASUS / ROG 型号目录

**简体中文** | [English](ROG_MODELS.en.md)

[返回项目介绍](README.md) · [多品牌兼容性说明](COMPATIBILITY.md)

本页保留 ASUS / ROG 的内置识别目录，其他品牌及逐型号功能范围见兼容性说明。目录收录表示程序能够匹配对应型号，不代表所有功能都已在该硬件上验证。

已验证 ROG HARPE ACE MINI 的 USB 直连和 ROG OMNI 接收器连接。接收器本身没有电池，只用于转发已配对设备的信息。

## 鼠标（15 款）

| 型号 | 英文型号 | PID |
| --- | --- | --- |
| ROG 龙鳞 Ace Aim Lab 合作版 | ROG HARPE ACE AIM LAB EDITION | `0x1A92` / `0x1A94` |
| ROG 月刃2 ACE | ROG KERIS II ACE | `0x1B16` / `0x1B18` |
| **ROG 龙鳞 Ace Mini**（已验证） | ROG HARPE ACE MINI | `0x1B63` / `0x1B65` |
| ROG 龙鳞 Ace Extreme | ROG HARPE ACE EXTREME | `0x1B67` / `0x1B69` |
| ROG 月刃2 RGB | ROG KERIS II ORIGIN | `0x1C0C` / `0x1C0E` |
| ROG 龙鳞 2 Ace | ROG HARPE II ACE | `0x1C69` / `0x1C6B` |
| PROART MOUSE MD301 | PROART MOUSE MD301 | `0x1CD3` / `0x1CD5` |
| ROG 月刃 2 RGB - KJP | ROG KERIS II ORIGIN-KJP | `0x1D4C` / `0x1D4E` |
| ROG 龙鳞 2 Extreme 20 周年版 | ROG HARPE II EXTREME EDITION 20 | `0x1D7A` / `0x1D7B` / `0x1D7C` |
| ROG 战刃 4 ACE | ROG GLADIUS IV ACE | `0x1DB0` / `0x1DBF` |
| ROG 斯巴达 X 65K | ROG SPATHA X 65K | `0x1DDE` / `0x1DDF` / `0x1DE0` / `0x1DE1` |
| ROG 龙鳞 2 Ace Mini | ROG HARPE II ACE MINI | `0x1E2F` / `0x1E32` |
| ROG 龙鳞 2 ACE - PBZ | ROG HARPE II ACE-PBZ | `0x1E4A` / `0x1E4B` / `0x1E4D` |
| ROG 战刃 4 ACE MAX | ROG GLADIUS IV ACE MAX | `0x1E4E` / `0x1E4F` |
| ROG 龙鳞 2 Ace Mini Demon1 联名版 | ROG HARPE II ACE MINI DEMON1 EDITION | `0x1E51` / `0x1E52` |

## 键盘（13 款）

键盘与鼠标共用同一套电量协议（`12 01`），可读取电量与基本信息；DPI、回报率等鼠标专属项不适用于键盘。

| 型号 | 英文型号 | PID |
| --- | --- | --- |
| ROG 夜魔 | ROG AZOTH | `0x1A83` / `0x1A85` |
| ROG 游侠 2 98 无线版 | ROG STRIX SCOPE II 96 WIRELESS | `0x1AAE` / `0x1AAF` / `0x1AB0` |
| ROG 夜魔 EXTREME | ROG AZOTH EXTREME | `0x1B3F` / `0x1B40` / `0x1B42` |
| ROG 夜魔 98 HE / LITE | ROG AZOTH 96 HE / LITE | `0x1C10` / `0x1C11` / `0x1C12` |
| ROG 夜魔 X | ROG AZOTH X | `0x1C24` / `0x1C25` / `0x1C26` |
| ROG 影魔 | ROG FALCATA | `0x1C2F` / `0x1C30` / `0x1C31` |
| ROG 术士 98 | ROG STRIX MORPH 96 WIRELESS | `0x1CE8` / `0x1CE9` / `0x1CEA` |
| ROG 夜魔 EXTREME SPECIAL EDITION | ROG AZOTH EXTREME SPECIAL EDITION | `0x1CEF` / `0x1CF0` / `0x1CF1` |
| PROART KEYBOARD KD300 | PROART KEYBOARD KD300 | `0x1D45` / `0x1D46` / `0x1D47` |
| 华硕天选 TX75 | ASUS TX75 ANALOG GAMING KEYBOARD | `0x1D68` / `0x1D6A` |
| ROG 夜魔 Extreme 20 周年版 | ROG Azoth Extreme Edition 20 | `0x1DA2` / `0x1DA3` / `0x1DA4` |
| ROG 术士 98 X | ROG STRIX MORPH 96 X WIRELESS | `0x1E0A` / `0x1E0B` / `0x1E0C` |
| ROG 术士 98 - PBZ | ROG STRIX MORPH 96 WIRELESS-PBZ | `0x1E5B` / `0x1E5C` / `0x1E5D` |

## 手柄（2 款）

| 型号 | 英文型号 | PID |
| --- | --- | --- |
| ROG 雷切 II PRO PC | ROG RAIKIRI II PRO PC | `0x1C64` / `0x1C65` |
| ROG 雷切 II XBOX 无线手柄 | ROG RAIKIRI II XBOX WIRELESS | `0x1C91` / `0x1C92` |

## 无线接收器（3 类，5 个 PID）

接收器自身没有电池，只作为转发通道。同一型号常有两种形态，协议相同。

| 接收器 | PID |
| --- | --- |
| ROG OMNI 无线接收器 | `0x1ACE` / `0x1ACF` |
| ROG SpeedNova 8K 无线接收器 | `0x1AD0` / `0x1AD1` |
| 华硕无线接收器 | `0x1D54` |
