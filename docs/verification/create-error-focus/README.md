# 创建压缩包：错误字段可见性

在 Windows 11 x64、真实 150% 系统缩放的非输入私有桌面中，使用实际 Tauri/WebView2，CDP 设定真实窗口，视口为 780 × 750 CSS px、DPR 1.5。数据为临时 G: 普通文件，独立配置和工作目录；无真实用户文件。基线源码为 `42caf6479cb595dddbd1bb4869580c0cd27a82f9`，候选只在压缩表单错误出现后重新滚动当前无效焦点字段。两版使用同一套 CLI、worker 和运行时；GUI 分别为 `822d738e5efb524538c828e34ddb7fac00aa47cedc9dc04f8c22f7b8828349ce` 与 `673a378d45fa7e2b969080c74148599f77d9ab568f590ed35fff9f55e16c3db5`。

默认界面集中呈现来源、输出、格式和压缩级别；密码与分卷和高级设置折叠。展开高级设置，输入线程数 999 并提交：两版均显示相同错误并将焦点移到线程数。**基线**输入框下缘 611.83、滚动区下缘 609.83，底部裁切；**候选**下缘 603.83，完整可见。两种主题均观察到相同关系。候选输入合法值后错误消失，Tab 可继续；测试未对实际压缩结果作性能判断。

[浅色前](before/light-invalid.png) · [浅色后](after/light-invalid.png) · [深色前](before/dark-invalid.png) · [深色后](after/dark-invalid.png) · [默认界面](after/light-default.png) · [前回执](before/receipt.json) · [后回执](after/receipt.json)

两版各主题各 5 次 Playwright 点击到 DOM 错误可见的诊断耗时，中位数浅色 18.88→19.98 ms、深色 20.99→20.03 ms；这些是同机非交错的小样本，包含自动化与探测开销，不能代表端到端压缩速度或证明无性能差异。

复跑：使用隔离便携目录与 Python Playwright/pywin32，执行 `python scripts/check-create-error-focus.py --exe <qing-zip.exe> --out <新目录> --expect-visible`。脚本默认在系统临时目录创建普通样本和独立配置；为截图使用短路径时，可先建立 `G:/QingZipQA` 并追加 `--fixtures-in G:/QingZipQA`。脚本使用真实系统缩放与 WebView2，测试浅深两主题，等待 GUI 正常退出并核查临时目录清理。该检查要求当前机器真实缩放为 150%，不是 100% 或 200% 模拟。无 `--expect-visible` 可记录旧版裁切；回执的 `status=PASS` 表示采集完成，`visible` 字段才是可见性结果。

追加复跑验证了默认系统临时目录和 `--fixtures-in G:/QingZipQA` 两种夹具位置，两版 GUI 均以退出码 0 正常退出，独立临时目录均清理；[基线退出回执](before/exit-check.json) · [候选退出回执](after/exit-check.json)。截图来自同一 GUI 哈希的先前短路径运行，回执来自追加复跑。
