# 第三轮 ARM64 便携包的实际菜单

一次性 Windows 11 ARM64 build 26200、实际 100% 系统缩放。在交付的同一份便携包上打开菜单，切换系统高对比度与减少动画，测量 WebView2 的强制颜色和动画偏好，再恢复运行前状态。四次均显示 7 个菜单项。高对比度时应用采用实色菜单、黑字白底。减少动画时 WebView 报告 reduced。

[普通](normal.png) · [高对比度](contrast.png) · [减少动画](reduced.png) · [系统设置恢复](normal-again.png) · [测量与截图 SHA-256](native-settings.json) · [启动测试设置恢复](desktop-session.json)

源码 `fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6`，便携包 SHA-256 为 `cc0e06dafef5d4caf7f8905ee87c5396f8e3f022c03c2a6f382dada4321f3bbe`。[源构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128040730) · [原生窗口运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45129763865) · [包与构建的对应记录](package.json)

可在本验证分支调度 windows-arm64.yml，选中历史菜单测试开关 round2_fluent_capture。留空其余捕获参数会复跑固定 R2 样本。对另一份同仓库可信构建，填入 capture_build_run、capture_source、capture_package_sha 后复用相同流程。历史默认入口也已在更新后复跑。[历史样本复跑](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128537216)
