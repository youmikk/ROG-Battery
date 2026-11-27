# 本地分步骤采集 / Local guided capture

用于排查已发现设备的电量变化：设置 → 反馈设备 → “附带设备诊断”下方的“设备通信采集”。选择设备后开始，正常使用、接上充电线、拔掉充电线、关闭外设、重新开启外设，每完成一步点对应按钮；可跳过或重复。每步等待一次正常电量刷新。停止后查看最近记录，返回反馈页面提交；有数据时默认附带采集文件，也可关闭附带或另存到本机。

This records changes for an already discovered device: Settings → Device feedback → Device traffic capture, below Include device diagnostics. Start, then mark normal use, connecting/disconnecting charging and powering the peripheral off/on. Steps can be skipped or repeated. Wait for an ordinary battery refresh between steps. Stop, review and return to the form. Captures containing data are included by default; they can be excluded or saved locally.

采集到报文后再停止。只有操作标记或空超时时，表单显示“本次未采集到通信数据”，点击“查看结果”后可“重新采集”；不会出现上传开关。停止后显示实际采集时长，不再继续计时。

Wait for captured traffic before stopping. Marker-only or empty-timeout captures show a no-data result on the form; open View result to try again. They have no upload toggle. The elapsed time stops when collection stops.

- 旁路记录本进程在所选厂商接口上的既有通信，覆盖 `HidDevice`、`FeatureChannel`、共用控制报告查询，以及雷柏 legacy / office 原生传输。开始或重新采集时请求一次正常后台刷新，后续按原轮询间隔读取。标准键鼠输入接口、蓝牙系统属性和 XInput 不提供这个入口；未识别设备可通过普通反馈提交接口信息。
- Records existing communication on one selected vendor interface, including `HidDevice`, `FeatureChannel`, shared control-report queries and native Rapoo legacy/office transports. Starting or retrying capture requests one ordinary background refresh; subsequent reads use the configured polling interval. Standard input collections, Bluetooth OS properties and XInput have no capture entry; ordinary feedback can include interface information for unrecognized devices.
- 一个接收器接口可能包含多个外设槽位，采集范围为整个接口。记录完成时间、未归一化字节、API、成功状态和用户步骤。采集仅覆盖本进程已接入记录的传输，不包含其他应用或其他接口的流量。充电导致接口切换时，需选择新接口重新采集。
- One receiver interface can carry several slots. Captures cover that interface, with completion times, unnormalized bytes, API results and user markers. Only instrumented transfers in this process are recorded; other applications and interfaces are outside the capture. If charging changes the interface, select the new interface and start another capture.
- 非零等待的 Input 超时记录为 `input-timeout`；零等待排空检查不算超时事件。Nonzero-wait Input timeouts are recorded; empty queue-drain checks are excluded.
- 5 分钟、1024 条、512 KiB JSONL 记录，任一上限达到后停止。限制包括十六进制与 JSON 开销；导出头尾和摘要有额外的有界开销。返回或关闭窗口停止采集，清除记录或新建反馈释放内存。日常未启用采集时，仅检查原子标记，不保存报文。
- Stops after 5 minutes, 1024 records or 512 KiB of JSONL records, including hex/JSON overhead. Headers, footer and summary add bounded overhead. Going back or closing stops collection; clear/new feedback releases memory. With capture disabled, the hook only checks an atomic flag.
- 点击“提交反馈”时，已勾选的 `raw.jsonl` 经现有免登录中继上传至附件存储，公开的 GitHub 议题包含下载链接。原始字节可能包含设备标识或输入信息，提交前可查看、关闭附带或清除记录。只有步骤标记或空超时记录时不附带文件。普通诊断摘要继续使用原有白名单，采集文件单独验证，最大 544 KiB（含头尾）。
- Submitting feedback uploads the included `raw.jsonl` through the existing anonymous relay to attachment storage and adds a download link to the public GitHub issue. Bytes may contain identifiers or input information. Review, exclude or clear the capture before submitting. Marker-only or empty-timeout captures are omitted. Normal diagnostic summaries retain their allowlist; captures have separate validation and a 544 KiB whole-file limit.
- 另存到本机时写入用户缓存目录 `captures/capture-…/raw.jsonl` 和 `summary.txt`。重试使用同一份采集附件，待提交草稿加密保存在本机；切换设备、新建反馈或提交成功会释放当前采集。采集动作本身不会立即发送反馈。
- Local copies are saved under `captures/capture-…/raw.jsonl` and `summary.txt` in the user cache. Pending retries reuse the same capture, stored in encrypted local draft history. Changing devices, starting a new report or completing submission clears the current capture. Collection itself does not submit feedback.
- 保存期间可关闭窗口，页面切换需等待保存结果，避免丢失错误或成功位置。保存失败清除本次创建的未完成文件；如果无法清除，会明确提示残留路径。Export completion is retained across closing; navigation waits for the result. Failed exports remove only newly created files and report the residual location if cleanup fails.

设计参考 VigilWatt `c97adc389b60b0e945a62cddacbaaace39887723` 的分步骤标记；实现独立，采用现有通道旁路记录以避免新增读取竞争。本项目测试使用合成数据验证隔离、停止、容量、导出、可选附件和重试恢复，不代表设备真机采集验收。

Inspired by VigilWatt's guided markers at the pinned commit above. Independently implemented as observation of existing channels to avoid additional read contention. Synthetic tests verify isolation, lifecycle, bounds, exports, optional feedback attachments and retry recovery; they do not constitute hardware validation.
