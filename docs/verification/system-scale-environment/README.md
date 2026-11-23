# 200% 系统缩放环境只读核对

本机两台活动显示器的真实有效 DPI 均为 144（150%），无已处于 200% 的屏幕。采用 `EnumDisplayMonitors`、`GetMonitorInfoW` 和 `GetDpiForMonitor`，只让查询进程使用 per-monitor DPI awareness，没有修改显示设置。[本机非个人化收据](local-readonly.json)。VMware 服务停止，未运行或启动用户 VM；Hyper-V 功能状态查询要求提升，未请求提升或改变系统。

[托管只读运行 45160648520](https://github.com/qing-yi-5427/qing-zip/actions/runs/45160648520) 分别检查一次性 Windows 11 Enterprise ARM64 和 Windows Server 2025 x64：两者系统 Settings 标准缩放菜单均只有 100% 和 125%，可导航到自定义缩放页面，页面提供 100–500% 输入，但未填写或应用。两台的 `Runner.Listener`、`Runner.Worker` 和交互桌面均属于同一活动控制台会话；没有观察到注销后会自动恢复作业的独立控制会话，因此未注销或修改缩放。并未尝试启动额外 VM，也没有对应用使用模拟 DPI。

| 托管目标 | 系统与会话 | 原生设置回执 |
| --- | --- | --- |
| Windows 11 ARM64 | [主机](arm-host.json)、[自定义页和会话](arm-session.json) | [标准缩放选项](arm-settings.json) |
| Windows Server 2025 x64 | [主机](x64-host.json)、[自定义页和会话](x64-session.json) | [标准缩放选项](x64-settings.json) |

[此前 #32 ARM64 测试](https://github.com/qing-yi-5427/qing-zip/pull/32) 在多种可用分辨率下同样没有 200% 预设，结果仍为 BLOCKED，不是通过。微软说明自定义缩放范围是 100–500%，并提示此设置可能影响可读性；其 DPI 文档也指出缩放后的有效垂直分辨率限制可能约束可选预设。[微软无障碍显示设置](https://support.microsoft.com/en-us/accessibility/windows/make-windows-easier-to-see) · [微软 DPI 文档](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/dpi-related-apis-and-registry-settings?view=windows-11)

要完成真实 200% 验收，还需现成、可恢复且控制通道不依赖待注销桌面会话的隔离 Windows 环境；在该环境通过 Windows Settings 设置后，重新登录并同时记录系统设置、窗口 `GetDpiForWindow` 和 WebView2 DPR。当前未观察到满足条件的会话，本次没有 200% 实测。

复跑只读环境探测：在 `Windows ARM64 native validation` 工作流手动设置 `hosted_scale_probe=true`。此入口只导航设置页面和读取选项/会话信息，不应用缩放、不注销；Windows Server 2025 结果不标为 Windows 11。
