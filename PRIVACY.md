# 隐私说明

R Battery（应用包标识 `Sonder.RBattery`）不收集、不存储、不上传任何个人信息。

- **不联网**：应用不包含任何网络请求代码，不访问任何服务器。
- **不收集**：没有遥测、没有崩溃上报、没有分析 SDK、没有账号体系。
- **只在本机读写**：
  - 设置：`%APPDATA%\rog-mouse-ctl\settings.ini`
  - 电量历史：`%LOCALAPPDATA%\rog-battery\history.tsv`
  - 从 Microsoft Store 安装时，上述路径由系统重定向到
    `%LOCALAPPDATA%\Packages\Sonder.RBattery_*\LocalCache\` 下，卸载时随包清理。
- **设备访问**：应用通过 USB HID 与本地鼠标通信，读取电量与参数、写入用户主动修改的
  设置。通信不经过网络，也不读取鼠标以外的设备。
- **不需要管理员权限**：以当前用户身份运行。

卸载应用即删除上述本机数据。

---

# Privacy

R Battery (package identity `Sonder.RBattery`) does not collect, store, or transmit
any personal information.

- **No network**: the app contains no networking code and contacts no server.
- **No collection**: no telemetry, no crash reporting, no analytics SDK, no accounts.
- **Local files only**:
  - settings: `%APPDATA%\rog-mouse-ctl\settings.ini`
  - battery history: `%LOCALAPPDATA%\rog-battery\history.tsv`
  - When installed from the Microsoft Store these paths are redirected by Windows to
    `%LOCALAPPDATA%\Packages\Sonder.RBattery_*\LocalCache\` and are removed on uninstall.
- **Device access**: the app talks to the local mouse over USB HID to read battery
  level and settings and to write back the changes the user makes. That traffic never
  leaves the machine, and no device other than the mouse is accessed.
- **No administrator rights required**: it runs as the signed-in user.

Uninstalling the app removes the local data described above.
