# 第二轮 ARM64 Fluent 原生窗口复核

[一次性 Windows 11 ARM64 托管运行 45121970580](https://github.com/qing-yi-5427/qing-zip/actions/runs/45121970580) 直接使用 [原生构建 run 45119972417](https://github.com/qing-yi-5427/qing-zip/actions/runs/45119972417) 的现成便携包，没有重新编译。源码 `b2bf9f3f4eaddcbfefd3144ad0fdb8769fa1824b`，包 SHA-256 `39b494dcbff1b3afa20b4c85bcb0ddb4de5da017fcee3c056593daab038fc10e`；artifact 及包校验记录见 [package.json](package.json)。

沿用 #13 的实际 Tauri/WebView2 菜单和原生系统设置驱动，在托管桌面切换并核验 [普通](normal.png)、[高对比](contrast.png)、[减少动画](reduced.png) 及 [恢复普通](normal-again.png)。高对比实际报告强制颜色、纯色背景；减少动画实际报告 reduced；全部菜单可见且各有七项。[native-settings.json](native-settings.json) 包含各状态测量、截图哈希及系统设置恢复结果，[desktop-session.json](desktop-session.json) 记录调试覆盖恢复。结论限本次 ARM64 托管环境与所验菜单，并不代表其他分辨率或硬件均已验证。

原始完整 artifact `round2-arm64-fluent-evidence` 位于上述运行；其保留期为 30 天。本目录保存同一证据供长期审阅。复跑入口位于此验证分支的 windows-arm64 workflow，只可显式调度并固定该包及构建，不启用深度特殊文件系统测试。
