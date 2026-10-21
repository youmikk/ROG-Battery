# MSIX 打包用的可变参数。build-msix.ps1 读这个文件填 AppxManifest.xml.template。
#
# 为什么单独一个 psd1 而不是塞进脚本：这 8 个值每次发布/每次换身份都要动，
# 而脚本逻辑不该跟着动。改这里不会有改坏逻辑的风险。
#
# 【最需要注意的是 Publisher】
# 它必须和签名证书 Subject 逐字一致（大小写、空格、逗号后空格都算）。
# 现在填的是占位用的自签名身份；**提交 Microsoft Store 前必须换成
# Partner Center → 产品标识（Product identity）页给出的原值**，
# 形如 CN=1A2B3C4D-5E6F-...。填错了提交会被拒。

@{
    # ---------- 身份 ----------
    # Store 上的唯一包名。一旦提交就不能改，所以用反向域名风格最稳。
    # 这里用 github 用户名占位，等你确定网名再改（改早不改晚，提交后锁死）。
    IdentityName = 'Neonrain.RBattery'

    # 见上面的大注释。自签测试时用 CN= 加你想显示的名字；
    # 提交 Store 时换成 Partner Center 给的那串。
    Publisher    = 'CN=Neonrain'

    # 商店页面「由 XXX 发布」显示的名字。个人开发者建议用自己的网名/GitHub 用户名。
    PublisherDisplayName = 'Neonrain'

    # ---------- 展示 ----------
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
