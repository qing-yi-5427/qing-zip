# 托管 Windows 11 的 200% 系统缩放复核

GitHub 托管 Windows 11 ARM64 build 26200 的标准显示缩放列表只有 **100%（推荐）和 125%**。实际尝试更大的两种受支持分辨率后，Windows 设置仍未提供 200%。真 200% 缩放验收记为 `BLOCKED_200_UNAVAILABLE`。严格门禁因此失败，证据上传成功。 [托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45131852234)

| 步骤 | 活动分辨率 | Windows 设置的启用缩放预设 |
| --- | --- | --- |
| 初始 | 1024 × 768（推荐） | 100%、125% |
| 临时模式 | 1600 × 1200 | 100%、125% |
| 临时模式 | 1920 × 1080 | 100%、125% |
| 恢复 | 1024 × 768（推荐） | 100%、125% |

显示驱动为 Microsoft Hyper-V Video 10.0.26100.1150。通过普通 Windows Settings 的 Scale 下拉框和 UI Automation 读取预设，并用 `ChangeDisplaySettingsW` 的临时模式验证两种分辨率。Windows Settings 确认实际活动分辨率已切换并恢复。未尝试自定义缩放、注销或 WebView 设备模拟。 [原生设置与应用回执](system-scale.json) · [微软显示设置](https://support.microsoft.com/en-gb/windows/hardware/display-graphics/change-your-screen-resolution-and-layout-in-windows) · [微软临时显示模式 API](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-changedisplaysettingsw)

实际打包程序主窗口的 Win32 DPI 和 WebView2 DPR 在试验前与恢复后均为 96 和 1。菜单均显示 7 项，菜单边界在 1028 × 779 CSS px 的应用视口内。前后两张应用截图 SHA-256 相同。托管 Settings 窗口的独立截图为空白，已排除，Windows 的选项以 UI Automation 回执为准。 [应用前后相同的截图](menu-before.png) · [测试会话恢复回执](desktop-session.json)

被测便携包源码为 `fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6`，ARM64 便携 ZIP 的 SHA-256 为 `cc0e06dafef5d4caf7f8905ee87c5396f8e3f022c03c2a6f382dada4321f3bbe`。 [包和运行绑定](package.json) · [原始便携包构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128040730)

在此验收分支复跑时，调度 `windows-arm64.yml`，启用 `round2_fluent_capture` 和 `capture_system_scale`，传入这里记录的构建编号、源码 SHA 及包 SHA-256。完整的托管证据包还保留每次展开的系统选项和分别保存的应用前后截图。
