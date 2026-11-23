# 第九轮批次恢复与高对比实测

## 普通批次异常退出

Windows 11 x64 上使用 #63 正式候选 `ddfdfc11db6b8e7dd5caa2113642cf56ecb59284` 的加固便携包，在真实窗口与普通临时 ZIP 上完成九案。源归档、输出和无关文件按各案回执核对；此测试使用独立配置和临时数据。

| 场景 | 回执 |
| --- | --- |
| 新批次、续跑批次、最后一项强退 | [新批次](crash-fresh.json) · [续跑](crash-resumed.json) · [最后一项](crash-last.json) |
| 人工截断尾记录 | [回执](crash-partial.json) |
| 正常停止并继续、替换确认 | [继续](normal-continue.json) · [确认](normal-overwrite-review.json) |
| 记录不可写、损坏及未来格式、关窗交接 | [不可写](normal-write-failure.json) · [记录恢复](normal-record-recovery.json) · [关窗](normal-exit-while-saving.json) |

[结果汇总](summary.json) · [重开窗口](batch-reopened.png) · [来源校验](checked-source.json)。人工截断和受控 IPC 延迟分别是模拟数据及测试门控；进程强退不等于断电、文件系统故障或介质故障。当前仅一份可恢复记录，并非多历史批次恢复。

候选来自 [Windows x64 run 45273082510](https://github.com/qing-yi-5427/qing-zip/actions/runs/45273082510)：PR 检出的合并提交 `ad3f405fb6e111ccdaab9bfea5697c24a3b3c19f` 与候选头具有相同源码树 `a846703561df9680a6d7579528005954e0461bf4`。便携 ZIP SHA-256 为 `e296303aa045fed8425025399809c454aa97b982c4c3204e810661d528824976`，GUI 为 `6b0db6dd75b30d913c0f80be6eb867e65538e4ab51ea2569e02835ff1a2d5908`。这组验证针对 #63 固定候选，不冒称验证了之后的主线包。

复跑时先验证该 run 的便携包及加固构建记录，准备包含 `portable.zip`、`hardened-build.json`、`checked-source.json` 和 `staged/qing-zip-0.12.23-portable/qing-zip.exe` 的目录；检出固定候选源码并配置其 Windows GUI 测试依赖。提供空的独占临时目录和新输出目录：

```powershell
./docs/verification/round9/final-acceptance/run-ddfdfc1-gui-qa.ps1 -Repo <固定源码目录> -Stage <校验后的包目录> -FixtureRoot <临时目录> -Out <新输出目录>
```

脚本复用固定源码的 [强退驱动](https://github.com/qing-yi-5427/qing-zip/blob/ddfdfc11db6b8e7dd5caa2113642cf56ecb59284/scripts/check-batch-crash-desktop.py) 和 [正常续跑驱动](https://github.com/qing-yi-5427/qing-zip/blob/ddfdfc11db6b8e7dd5caa2113642cf56ecb59284/scripts/check-batch-resume-desktop.py)。同会话替换确认的基线和两种修复后场景见[独立记录](../same-session-overwrite/README.md)。

## 四档实际操作耗时

[五对逐次记录](batch-four-workloads.json)包括预热、实际点击至结果、逐项哈希及系统信息；报告中的四档中位数和配对差均从该文件计算。相比固定的 #64 基线，#63 候选在全部成功的 8 项样本五对均较慢、24 个小归档样本四对较慢；其他两档差异跨过零。缓存未清除，场景大小、系统负载和样本数限制外推。

[复跑脚本](measure-reviewed-batch-windows.py)复用固定源码目录的 GUI 驱动。公开版本只将原先写死的 CPU 和内存记录改为可选参数，不改变计时及判断逻辑。指定已验证的基线和候选便携 GUI、独占临时目录以及新输出文件：

```powershell
python docs/verification/round9/final-acceptance/measure-reviewed-batch-windows.py --scripts <固定源码目录>/scripts --baseline <基线GUI> --candidate <候选GUI> --fixtures-in <临时目录> --out <新JSON> --baseline-source 5cab593 --candidate-source ddfdfc1 --cpu-label "<CPU>" --ram-gib <内存GiB> --pairs 5 --case all
```

## 原生高对比与启动资源

同一台一次性 Windows 11 ARM64 系统、真实 100% 缩放下，在系统高对比主题对比 [修复前](contrast-before.png) 和 [修复后](contrast-after.png)。基线 GUI SHA-256 为 `55359dd380047c221868f872cace3b410ade6369ad2cd1385a4200e6e9461e39`；[候选包记录](contrast-after-package.json)绑定 #61 精确候选。实际截图中文字核心区域颜色为 RGB (38,59,80)，背景从 (32,32,32) 变为 (142,227,240)，按 sRGB 计算对比约 1.42:1 → 7.89:1；这不是校色后显示器测量。[基线设置回执](contrast-before-settings.json) · [候选设置回执](contrast-after-settings.json) · [候选运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45264847524)。

同一托管机另做每版一次预热和交替五对启动：[原始数据](contrast-launch-paired.json) · [包来源](contrast-launch-packages.json) · [运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45266730690)。时间从启动进程到首页 DOM 可见并经两帧，包含调试连接和轮询；私有内存为创建表单打开后进程树的值。缓存未清，未判定前台焦点，也未测物理屏幕首帧或整机内存。测量脚本及依赖保存在[固定源码](https://github.com/qing-yi-5427/qing-zip/tree/89fc9b48d50e9dfb7eb9ca248e05bd883fd4c4cb/scripts)；其会话收尾经原运行回执核对。

[十二次会话收尾记录](contrast-launch-sessions.json)中的 `passed: false` 来自驱动将正常的 `SystemExit(0)` 记为异常，不能忽略这一脚手架缺陷；各次均另记有进程退出码 0 和调试设置已恢复。启动数据可读取，但该标记本身不是测试通过证据。

## 系统 200% 边界

真实 200% 系统缩放仍为 BLOCKED。该托管会话的原生设置只提供 100% 和 125%，新临时用户的凭据进程仍在 runner 同一交互会话，未产生独立登录桌面；未注销或将模拟 DPI 算作通过。[原生设置](system-200-settings.json) · [用户会话记录](system-200-user-session.json)。Windows 10 与真实断电均未在本轮验收。
