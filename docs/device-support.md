# ROG 外设 PID 型号表

> 数据来源：ASUS Gear Link 的 `hid-whitelist.json` 与每个 PID 的 `manifest.json`
> （`caps.deviceName` / `caps.deviceNameZH`），抓取日期 2026-10-21，白名单 102 个 PID。
>
> 为什么需要这张表：接收器的扫描应答只给 PID + controlReportId + notifyReportId，
> 型号名必须自己映射。固件自报的产品名不可控（可能夹厂商缩写、英文、甚至空串），
> 所以「表里有的 PID 以表为准，表里没有才退回自报名，最后才用 PID 兜底」。
>
> 命名规律：同一型号通常占两个相邻 PID（有线直连 / 2.4G 经接收器），
> Spatha X 65K 与 Harpe II Ace-PBZ 有三个。

共收录 **75 个 PID**、5 个接收器。

## 接收器

这些 PID 自身没有电池，只是转发通道。

| PID | 十进制 | 名称 |
| --- | --- | --- |
| `0x1ACE` | 6862 | ROG OMNI 无线接收器 |
| `0x1ACF` | 6863 | ROG OMNI 无线接收器 |
| `0x1AD0` | 6864 | ROG SpeedNova 8K 无线接收器 |
| `0x1AD1` | 6865 | ROG SpeedNova 8K 无线接收器 |
| `0x1D54` | 7508 | 华硕无线接收器 |

## 鼠标

| PID | 十进制 | 英文型号 | 中文型号 |
| --- | --- | --- | --- |
| `0x1A92` | 6802 | ROG HARPE ACE AIM LAB EDITION | ROG 龙鳞 Ace Aim Lab 合作版 |
| `0x1A94` | 6804 | ROG HARPE ACE AIM LAB EDITION | ROG 龙鳞 Ace Aim Lab 合作版 |
| `0x1B16` | 6934 | ROG KERIS II ACE | ROG 月刃2 ACE |
| `0x1B18` | 6936 | ROG KERIS II ACE | ROG 月刃2 ACE |
| `0x1B63` | 7011 | ROG HARPE ACE MINI | ROG 龙鳞 Ace Mini |
| `0x1B65` | 7013 | ROG HARPE ACE MINI | ROG 龙鳞 Ace Mini |
| `0x1B67` | 7015 | ROG HARPE ACE EXTREME | ROG 龙鳞 Ace Extreme |
| `0x1B69` | 7017 | ROG HARPE ACE EXTREME | ROG 龙鳞 Ace Extreme |
| `0x1C0C` | 7180 | ROG KERIS II ORIGIN | ROG 月刃2 RGB |
| `0x1C0E` | 7182 | ROG KERIS II ORIGIN | ROG 月刃2 RGB |
| `0x1C69` | 7273 | ROG HARPE II ACE | ROG 龙鳞 2 Ace |
| `0x1C6B` | 7275 | ROG HARPE II ACE | ROG 龙鳞 2 Ace |
| `0x1CD3` | 7379 | PROART MOUSE MD301 | PROART MOUSE MD301 |
| `0x1CD5` | 7381 | PROART MOUSE MD301 | PROART MOUSE MD301 |
| `0x1D4C` | 7500 | ROG KERIS II ORIGIN-KJP | ROG 月刃 2 RGB - KJP |
| `0x1D4E` | 7502 | ROG KERIS II ORIGIN-KJP | ROG 月刃 2 RGB - KJP |
| `0x1D7A` | 7546 | ROG HARPE II EXTREME EDITION 20 | ROG 龙鳞 2 Extreme 20周年版 |
| `0x1D7B` | 7547 | ROG HARPE II EXTREME EDITION 20 | ROG 龙鳞 2 Extreme 20周年版 |
| `0x1D7C` | 7548 | ROG HARPE II EXTREME EDITION 20 | ROG 龙鳞 2 Extreme 20周年版 |
| `0x1DB0` | 7600 | ROG GLADIUS IV ACE | ROG 战刃 4 ACE |
| `0x1DBF` | 7615 | ROG GLADIUS IV ACE | ROG 战刃 4 ACE |
| `0x1DDE` | 7646 | ROG SPATHA X 65K | ROG 斯巴达 X 65K |
| `0x1DDF` | 7647 | ROG SPATHA X 65K | ROG 斯巴达 X 65K |
| `0x1DE0` | 7648 | ROG SPATHA X 65K | ROG 斯巴达 X 65K |
| `0x1DE1` | 7649 | ROG SPATHA X 65K | ROG 斯巴达 X 65K |
| `0x1E2F` | 7727 | ROG HARPE II ACE MINI | ROG 龙鳞 2 Ace Mini |
| `0x1E32` | 7730 | ROG HARPE II ACE MINI | ROG 龙鳞 2 Ace Mini |
| `0x1E4A` | 7754 | ROG HARPE II ACE-PBZ | ROG 龙鳞2 ACE - PBZ |
| `0x1E4B` | 7755 | ROG HARPE II ACE-PBZ | ROG 龙鳞2 ACE - PBZ |
| `0x1E4D` | 7757 | ROG HARPE II ACE-PBZ | ROG 龙鳞2 ACE - PBZ |
| `0x1E4E` | 7758 | ROG GLADIUS IV ACE MAX | ROG 战刃 4 ACE MAX |
| `0x1E4F` | 7759 | ROG GLADIUS IV ACE MAX | ROG 战刃 4 ACE MAX |
| `0x1E51` | 7761 | ROG HARPE II ACE MINI DEMON1 EDITION | ROG 龙鳞 2 Ace Mini Demon1联名版 |
| `0x1E52` | 7762 | ROG HARPE II ACE MINI DEMON1 EDITION | ROG 龙鳞 2 Ace Mini Demon1联名版 |

## 键盘（同属 12 01 电量协议）

| PID | 十进制 | 英文型号 | 中文型号 |
| --- | --- | --- | --- |
| `0x1A83` | 6787 | ROG AZOTH | ROG 夜魔 |
| `0x1A85` | 6789 | ROG AZOTH | ROG 夜魔 |
| `0x1AAE` | 6830 | ROG STRIX SCOPE II 96 WIRELESS | ROG 游侠2 98无线版 |
| `0x1AAF` | 6831 | ROG STRIX SCOPE II 96 WIRELESS | ROG 游侠2 98无线版 |
| `0x1AB0` | 6832 | ROG STRIX SCOPE II 96 WIRELESS | ROG 游侠2 98无线版 |
| `0x1B3F` | 6975 | ROG AZOTH EXTREME | ROG 夜魔 EXTREME |
| `0x1B40` | 6976 | ROG AZOTH EXTREME | ROG 夜魔 EXTREME |
| `0x1B42` | 6978 | ROG AZOTH EXTREME | ROG 夜魔 EXTREME |
| `0x1C10` | 7184 | ROG AZOTH 96 HE / LITE | ROG 夜魔 98 HE / LITE |
| `0x1C11` | 7185 | ROG AZOTH 96 HE / LITE | ROG 夜魔 98 HE / LITE |
| `0x1C12` | 7186 | ROG AZOTH 96 HE / LITE | ROG 夜魔 98 HE / LITE |
| `0x1C24` | 7204 | ROG AZOTH X | ROG 夜魔 X |
| `0x1C25` | 7205 | ROG AZOTH X | ROG 夜魔 X |
| `0x1C26` | 7206 | ROG AZOTH X | ROG 夜魔 X |
| `0x1C2F` | 7215 | ROG FALCATA | ROG 影魔 |
| `0x1C30` | 7216 | ROG FALCATA | ROG 影魔 |
| `0x1C31` | 7217 | ROG FALCATA | ROG 影魔 |
| `0x1CE8` | 7400 | ROG STRIX MORPH 96 WIRELESS | ROG 术士 98 |
| `0x1CE9` | 7401 | ROG STRIX MORPH 96 WIRELESS | ROG 术士 98 |
| `0x1CEA` | 7402 | ROG STRIX MORPH 96 WIRELESS | ROG 术士 98 |
| `0x1CEF` | 7407 | ROG AZOTH EXTREME SPECIAL EDITION | ROG 夜魔 EXTREME SPECIAL EDITION |
| `0x1CF0` | 7408 | ROG AZOTH EXTREME SPECIAL EDITION | ROG 夜魔 EXTREME SPECIAL EDITION |
| `0x1CF1` | 7409 | ROG AZOTH EXTREME SPECIAL EDITION | ROG 夜魔 EXTREME SPECIAL EDITION |
| `0x1D45` | 7493 | PROART KEYBOARD KD300 | PROART KEYBOARD KD300 |
| `0x1D46` | 7494 | PROART KEYBOARD KD300 | PROART KEYBOARD KD300 |
| `0x1D47` | 7495 | PROART KEYBOARD KD300 | PROART KEYBOARD KD300 |
| `0x1D68` | 7528 | ASUS TX75 ANALOG GAMING KEYBOARD | 华硕天选TX75 |
| `0x1D6A` | 7530 | ASUS TX75 ANALOG GAMING KEYBOARD | 华硕天选TX75 |
| `0x1DA2` | 7586 | ROG Azoth Extreme Edition 20 | ROG 夜魔 Extreme 20 周年版 |
| `0x1DA3` | 7587 | ROG Azoth Extreme Edition 20 | ROG 夜魔 Extreme 20 周年版 |
| `0x1DA4` | 7588 | ROG Azoth Extreme Edition 20 | ROG 夜魔 Extreme 20 周年版 |
| `0x1E0A` | 7690 | ROG STRIX MORPH 96 X WIRELESS | ROG 术士 98 X |
| `0x1E0B` | 7691 | ROG STRIX MORPH 96 X WIRELESS | ROG 术士 98 X |
| `0x1E0C` | 7692 | ROG STRIX MORPH 96 X WIRELESS | ROG 术士 98 X |
| `0x1E5B` | 7771 | ROG STRIX MORPH 96 WIRELESS-PBZ | ROG 术士 98 - PBZ |
| `0x1E5C` | 7772 | ROG STRIX MORPH 96 WIRELESS-PBZ | ROG 术士 98 - PBZ |
| `0x1E5D` | 7773 | ROG STRIX MORPH 96 WIRELESS-PBZ | ROG 术士 98 - PBZ |

## 手柄

| PID | 十进制 | 英文型号 | 中文型号 |
| --- | --- | --- | --- |
| `0x1C64` | 7268 | ROG RAIKIRI II PRO PC | ROG 雷切 II PRO PC |
| `0x1C65` | 7269 | ROG RAIKIRI II PRO PC | ROG 雷切 II PRO PC |
| `0x1C91` | 7313 | ROG RAIKIRI II XBOX WIRELESS | ROG 雷切 II XBOX无线手柄 |
| `0x1C92` | 7314 | ROG RAIKIRI II XBOX WIRELESS | ROG 雷切 II XBOX无线手柄 |
