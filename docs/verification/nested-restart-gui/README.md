# 嵌套 ZIP 跨会话：R3 真实窗口基线

[托管 Windows GUI run 45140184822](https://github.com/qing-yi-5427/qing-zip/actions/runs/45140184822) 使用 R3 源码 `fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6` 的现成便携包，包与 GUI 哈希见 [package.json](package.json)。通过真实 Tauri/WebView2 和 CDP 操作，使用独立 profile 和普通合成 ZIP；仅在一次性 runner 设置并恢复该进程的 WebView2 调试覆盖。每轮双击 `outer.zip/inner.zip`，将 `before.txt` 重命名为 `after.txt`，检查编辑工作副本、原外包、另一份相同内包、外层成员及同目录哨兵。

- **PASS：正常取消及放弃。** 点击窗口关闭时出现 [保存提示](close-prompt.png)；取消后编辑副本仍在，再关闭并明确放弃，进程正常退出码 0。重开内包显示 `before.txt`，原包及兄弟项未变。
- **PASS：保存并关闭。** 选择保存并关闭标签，再正常关闭窗口，进程退出码 0。重开内包显示 `after.txt`，无关成员及旁边哨兵不变。
- **已知 FAIL：强制中断后恢复入口。** [编辑中的内包](edited-inner.png) 随本脚本创建的 GUI 进程被强制结束；原外包未变，临时内包仍与中断前编辑副本哈希一致。重开后 [内包仍是原始内容](reopened-inner.png)，实际打开“保留的编辑副本”却 [显示为空](retained-empty.png)。第二次重开仍为零项，原包及哨兵未变。这不是工作副本已损坏或已被删除的证明，而是正常界面未发现它。

逐项哈希、PID、退出方式及第二次重开见 [机器回执](baseline.json)，调试策略恢复见 [desktop-session.json](desktop-session.json)。运行成功表示观察完成，不能当作强退恢复通过。另存恢复副本、候选修复以及系统文件对话框尚未在本基线上通过。[三层外部写入冲突基线](three-layer.md) 单独记录正常用户操作期间的嵌套冲突。

复跑：在 GitHub Actions 的 `Windows regression` workflow 选择本分支并设置 `nested_restart_capture=true`；该入口固定下载 R3 构建，不启用深度或特殊文件系统测试。候选测试脚本支持 `--expect-recovery`：要求恢复入口恰有一项，通过真正的 Windows 另存对话框写入全新独立目录，核验输出与编辑副本 SHA、成员及二次重开不新增副本；这一候选模式仍待产品实现及实跑，不能视为通过。
