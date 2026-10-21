# MSIX 打包用的可变参数。build-msix.ps1 读这个文件填 AppxManifest.xml.template。
#
# 为什么单独一个 psd1 而不是塞进脚本：这些值每次发布/每次换身份都要动，
# 而脚本逻辑不该跟着动。改这里不会有改坏逻辑的风险。
#
# 【最需要注意的是 Publisher】
# 它必须和签名证书 Subject 逐字一致（大小写、空格、逗号后空格都算）。
# 现在填的是 Partner Center 给的正式值，**不要自己改写格式**。
# 换产品/换账号时整段重新复制一遍。

@{
    # ---------- 身份 ----------
    # 下面三项**直接对应 Partner Center → 产品标识**页的原值，不要自己改写格式。
    # 一旦提交就不能改，所以现在就是最终值。

    # Package/Identity/Name
    IdentityName = 'Sonder.RBattery'

    # Package/Identity/Publisher
    # 必须是 CN=<GUID> 这个形式，逐字复制。它同时是自签测试证书的 Subject，
    # 所以 -Sign 打出来的包 Subject 会和提交值一致，本地装包不会因为身份不符被拒。
    Publisher    = 'CN=1307ADA3-FCC0-4C75-81A2-9CC947003BD7'

    # Package/Properties/PublisherDisplayName
    PublisherDisplayName = 'Sonder'

    # ---------- 展示 ----------
    # DisplayName / ShortName 是用户在开始菜单、设置、任务管理器里看到的名称，
    # 和上面的身份字段无关，可以随时改（改完重打包即可，Store 允许更新）。
    DisplayName  = 'R Battery'
    ShortName    = 'R Battery'
    Description  = 'Shows battery level, voltage and DPI for ASUS ROG wireless mice in the system tray.'

    # ---------- 版本 ----------
    # 4 段数字，第一段必须 >= 1。Store 只接受比已发布版本大的号。
    Version      = '1.0.0.0'

    # ---------- 开机自启 ----------
    # 必须和 tray.cs 里的 StartupTaskId 常量一致。
    # 用反向域名风格，避免和别的包撞（TaskId 在包内唯一，但写规范点没坏处）。
    StartupTaskId = 'RogBatteryAutoStart'

    # ---------- 本机工具路径（可选） ----------
    # 这台机器没装 Windows SDK，makeappx/signtool 是从 NuGet 的
    # Microsoft.Windows.SDK.BuildTools 包里解出来的。装了 SDK 的话留空即可，
    # 脚本会先找 PATH。
    #   D:\codex\_tools\makeappx 这个目录不能只挑 exe —— makeappx 靠同目录的
    #   .manifest 做免注册 COM 绑定，只拷 exe 会报「side-by-side configuration
    #   is incorrect」。整个目录 50 个文件 / 18 MB 留着最稳。
    MakeAppxPath = 'D:\codex\_tools\makeappx\makeappx.exe'
    SignToolPath = 'D:\codex\_tools\makeappx\signtool.exe'
}
