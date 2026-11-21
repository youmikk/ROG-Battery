# R Battery 隐私政策 / Privacy Policy

最后更新 / Last updated: 2026-11-11

## 中文

R Battery 是用于读取外设电量并调整支持的鼠标参数的 Windows 本地应用。应用不联网，不上传或共享设备信息，不包含遥测、广告或崩溃上报服务。

应用通过 Windows HID 接口读取型号、连接状态、电量、固件及设备提供的序列号等信息，并在您操作时修改支持的设备设置。设备标识用于本地识别设备和保存偏好，不会发送到服务器。

以下数据仅保存在您的电脑上：

- 设置、语言、主题、托盘选择、设备名称和类型偏好。默认设置位于 `%APPDATA%\rog-mouse-ctl\settings.ini`。
- 电量历史，用于绘制曲线和估算续航，默认位于 `%LOCALAPPDATA%\rog-battery\history.tsv`。
- 设备状态缓存，默认位于 `%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache`。
- 故障诊断与托盘日志，可能包含错误文本、设备状态或本地路径，不会自动上传。
- 由您主动保存的设备配置备份，以及自定义存储位置的路径记录。

启用开机自启时，绿色版使用当前用户的启动项，商店版使用 Windows StartupTask。您可以在设置中关闭，不需要管理员权限。

您可以随时删除本地数据。商店版数据由 Windows 管理；绿色版数据、自定义目录和导出的备份可能需要手动删除。若主动向开发者分享日志或备份，请先检查并移除序列号、个人路径等内容。

本政策更新时会修改上方日期。有关隐私的问题可通过 [GitHub issues](https://github.com/youmikk/ROG-Battery/issues) 联系。

## English

R Battery is a local Windows application for monitoring peripheral batteries and changing supported mouse settings. It makes no network connections, uploads or shares no device information, and includes no telemetry, advertising, or crash-reporting service.

Through Windows HID interfaces, the app reads information such as model, connection status, battery level, firmware, and device-provided serial numbers. It changes supported settings when you request an operation. Device identifiers are used locally to distinguish devices and store preferences; they are never sent to a server.

The following data stays on your computer:

- Settings, language, theme, tray selection, device names, and device type preferences. The default settings file is `%APPDATA%\rog-mouse-ctl\settings.ini`.
- Battery history for charts and runtime estimates, at `%LOCALAPPDATA%\rog-battery\history.tsv` by default.
- A device snapshot cache, at `%LOCALAPPDATA%\rog-mouse-ctl\snapshot.cache` by default.
- Diagnostic and tray logs, which may contain error messages, device status, or local paths. They are not uploaded automatically.
- Device configuration backups you explicitly save, and records of custom storage locations.

When enabled, startup uses a per-user startup entry in the portable build or Windows StartupTask in the Store build. You can disable it in settings without administrator rights.

You can delete local data at any time. Windows manages Store app data. Portable data, custom directories, and exported backups may require manual removal. Before voluntarily sharing a log or backup, remove serial numbers, personal paths, and other information you do not wish to disclose.

The date above changes when this policy is updated. For privacy questions, use [GitHub issues](https://github.com/youmikk/ROG-Battery/issues).
