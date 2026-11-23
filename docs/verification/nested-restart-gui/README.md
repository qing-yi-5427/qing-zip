# 嵌套 ZIP 跨会话：R3 真实窗口基线

[托管 Windows GUI run 45140184822](https://github.com/qing-yi-5427/qing-zip/actions/runs/45140184822) 使用 R3 源码 `fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6` 的现成便携包，包与 GUI 哈希见 [package.json](package.json)。通过真实 Tauri/WebView2 和 CDP 操作，使用独立 profile 和普通合成 ZIP；仅在一次性 runner 设置并恢复该进程的 WebView2 调试覆盖。每轮双击 `outer.zip/inner.zip`，将 `before.txt` 重命名为 `after.txt`，检查编辑工作副本、原外包、另一份相同内包、外层成员及同目录哨兵。

- **PASS：正常取消及放弃。** 点击窗口关闭时出现 [保存提示](close-prompt.png)；取消后编辑副本仍在，再关闭并明确放弃，进程正常退出码 0。重开内包显示 `before.txt`，原包及兄弟项未变。
- **PASS：保存并关闭。** 选择保存并关闭标签，再正常关闭窗口，进程退出码 0。重开内包显示 `after.txt`，无关成员及旁边哨兵不变。
- **已知 FAIL：强制中断后恢复入口。** [编辑中的内包](edited-inner.png) 随本脚本创建的 GUI 进程被强制结束；原外包未变，临时内包仍与中断前编辑副本哈希一致。重开后 [内包仍是原始内容](reopened-inner.png)，实际打开“保留的编辑副本”却 [显示为空](retained-empty.png)。第二次重开仍为零项，原包及哨兵未变。这不是工作副本已损坏或已被删除的证明，而是正常界面未发现它。

逐项哈希、PID、退出方式及第二次重开见 [机器回执](baseline.json)，调试策略恢复见 [desktop-session.json](desktop-session.json)。运行成功表示观察完成，不能当作强退恢复通过。原始 R3 基线没有可供另存的恢复记录；候选结果见下方。[三层外部写入冲突基线](three-layer.md) 单独记录正常用户操作期间的嵌套冲突。

复跑：在 GitHub Actions 的 `Windows regression` workflow 选择本分支并设置 `nested_restart_capture=true`；该入口默认固定下载 R3 构建，不启用深度或特殊文件系统测试；可显式提供同仓库成功的 Windows regression 构建 run、源码提交及便携 ZIP SHA-256，以绑定候选。优先使用直接分支构建或手动运行，避免 PR 合并树与 head 不一致。候选测试脚本支持 `--expect-recovery`：要求恢复入口恰有一项，通过真正的 Windows 另存对话框写入全新独立目录，核验输出与编辑副本 SHA、成员及二次重开记录从一项变零项。随后三层控制要求两项同名来源可区分、取消原生另存和一次直接后端保存失败后仍可各自导出，成功导出 B 后只保留 A，再导出 A 后为零项；候选的实际结果见下方；对话框取消是真实 UI 操作，失败情形为直接后端请求。

## AES 外层控制组

[托管 run 45141552609](https://github.com/qing-yi-5427/qing-zip/actions/runs/45141552609) 使用临时合成 AES256 加密外层 ZIP，正常输入密码并编辑普通内层，再只强制结束本次 GUI。冷启动后 [恢复菜单为空](encrypted-retained.png)，现有恢复索引不存在，因而未含合成密码；原包和旁边哨兵哈希不变，另用原密码解密并逐字节核对原始内包与外层成员。[回执](encrypted-baseline.json) 不含密码。该结果只约束恢复入口及索引，不表示临时目录中没有解密工作副本。候选包需重新执行此控制组。

## 候选真实恢复结果

[托管 run 45146040536](https://github.com/qing-yi-5427/qing-zip/actions/runs/45146040536) 使用产品源码 `b6201de6bcdbeb3eabc60a6b2528634964b94bd7` 的固定构建（[包与 GUI 哈希](candidate-package.json)）。正常取消、放弃和保存对照继续通过。强制结束本次 GUI 后，冷启动实际菜单显示 [一条带来源的副本](candidate-retained.png)；通过 [Windows 原生另存对话框](candidate-save-as.png) 保存到独立空目录，输出 ZIP SHA-256 与中断前已编辑的工作副本相同，成员均匹配；再次重开时 [记录已消失](candidate-second-reopen.png)，原外包和磁盘哨兵不变。[逐项回执](candidate.json) 含 PID 和退出方式。

同轮 AES 外层控制再次通过：实际恢复菜单无条目，恢复索引无合成密码，原包不变且用原密码解出正确内包和兄弟项。这不代表解密内容未曾存在于临时工作目录。

[三层候选回执](three-layer-candidate.json) 记录两个来源不同但同名 `middle.zip` 的冲突副本：界面可根据真实外层路径区分；选择 B 的 [真实窗口](three-layer-selected.png) 后，取消原生另存保留两项；对不存在的目标目录直接调用后端制造失败也保留两项（该失败不是原生文件对话框发起）。随后分别通过原生另存导出 B 和 A，逐层哈希匹配，B 成功后仅 A 留存，两项均导出后冷启动为零项；外包及哨兵保持不变。桌面调试覆盖和独立测试目录均经回执验证清理。

## 64 MiB 普通 stored 内包

[托管 run 45146592216](https://github.com/qing-yi-5427/qing-zip/actions/runs/45146592216) 对同一候选加入固定种子的 67,108,864 字节普通 stored 成员。真实另存的 ZIP 与中断前编辑副本 SHA-256 均为 `9580dee4be93b59ab2caeee870d06d60ac0bb3603ac273bf54606a44105248c6`，成员及来源、原包和哨兵核验通过，成功保存后第二次冷启动为零条。该轮构造样本 0.471 秒、编辑 0.875 秒、原生另存 5.679 秒；最后一项含自动化与系统对话框等待，仅为单轮诊断，不是跨版本性能比较。[逐项回执](candidate-large.json)。

## 同一会话连续两次编辑

[托管 run 45148196066](https://github.com/qing-yi-5427/qing-zip/actions/runs/45148196066) 使用同一固定候选 b6201de，在实际内层标签连续将 `before.txt` 改名为 `after.txt`、再改名为 `final.txt`。仅结束本测试 GUI 后冷启动，通过真实 Windows 对话框另存；输出 SHA-256 `71cbe9d36ea04b232daa10a238ce28f9900a1c360159608ea1ffd5e7bd4fb5c1` 与第二次编辑后的工作副本完全相同、不同于第一代，成员仅有 `final.txt` 和 `unchanged.txt`。原外包、同内容兄弟内包、外层成员和磁盘哨兵保持完整；再次启动后恢复记录从 1 条变为 0 条。[机器回执](double-edit.json)记录两代成员、哈希与 PID。本项单独运行，没有混入性能测量或重复截图。
