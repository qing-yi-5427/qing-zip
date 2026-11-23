# 同会话继续批量解压时确认替换

普通合成 ZIP 在隔离配置与私有 Windows 11 x64 桌面中运行：第一项完成，第二项在真实任务运行中停止，第三项的目标事先放置同名文件。来源、目标和旁文件均逐项记录哈希。

- [基线 main e2dfd0b](baseline.json)：同会话直接继续未弹确认，第三项目标被替换。GUI 和 CLI 来自 run 45265612278 的便携包。
- [早期候选 8738262](candidate.json)：同会话出现确认，返回后整个输出快照不变；重新继续并明确确认后，第三项才按策略替换。其 GUI 基于 #63 的独立候选，CLI、worker 与基线相同；它不是此 PR 的最终打包产物。
- [候选续跑目录不可写](unavailable.json)：以隔离配置中的普通文件阻挡续跑目录，界面显示仅本次会话可继续；返回仍无输出变化，确认后才替换。阻挡文件、所有源包和其他旁文件不变。

三次程序均正常退出并核查测试目录清理。[替换确认](confirmation.png) · [不可写时返回](unavailable-returned.png)。截图遮蔽临时路径。

当前 PR 的 x64 打包回归会使用该构建的 `qing-zip.exe`，通过 `check-same-session-overwrite.py --hosted` 在同一真实 Windows 桌面分别运行 normal 和 unavailable；校验便携包与加固二进制哈希。失败也会写入 `receipt.json` 并尝试保存脱敏截图，随现有 batch-resume artifact 保存。原始测试不涉及真实用户文件、特殊文件系统或前台用户桌面。
