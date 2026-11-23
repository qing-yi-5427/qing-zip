# Windows 11 ARM64 Explorer 托管验收

第二轮便携包 SHA-256 `39b494dcbff1b3afa20b4c85bcb0ddb4de5da017fcee3c056593daab038fc10e`，源码 `b2bf9f3f4eaddcbfefd3144ad0fdb8769fa1824b`，来自 ARM64 原生构建 run 45119972417。测试在一次性 `windows-11-arm` 上构建匹配架构的 Explorer DLL 和外置身份包；临时测试证书仅在该 runner 签名和信任，不是正式签名。以产品便携 CLI `shell-enable` 和 GUI 文件压缩开关启用产品入口，并在完成后验证移除包、测试设置及证书。系统首次隐私设置在测试桌面完成。

[run 45132037105](https://github.com/qing-yi-5427/qing-zip/actions/runs/45132037105) 中，真实 Windows 11 Explorer 一级菜单出现 `qing-zip` 分组；点击分组后分别点击 [解压到此处](here-submenu.png) 和 [智能解压](smart-submenu.png)。两个顶层成员分别在原包同级及 `archive` 同名目录产生，逐一 SHA-256 匹配；原包、普通文件及无关哨兵未变，目标目录内容与预期完全一致。完整 [逐项回执](extraction.json) 与 [注册/清理回执](extraction-setup.json) 均保留；该 run 的总状态为失败，因为压缩输出尚未验证，不影响已记录的两项结果。

[最终 run 45133376060](https://github.com/qing-yi-5427/qing-zip/actions/runs/45133376060) 三项均通过完整断言：[文件压缩子菜单](compress-submenu.png) 进入 [创建表单](compression-form.png)，以合成 `plain.txt` 创建 `plain.txt.zip`。ZIP 唯一成员 SHA-256 `451735e693c80828f6ae2df7fbef35d307cb2a4a7881ceaf23e9789839758548` 与来源相同；原包、来源及哨兵未变，目录内容与预期一致。[目标 ZIP 已可独立读取时的窗口](compression-readable.png) 仍显示创建进行中，并非完成画面。测试随后终止自己启动的应用窗口，重新读取 ZIP，包哈希与成员内容均一致；未观察任务中心的完成状态。该轮 [逐项回执](final.json) 和 [构建、签名及清理回执](final-setup.json) 记录 GUI、DLL、host 与测试 MSIX 哈希。便携迁移和卸载后菜单显示另行测试，正式受信签名和 x64 桌面实际注册未在此验证。此测试未触碰真实用户机器或个人数据，也未执行特殊文件系统测试。

复跑：Actions 的 `Explorer review package` workflow 选择本分支并显式设置 `hosted_explorer=true`；该入口只接受该仓库的一次性 GitHub 托管 Windows 11 ARM64 runner。完整原始日志、截图及逐项收据可从上述运行的 `hosted-explorer-review` artifact 获取（30 天）。
