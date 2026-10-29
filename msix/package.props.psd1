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
    #
    # 本包**沿用** rog-battery 的 identity，即商城上现有那个包的同一身份 ——
    # 用途是让 rog-mouse-ctl 的 MSIX 覆盖升级现有商店应用。

    # Package/Identity/Name
    IdentityName = 'Sonder.RBattery'

    # Package/Identity/Publisher
    # 必须是 CN=<GUID> 这个形式，逐字复制。它同时是上架证书的 Subject。
    Publisher    = 'CN=1307ADA3-FCC0-4C75-81A2-9CC947003BD7'

    # Package/Properties/PublisherDisplayName
    PublisherDisplayName = 'Sonder'

    # ---------- 展示 ----------
    # DisplayName / ShortName 是用户在开始菜单、设置、任务管理器里看到的名称，
    # 和上面的身份字段无关，可以随时改（改完重打包即可，Store 允许更新）。
    DisplayName  = 'R Battery'
    ShortName    = 'R Battery'
    Description  = 'ASUS ROG mouse HID settings tool: DPI, polling rate, lighting, debounce and backup, no ASUS driver needed.'

    # ---------- 版本 ----------
    # 4 段数字，第一段必须 >= 1。Store 只接受**严格大于**已发布版本的号。
    #
    # 为什么是 1.1.0.0 而不是 1.0.1.0：
    #   - 商店里已上架的是 C# 版 1.0.0.0，同 identity 同版本无法覆盖升级，
    #     本地 Add-AppxPackage 也会直接拒绝。
    #   - Rust 重写是功能大改（灯光/按键/性能全参数可写 + 历史曲线），
    #     按语义化版本属于 minor 升级；1.0.1.0 会让人误以为只是修 bug。
    # build-msix.ps1 会拿本机已装版本做前置比对，忘了升版本会被当场拦住。
    #
    # 2026-10-29：商店侧已经提交过 1.1.9.0（CI 测试跑上去的），所以这里必须
    # 大于它，顺手对齐成 1.2.0.0 —— 与 Cargo.toml / tag 的主次号一致。
    Version      = '1.2.0.0'

    # ---------- 可执行文件 ----------
    # 相对包根的路径。.exe 由 build-msix.ps1 从 ..\target\release\ 拷进 stage 根目录，
    # 所以这里是裸文件名。改二进制名时这里和 Cargo.toml 的 [[bin]] name 要一起改。
    Executable   = 'rog-mouse-ctl.exe'

    # ---------- 开机自启 ----------
    # 必须和程序侧的 StartupTaskId 常量一致（程序侧定为 RogBatteryAutoStart）。
    # 改这里 = 程序调 StartupTask.GetAsync 找不到任务，静默失效。
    StartupTaskId = 'RogBatteryAutoStart'

    # ---------- 本机工具路径（可选） ----------
    # 这台机器没装 Windows SDK，makeappx/signtool 是从 NuGet 的
    # Microsoft.Windows.SDK.BuildTools 包里解出来的。装了 SDK 的话留空即可，
    # 脚本会先找 PATH。
    #   D:\codex\_tools\makeappx 这个目录不能只挑 exe —— makeappx 靠同目录的
    #   .manifest 做免注册 COM 绑定，只拷 exe 会报「side-by-side configuration
    #   is incorrect」。整个目录留着最稳。
    MakeAppxPath = 'D:\codex\_tools\makeappx\makeappx.exe'
    SignToolPath = 'D:\codex\_tools\makeappx\signtool.exe'

    # makeappx 找不到时的第二备选（SDK BuildTools 解压目录）
    FallbackMakeAppxPath = 'D:\codex\_tools\sdk-buildtools\extracted\bin\10.0.28000.0\x64\makeappx.exe'
}
