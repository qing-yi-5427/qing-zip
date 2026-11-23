# 三层 ZIP 写回冲突验收

在 Windows 上从 R3 加固便携包启动真实 Tauri/WebView2。脚本独占一个非输入 Win32 桌面和独立应用资料目录，CDP 只监听环回端口并使用有限进程存活检查。启动前检查 qing-zip 单实例锁，结束后确认自己创建的归档、窗口和配置目录已释放。

`check-nested-multi-layer.py` 使用三个普通 ZIP 和一个磁盘哨兵。双击打开 `outer.zip › middle.zip › inner.zip`，在右键菜单对叶层执行重命名。Python 对外包以同一文件 ID 写入不同、完整合法的 ZIP，并同步。真实界面的「保存至外层」使叶层写入私有中层，后续写回原外包因原成员改变而失败。脚本分别核对三个层级的相邻成员、外包实际哈希、私有中层中编辑后的内包字节和各自会话的 `dirty` 状态；R3 分支明确丢弃后关闭并冷启动，确认新进程及外包仍保持独立写入的版本。

需要适用于 Windows x64 的 Python、`pywin32` 和 `playwright`。浏览器通过运行中的 WebView2 提供，因此无需另下载 Playwright 浏览器。使用本轮 `fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6` 的 [托管构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128040411) 产物 `qing-zip-windows-x64-trial`。每次写到新的证据输出目录。脚本核对运行的 GUI、CLI 与 worker 同托管加固清单的 SHA-256。

```powershell
python check-nested-multi-layer.py `
  --exe '<解包目录>/qing-zip.exe' `
  --hardening '<产物目录>/target/hardened/x86_64-pc-windows-msvc/release/hardened-build.json' `
  --package '<产物目录>/dist/x64/qing-zip-0.12.23-portable.zip' `
  --out '<新证据目录>' `
  --source fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6
```

比较重点是写回失败后唯一标为 `dirty` 的工作副本为中层，内层编辑已在该中层字节内。该基线用例不涉及系统会话终止、异常杀进程、特殊文件系统或真实磁盘空间不足。

对于包含修复 `03ee431` 的候选包，追加 `--expect-feedback`。它要求真实任务卡提示外层被修改，要求中层标签显示未保存标记，原内层标签移除该标记。

`measure-nested-feedback.py` 运行等量旧版和候选，奇偶轮反转顺序，每次重新建立同样的三层 ZIP 并复核哈希。`--baseline` 和 `--candidate` 各放入 GUI 路径、加固 JSON 路径、便携 ZIP 路径和源码完整 SHA，默认五对。点击开始点取自 WebView 捕获阶段的 `performance.now()`，观察点为对应失败文本所属任务卡首先在 DOM 中可见。

采样包含真实后台写回和 UI 更新，排除进程启动、外部写入及 Playwright 检查等待。DOM 可见不同于 Windows 合成器绘制，低负载的普通三层小 ZIP 不能代表大包。旧版即时未标出正确中层时，记录缺失而非延迟。逐轮时间与汇总在 `timing.json`，各轮目录另有保全收据和截图。
