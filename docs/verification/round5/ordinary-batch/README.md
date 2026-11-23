# 普通批量解压的结果和继续操作

本机 Windows 11 x64 26300，真实 Tauri/WebView2 窗口，独立 Win32 桌面、配置目录与普通临时文件。对照为已安装的 0.12.23 主线产品，候选为从 `11b5365` 加固构建的 GUI，两者的 CLI、worker 逐字节相同。两份回执含各二进制与样本的 SHA-256。[原版回执](baseline/receipt.json) · [候选回执](candidate/receipt.json)

| 用户操作 | 原版 | 候选 |
| --- | --- | --- |
| 按顺序解压三个 ZIP，第二项遇到已有同名文件 | 最后一项显示「已完成」，整批进度停在 2/3，完整的冲突原因需进入任务记录查看。[原版小窗](baseline/mixed-result.png) | 直接显示「成功 2 · 失败 1」。展开后可看见冲突的归档和目标文件；任务中心分别显示第一和第三项 100%、第二项失败。[批量结果](candidate/mixed-result.png) · [任务中心](candidate/mixed-history.png) |
| 在第二个 ZIP 正在解压时点击停止 | 按钮文字为「取消任务」，随后第三项也停止。单独重填第三项时预计目标变为 `1-third`。[原版取消](baseline/stop-result.png) | 按钮写明「停止批量处理」。停止后可区分已成功、已取消和未开始的各一项。[停止后的实际窗口](candidate/stop-result.png) |
| 停止后点击「继续未开始的项目」 | 缺少该入口 | 原编号 `3-third` 收到第三项；第一项的文件身份、修改时间和长度均相同；第二项没有再次执行。结果为「成功 2 · 已取消 1」。[继续后的实际窗口](candidate/stop-resumed.png) |

切换成「检查完整性」，提交两个有效 ZIP 和一个缺失路径。真实弹窗显示两项成功、一项失败，「查看批量结果」返回批量页并可展开缺失项。两个界面都未显示解压专用的输出目录按钮。原包和旁文件内容保持一致。[完整性检查回执](candidate/integrity.json)

在应用中切换深色，将原生窗口的客户区缩为 1170 × 1125 物理像素。系统实测 144 DPI，WebView2 为 780 × 750 CSS 像素。两条 197 至 198 字符的路径都能换行并用滚轮读全，Tab 键可依次到达「继续未开始的项目」和「打开输出目录」；点击继续后第三项的输出哈希正确。[原生全窗截图](candidate/dark-narrow-150.png) · [窗口尺寸、键盘和文件回执](candidate/dark-narrow-150.json)

四个独立临时目录均在操作后完成清理。逐项校验确认三个来源归档、冲突文件和旁文件的内容保全，输出文件与对应来源内容相符。批量页提供「打开输出目录」，本机上述回执没有启动 Explorer。继续入口保留到开始新批次或退出程序。重新启动程序后找回普通批次尚未验收。

另取一对预热和五对交替运行。三个小 ZIP 从点击到显示终态，原版和候选中位分别为 78.3 和 83.0 ms，配对差中位 +5.7 ms，范围 −5.2 至 +7.5 ms。各次均检查文件结果。[逐次读数](timing.json) · [性能报告](../../../PERFORMANCE_REPORT.md)

### 复跑

本机运行环境为 Python 3.14.0、Playwright 及 pywin32。从仓库根目录执行下列命令。程序目录需同时包含同版本的 GUI、CLI、worker 和运行时，证据目录须为新目录。

```powershell
python scripts/check-batch-extract-desktop.py --mode candidate --case both --exe C:/qa/candidate/qing-zip.exe --out C:/qa/evidence/candidate
python docs/verification/round5/ordinary-batch/measure.py --baseline C:/qa/base/qing-zip.exe --candidate C:/qa/candidate/qing-zip.exe --out C:/qa/evidence/timing.json
```

将模式换为 `baseline` 可在修复前的版本记录原有行为。`--public-scratch-root` 接受专门用于合成数据的父目录，便于在真实截图中保留可读的测试路径。
