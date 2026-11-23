# 错误密码重试：Windows 实测

在独立的非输入 Win32 桌面启动真实 Tauri/WebView2 程序，通过 CDP 操作并截图；没有切换系统前台。每轮使用新建的普通合成 AES 加密 ZIP、独立应用数据目录及输出目录。截图为 WebView2 截图，并非系统合成器截图。

基线的任务卡显示 `incorrect password or damaged encrypted archive`，toast 显示带 `Error:` 前缀的同一信息；候选两处均显示“密码不正确，或加密压缩包已损坏。”，保留密码错误与文件损坏两种可能。原始收据见 [before.json](before.json)、[after.json](after.json)，截图分别为 [基线失败](before-error.png)、[候选失败](after-error.png)、[重试输入](retry-dialog.png)、[完成](completed.png)。

两轮错误密码均未生成输出，归档、来源及无关哨兵保持不变。重试按钮可见，密码输入初始为空且目标位置不变；正确密码重试生成目标目录下 `sample.txt`，输出与来源 SHA-256 均为 `7760fcdb1f113918c3e6d8a88e0eba57ab936ed86d1834b4a31a1ecab0ac0cd6`。两轮分别生成加密归档，随机盐导致归档哈希不同；每轮内部均检查归档保持不变。完成后“打开输出位置”按钮可见，但未点击验证；系统前台也未观察。

基线 GUI 来自既有 roadmap-smart 便携包，SHA-256 为 `07038b863da06706b04a8fbba0c078d7040486718a4ec549248b683288450a0a`。候选 GUI 以本分支源代码 `cargo build --locked --manifest-path obz-gui/Cargo.toml --release --target-dir target` 构建，SHA-256 为 `c15214a71bf4c234c371b7670c95b7ed544530dbfb38ebae86e901ce38828bdb`，替换到同一便携包副本中进行测试；两轮 CLI 和运行时均来自该既有包，CLI SHA-256 为 `bfeb1af65841c9d0e6a7a3dacaf0868497e61e520a9a72f2c14af3537f127e49`。这不是重新打包的完整候选发行版。

复跑需要 Windows、Python、pywin32、Playwright 和带完整运行时的便携包。先确认没有其他 qing-zip 实例，然后在隔离副本上执行：

```powershell
$env:PYTHONPATH = '<包含 playwright 的目录>'
python scripts/check-password-retry-gui.py --exe '<便携包目录>/qing-zip.exe' --fixtures-in '<独立测试根>' --out '<证据输出目录>' --expect localized
```

对未经修复的基线使用 `--expect baseline`。脚本等待 WebView2 释放本轮独占的临时目录，移除后才写入成功收据。候选已复测了完整的密码重试流程和临时目录清理。[清理复测](cleanup-retry.json)
