# Windows 11 ARM64 原生无障碍验收

Windows 11 build 26200，ARM64，应用报告实际缩放为 100%。同一进程在四个连续状态下打开主菜单。每次先读取 Windows 系统设置，再从运行中的 WebView2 读取实际媒体查询、菜单材质、可见项目与过渡时长。测试最后恢复系统原值与仅针对该程序的调试设置。[托管运行和原始产物](https://github.com/qing-yi-5427/qing-zip/actions/runs/45109159282) · [系统及窗口数据](native-settings.json) · [测试会话清理](desktop-session.json)

| 实际系统设置 | 运行中的应用 | 截图 |
| --- | --- | --- |
| 初始，开启动画 | Acrylic，菜单 7 项可见，浅色渐变 | [初始](normal.png) |
| 开启高对比度 | 强制颜色生效，菜单切为纯色，背景 RGB 32/32/32、文字白色、渐变消失，焦点轮廓清晰 | [高对比度](contrast.png) |
| 关闭高对比度并减少动画 | WebView2 报告减少动画，菜单项计算过渡为 0.00001 秒，Acrylic 恢复 | [减少动画](reduced.png) |
| 恢复初始设置 | Acrylic 和初始媒体查询恢复，7 项仍可见 | [恢复后](normal-again.png) |

截图取自实际打包的 Tauri 窗口。程序 SHA-256 为 `1a5f5bf64e507846688eacb96efa7a7c564e2431bca4d35ef595c80fb391cc3a`。本轮使用实际 Windows 系统参数广播，覆盖主菜单。Windows 10、200% 系统缩放及其它控件各需实测。试验结束时驱动终止自己启动的程序，进程终止码记录于清理回执。[Windows 系统参数 API](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow)
