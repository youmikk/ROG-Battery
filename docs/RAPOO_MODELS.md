# 雷柏 nRF54L 型号表 / Rapoo nRF54L models

本表为 1.4.4 的协议覆盖清单，尚未逐型号实机验证。仅当设备提供 FF00:0002 控制集合与有效的 0x07 状态报告时读取电量及当前 DPI，不写入设备设置。型号属于官方 A HUB 的 VT_nrf54L 驱动，其他主控不能套用。

This is a protocol coverage list for version 1.4.4, pending per-model hardware validation. Reads require the FF00:0002 control collection and a valid 0x07 status report. Battery and current DPI are read-only. Models are assigned to VT_nrf54L by the official A HUB driver; other controllers require their own protocols.

VID: `24AE` · 77 model entries · 146 product IDs

| 型号 / Model | PID |
| --- | --- |
| ESM612ACE | `0x464d` |
| ESM612MAX | `0x4681` |
| ESM612PRO | `0x4631` |
| ESM622MAX | `0x4637` |
| ESM622PRO | `0x4632` |
| ESM652PRO | `0x4633` |
| ESM672MAX | `0x4640` |
| ESM672PRO | `0x4635` |
| VT0 | `0x4610 / 0x1410` |
| VT0 Air MAX | `0x46a1 / 0x14a1` |
| VT0 MAX | `0x4620 / 0x1420` |
| VT0 MAX MASTER | `0x464a / 0x144a` |
| VT1 | `0x4615 / 0x1415` |
| VT1 MAX | `0x4644 / 0x1444` |
| VT1S MAX V2 | `0x4b0a / 0x1b0a` |
| VT1S V2 | `0x466a / 0x146a` |
| VT2 | `0x4622 / 0x1422` |
| VT2 Air MAX V2 | `0x4b12 / 0x1b12` |
| VT2 MAX | `0x4623 / 0x1423` |
| VT2 MAX MASTER V2 | `0x4675 / 0x1475` |
| VT2 MAX V2 | `0x4b09 / 0x1b09` |
| VT2 V2 | `0x4668 / 0x1468` |
| VT2K MAX | `0x46a4 / 0x14a4` |
| VT2S Air MAX V2 | `0x4b13 / 0x1b13` |
| VT2S MAX MASTER V2 | `0x4676 / 0x1476` |
| VT2S MAX V2 | `0x4b04 / 0x1b04` |
| VT2S V2 | `0x465d / 0x145d` |
| VT3 | `0x4612 / 0x1412` |
| VT3 Air MAX V2 | `0x4b0c / 0x1b0c` |
| VT3 MAX | `0x4617 / 0x1417` |
| VT3 MAX MASTER V2 | `0x466d / 0x146d` |
| VT3 MAX V2 | `0x4b06 / 0x1b06` |
| VT3 PRO V2 | `0x46a5 / 0x14a5` |
| VT3 V2 | `0x4662 / 0x1462` |
| VT3S | `0x4611 / 0x1411` |
| VT3S Air Max | `0x46a2 / 0x14a2` |
| VT3S Air MAX V2 | `0x4b0d / 0x1b0d` |
| VT3S MAX | `0x4621 / 0x1421` |
| VT3S MAX MASTER | `0x464b / 0x144b` |
| VT3S MAX MASTER V2 | `0x466e / 0x146e` |
| VT3S MAX V2 | `0x4b07 / 0x1b07` |
| VT3S PRO V2 | `0x46a6 / 0x14a6` |
| VT3S V2 | `0x4664 / 0x1464` |
| VT4 Air MAX V2 | `0x4b14 / 0x1b14` |
| VT4 MAX MASTER V2 | `0x4677 / 0x1477` |
| VT4 MAX V2 | `0x4b03 / 0x1b03` |
| VT4 V2 | `0x465b / 0x145b` |
| VT6 | `0x4651 / 0x1451` |
| VT6 MAX | `0x4652 / 0x1452` |
| VT7 | `0x4613 / 0x1413` |
| VT7 Air MAX V2 | `0x4b0e / 0x1b0e` |
| VT7 MAX | `0x4616 / 0x1416` |
| VT7 MAX MASTER V2 | `0x4670 / 0x1470` |
| VT7 MAX V2 | `0x4b05 / 0x1b05` |
| VT7 PRO V2 | `0x46a7 / 0x14a7` |
| VT7 V2 | `0x4660 / 0x1460` |
| VT7S Air MAX V2 | `0x4b0f / 0x1b0f` |
| VT7S MAX MASTER V2 | `0x4671 / 0x1471` |
| VT7S MAX V2 | `0x4b02 / 0x1b02` |
| VT7S PRO V2 | `0x46a8 / 0x14a8` |
| VT7S V2 | `0x4659 / 0x1459` |
| VT8 Air MAX V2 | `0x4b15 / 0x1b15` |
| VT8 MAX MASTER V2 | `0x4678 / 0x1478` |
| VT8 MAX V2 | `0x4b0b / 0x1b0b` |
| VT8 V2 | `0x466c / 0x146c` |
| VT9 | `0x4614 / 0x1414` |
| VT9 Air MAX V2 | `0x4b10 / 0x1b10` |
| VT9 MAX | `0x4618 / 0x1418` |
| VT9 MAX MASTER V2 | `0x4673 / 0x1473` |
| VT9 MAX V2 | `0x4b08 / 0x1b08` |
| VT9 PRO V2 | `0x46a9 / 0x14a9` |
| VT9 V2 | `0x4666 / 0x1466` |
| VT9S Air MAX V2 | `0x4b11 / 0x1b11` |
| VT9S MAX MASTER V2 | `0x4674 / 0x1474` |
| VT9S MAX V2 | `0x4b01 / 0x1b01` |
| VT9S PRO V2 | `0x46aa / 0x14aa` |
| VT9S V2 | `0x4653 / 0x1453` |

资料 / Reference: [Rapoo A HUB](https://hub.rapoo.cn/) · [设备兼容性](COMPATIBILITY.md) · [Device compatibility](COMPATIBILITY.en.md)
