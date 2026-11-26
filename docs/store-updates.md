# Microsoft Store 更新

每天首次启动检查一次，有网络失败时限次退避重试。查询与下载均使用 `Windows.Services.Store.StoreContext`。绿色版、模拟设备模式不调用商店更新。

发现更新后，仅当前台主页空闲时显示“发现新版本”，按钮为“立即更新”“稍后”。提醒按本地日期记录，重复启动不重复弹出；手动检查可再次显示。托盘、任务栏电量和其他页面不会触发自动弹窗，设置中的更新按钮仍可主动使用。

点击“立即更新”后重新确认可用包、等待在途设备写入结束并保存设置与历史，然后一次调用官方下载安装 API。允许静默更新时使用 `TrySilentDownloadAndInstallStorePackageUpdatesAsync`；否则使用 `RequestDownloadAndInstallStorePackageUpdatesAsync`，微软自己的授权提示由系统管理。应用不再分成下载、再次确认安装两步，也不会在用户取消后改走另一条更新路径。

安装前登记 `RegisterApplicationRestart("--updated")`，并保证当前进程运行至少60秒。系统部署关闭应用时保留 `WM_QUERYENDSESSION`/`WM_ENDSESSION` 流程。若 API 返回安装完成而当前进程仍运行，则启动本程序的独立重启助手：取得父进程句柄和 COM 激活对象、完成双向管道确认、等待父进程真正结束，再以实际 AUMID 调用 `IApplicationActivationManager::ActivateApplication`。AUMID 激活选择已注册包，避免启动旧版本路径。更新启动参数强制显示主页。

助手准备失败时保留应用运行并显示“更新已安装，重启失败”；重试同样先等待设备操作、保存设置与历史，不重复下载安装。助手使用 MTA，等待父进程最多60秒；激活最多自动重试3次，仍失败则显示“更新已安装，重新打开失败”及重试按钮，记录 HRESULT。系统关闭路径和主动助手路径互斥，主动退出前取消系统重启登记。更新期间的错误与取消分别记录，已有设备写入与历史保存失败不会被当作安装成功。

## 验证

自动测试覆盖日期缓存、主页限制、无设备主页、遮罩交互、取消与写入恢复、保存顺序、重启握手失败、退出等待和单设备更新后路由。设备回归与完整测试一并运行。

发布前必须用真实商店旧包升级到新包，覆盖静默开关、计费网络、取消、启动不足/超过60秒、前后台切换、系统关闭和完成后存活两种分支；记录旧新 PID、包版本及主页显示。绿色版和合成测试不证明 Store 部署成功。

## 官方依据

- https://learn.microsoft.com/en-us/windows/uwp/packaging/self-install-package-updates
- https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-registerapplicationrestart
- https://learn.microsoft.com/en-us/windows/win32/rstmgr/guidelines-for-applications
- https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iapplicationactivationmanager
- https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iapplicationactivationmanager-activateapplication
