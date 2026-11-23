# 普通批量解压在强退后的续跑基线

Windows 11 x64 26300，普通本地 NTFS，真实 Tauri/WebView2，私有非输入 Win32 桌面及独立配置。分支从主线 26be67b065fec26fc740b980260eee59befd5f55 开始；该提交与本次实跑的 6e29fad4cfaca19975bae7f93343dae5f5fabee3 加固便携包之间仅有验收文档及性能回执变化。每个回执带有便携包、GUI、CLI 和 worker 的 SHA-256。[官方仓库托管构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45253326474)

| 实际操作 | 强退时的工作与输出 | 按原配置重开 |
| --- | --- | --- |
| 四个 ZIP，替换同名文件，两轮在第二项运行中强退 | 第一项已发布。第二项原生状态均为 decoding，回执观察到分别完成 64 和 1 个成员，进程退出后该目录分别有 79 和 2 个文件，均含一份预先生成的旁文件。第三项没有输出，第四项既有同名文件字节一致。 | 两轮任务页都无记录。输出在重开及随后正常退出期间逐文件匹配。 |
| 五个 ZIP，原生第二项停止后正常退出，再打开并主动继续 | 正常退出以「成功 1 · 已取消 1 · 未开始 3」恢复。继续进入原 3/5 时，旧待续记录已领取并消耗。此时强退，第三项原生状态为 decoding，目标目录保留 202 个成员。第四项原有同名文件保留，第五项未输出。 | 再次重开显示无任务记录。尚未处理的第四、第五项没有续跑入口。 |

三次均只结束脚本持有的测试 GUI 进程句柄，退出码为 137。每次核对全部来源和旁文件 SHA-256、第一项已发布文件标识与修改时间，并比对重开前后的所有输出。目录清单摘要以完整的相对文件名与各文件 SHA-256 计算。[启动后截图](running-before-exit.png) · [重开任务页](reopened.png) · [新批次一](fresh-first.json) · [新批次二](fresh-second.json) · [继续后的强退](resumed.json)

## 现有保存及启动成本

独立第三组普通三项 ZIP，原有中间同名冲突。七次热 CAS 保存（含本地写入、文件同步、替换及 WebView IPC）中位 5.2 ms，范围 5.1–5.5 ms。随后通过实际批量窗体开始，三个原生 `start_operation` 响应约 4.6、1.4 和 1.0 ms，首尾生命周期调用为 5.3 和 2.1 ms。验证实际终态、冲突文件及输出哈希。此测量是现有实现的端点诊断，后续方案的额外成本需要配对实测。[原始耗时](timings.json)

## 复跑

Windows，Python 3.12+，pywin32 和 Playwright。程序目录需要完整的便携包内容，夹具父目录需要已存在，输出目录需为新目录。脚本只在独占的非输入桌面启动和结束实验进程；发现已有应用实例时会拒绝启动。选择所测精确便携包及其构建收据，再运行：

```powershell
$common = @('scripts/check-batch-crash-desktop.py', '--source', '26be67b (product bits 6e29fad)',
  '--exe', 'C:/QingZipQA/portable/qing-zip.exe', '--package', 'C:/QingZipQA/portable.zip',
  '--hardening', 'C:/QingZipQA/hardened-build.json', '--fixtures-in', 'C:/QingZipQA/fixtures')
python @common --out C:/QingZipQA/evidence/fresh
python @common --scenario resumed --out C:/QingZipQA/evidence/resumed
python @common --scenario timing --out C:/QingZipQA/evidence/timing
```

这些结果只覆盖应用进程强退及普通文件，不推断机器断电、其他文件系统或孤儿工作进程的生命周期。现有正常停止、覆盖再确认和竞争条件由第七轮脚本核对。[正常退出验收](../../round7/batch-resume/README.md)
