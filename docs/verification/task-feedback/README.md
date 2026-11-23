# 真实 TAR.GZ 任务反馈

在 Windows 上分别运行原版 0.12.23 便携程序和包含 #5、#6、#9 的候选便携程序；均使用隔离的 LOCALAPPDATA、TEMP 和 WebView2 profile。对同一合成 256 MiB 普通文件创建的 TAR.GZ，通过真实程序打开并解压。截图由运行中的 WebView2 捕获，未展示个人路径或数据。

| 检查 | 原版 | 候选 |
| --- | --- | --- |
| 截图前后原生任务状态 | `running` / `decompressing` | `running` / `decompressing` |
| 任务面板阶段 | 正在处理 | 解压压缩流 |
| 任务完成与输出 | 显示已完成；输出文件 SHA-256 与来源一致；输出按钮可见 | 同左 |

[原版阶段](before-decompressing.png) · [候选阶段](after-decompressing.png) · [原版完成](before-complete.png) · [候选完成](after-complete.png)。完整程序和归档 SHA-256、状态采样及输出哈希见 [原版收据](before.json) 和 [候选收据](after.json)。状态通过实际 Tauri `operation_status` 查询，采样分别位于截图之前和之后；两次均为同一阶段的运行态。

本次没有实际点击“打开输出位置”，没有观察物理取消过程，也没有观察到只有字节总数而文件总数未知的阶段；这些场景不因本截图标记为真实 GUI 验收通过。候选为合并构建，本实测不声称其余改动来自单独的 #6 二进制。关于取消及文件计数的回归仍是 Node 模拟快照。

复现前关闭其他 qing-zip 实例，并准备 Python Playwright 和完整便携目录。以下输出目录须不存在，`fixtures-in` 应为测试专用目录，需有足够空间容纳输入、压缩包和解压输出：

```powershell
python scripts/check-task-targz-gui.py --baseline <baseline>/qing-zip.exe --candidate <candidate>/qing-zip.exe --fixtures-in <test-fixture-directory> --out <new-evidence-directory>
```

脚本只创建并尝试清理其拥有的临时测试目录；WebView2 延迟释放句柄时目录可能暂时保留。若未能在真实任务期间捕获预期阶段，会报告失败，不以模拟状态代替。截图与证据仅说明所采集的一次前后运行，并非性能比较。
