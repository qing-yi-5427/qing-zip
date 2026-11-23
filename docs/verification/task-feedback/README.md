# 真实 TAR.GZ 任务反馈

在 Windows 上分别运行原版 0.12.23 便携程序和包含 #5、#6、#9 的候选便携程序；均使用隔离的 LOCALAPPDATA、TEMP 和 WebView2 profile。对同一合成 256 MiB 普通文件创建的 TAR.GZ，通过真实程序打开并解压。截图由运行中的 WebView2 捕获，未展示个人路径或数据。

| 检查 | 原版 | 候选 |
| --- | --- | --- |
| 截图前后原生任务状态 | `running` / `decompressing` | `running` / `decompressing` |
| 任务面板阶段 | 正在处理 | 解压压缩流 |
| 任务完成与输出 | 显示已完成；输出文件 SHA-256 与来源一致；输出按钮可见 | 同左 |

[原版阶段](before-decompressing.png) · [候选阶段](after-decompressing.png) · [原版完成](before-complete.png) · [候选完成](after-complete.png)。完整程序和归档 SHA-256、状态采样及输出哈希见 [原版收据](before.json) 和 [候选收据](after.json)。状态通过实际 Tauri `operation_status` 查询，采样分别位于截图之前和之后；两次均为同一阶段的运行态。

## 实际取消和打开输出位置

用同一个合成 1 GiB 普通文件创建 TAR.GZ，并运行两次候选便携程序。第一次在目标目录预放 `sample.bin` 和 `keep.txt`，选「替换已有文件」，实际点击运行中任务的「取消任务」。

| 检查 | 观察结果 |
| --- | --- |
| 点击前 | 原生状态为 `running` / `decompressing`，任务面板显示本阶段 0%。 |
| 点击后 | 任务面板依次显示「正在取消」和「已取消」。百分比清除，其间未显示剩余估算。取消任务没有输出按钮。 |
| 原包和既有目标 | 取消前后原包、`sample.bin` 和 `keep.txt` 的 SHA-256 各自一致，目录没有额外结果。 |
| 第二次成功任务 | 输出文件 SHA-256 与合成来源一致。点击「打开输出位置」后，独立读取新出现的 Explorer 窗口位置，确认对应临时目标目录。 |

[取消终态截图](cancelled.png) · [取消回执](cancelled.json) · [实际打开输出位置回执](opened-output.json)。文件总数未知但有已知字节进度的显示仍由 Node 回归覆盖。

复现前关闭其他 qing-zip 实例，并准备 Python Playwright 和完整便携目录。以下输出目录须不存在，`fixtures-in` 应为测试专用目录，需有足够空间容纳输入、压缩包和解压输出：

```powershell
python scripts/check-task-targz-gui.py --baseline <baseline>/qing-zip.exe --candidate <candidate>/qing-zip.exe --fixtures-in <test-fixture-directory> --out <new-evidence-directory>
```

分别验证取消和实际打开输出位置时，提供候选便携程序。验证 Explorer 位置另需 Python pywin32。

```powershell
python scripts/check-task-targz-gui.py --cancel --open-output --candidate <candidate>/qing-zip.exe --fixtures-in <test-fixture-directory> --out <new-evidence-directory>
```

脚本只创建并尝试清理其拥有的临时测试目录；WebView2 延迟释放句柄时目录可能暂时保留。若未能在真实任务期间捕获预期阶段，会报告失败，不以模拟状态代替。截图与证据仅说明所采集的一次前后运行，并非性能比较。
