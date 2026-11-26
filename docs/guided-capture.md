# 本地分步骤采集 / Local guided capture

用于排查已发现设备的电量变化：设置 → 反馈设备 → 查看诊断 → 本地分步骤采集。选择设备后开始，正常使用、接上充电线、拔掉充电线、关闭外设、重新开启外设，每完成一步点对应按钮；可跳过或重复。每步等待一次正常电量刷新。停止后查看最近记录，保存原始包和摘要，再打开保存文件夹检查完整文件。

This records changes for an already discovered device: Settings → Device feedback → View diagnostics → Local guided capture. Start, then mark normal use, connecting/disconnecting charging and powering the peripheral off/on. Steps can be skipped or repeated. Wait for an ordinary battery refresh between steps. Stop, review, save, then open the folder to inspect the complete files.

- 只旁路记录本进程在所选厂商接口上的既有 `HidDevice` / `FeatureChannel` 传输，不打开第二个输入通道，不增加查询、改变轮询或运行未知命令。标准键鼠输入接口、蓝牙系统属性和 XInput 不提供这个入口。未识别设备仍先使用普通反馈；这不是全设备抓包工具。
- Records existing `HidDevice` / `FeatureChannel` transfers on one selected vendor interface. It opens no extra handles, changes no polling interval and sends no additional commands. Standard input collections, Bluetooth OS properties and XInput are excluded. Unrecognized devices should use ordinary feedback first; this is not a universal capture tool.
- 一个接收器接口可能包含多个外设槽位，采集范围为整个接口。记录完成时间、未归一化字节、API、成功状态和用户步骤；不保证覆盖本程序之外的驱动流量、其他接口或后端直接调用的同步 HID API。充电导致接口切换时，不自动跟踪新接口，避免关联到错误设备。
- One receiver interface can carry several slots. Captures cover that interface, with completion times, unnormalized bytes, API results and user markers. They do not capture other applications, other interfaces or backend-specific synchronous HID calls. Switching interfaces while charging does not automatically follow a different path.
- 5 分钟、1024 条、512 KiB JSONL 记录，任一上限达到后停止。限制包括十六进制与 JSON 开销；导出头尾和摘要有额外的有界开销。返回或关闭窗口停止采集，清除记录或新建反馈释放内存。日常未启用采集时，仅检查原子标记，不保存报文。
- Stops after 5 minutes, 1024 records or 512 KiB of JSONL records, including hex/JSON overhead. Headers, footer and summary add bounded overhead. Going back or closing stops collection; clear/new feedback releases memory. With capture disabled, the hook only checks an atomic flag.
- 保存到用户可写的缓存目录 `captures/capture-…/raw.jsonl` 和 `summary.txt`。原始包不写入序列号或接口路径元数据，但原始字节仍可能包含身份或输入信息，不承诺匿名。分享前检查完整文件。两个文件均不自动附加到 GitHub 反馈，普通反馈仍走原有白名单。
- Saves under the writable user cache directory in `captures/capture-…/raw.jsonl` and `summary.txt`. Raw bytes can contain identities or input even though metadata excludes serials and paths. Review before sharing. Neither file is automatically attached to GitHub; normal feedback retains its allowlist.

设计参考 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723` 的分步骤标记；实现独立，采用现有通道旁路记录以避免新增读取竞争。本项目测试使用合成数据验证隔离、停止、容量、导出和反馈边界，不代表设备真机采集验收。

Inspired by VigilWatt's guided markers at the pinned commit above. Independently implemented as observation of existing channels to avoid additional read contention. Synthetic tests verify isolation, lifecycle, bounds, export and feedback separation; they do not constitute hardware validation.
