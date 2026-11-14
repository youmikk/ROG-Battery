# Keep the Store identity unchanged so existing installations can be upgraded.
@{
    IdentityName = 'Sonder.RBattery'
    Publisher = 'CN=1307ADA3-FCC0-4C75-81A2-9CC947003BD7'
    PublisherDisplayName = 'Sonder'
    DisplayName = 'R Battery'
    ShortName = 'R Battery'
    Description = 'ROG peripheral battery monitor and mouse settings utility.'
    Version = '1.4.2.0'
    Executable = 'rog-mouse-ctl.exe'
    StartupTaskId = 'RogBatteryAutoStart'

    # Optional overrides. Otherwise use tools on PATH or in the Windows SDK.
    MakeAppxPath = ''
    SignToolPath = ''
    FallbackMakeAppxPath = ''
}
