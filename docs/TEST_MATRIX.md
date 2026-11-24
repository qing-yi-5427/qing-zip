# 测试矩阵

针对当前提交的结果见 [Windows 回归运行](https://github.com/qing-yi-5427/qing-zip/actions/workflows/windows-ci.yml)。本地从仓库根目录运行，环境和基础命令见[贡献指南](../CONTRIBUTING.md)。

| 范围 | 入口 | 核对内容 |
| --- | --- | --- |
| x64 运行时与前端 | `python scripts/verify-runtime.py`、`node --test obz-gui/tests/*.cjs` | 随包组件与来源清单一致，任务、密码、选择、创建参数及键盘操作回归 |
| ZIP、7z、RAR 互读 | [普通文件对照](../scripts/compare-common-extraction.py)、[格式互读](../scripts/check-archive-interop.py) | 独立工具解压到空目录，比较相对路径、完整文件内容及原包哈希 |
| 深度解压、冲突与分卷 | [归档集成测试](../obz-rs/tests)、[深度解压说明](deep-extraction.md) | 首卷和嵌套归档、密码续接、同名文件选择、部分失败时的源与目标 |
| 桌面批量解压 | [真实窗口测试](../scripts/check-batch-extract-desktop.py)、[继续任务](../scripts/check-batch-resume-desktop.py) | 从界面启动、停止、继续，检查每项结果与目标目录 |
| Windows Explorer | [组件与调用说明](../qing-shell/README.md) | 原生菜单转交、选择范围、注册和注销的适用条件 |
| 大文件 ZIP64 | [读取成员](../scripts/zip64/member.py)、[选择靠后的成员](../scripts/zip64/offset.py)、[创建和追加](../scripts/zip64/writer.py) | 需要时显式运行，生成样本、核对完整文件内容，临时卷至少预留 24 GiB |

记录执行结果时使用 PASS、FAIL、SKIP、BLOCKED、NOT RUN，并注明跳过或阻塞原因。每个任务使用独立临时目录。截图、日志和回执附在该 PR 或 Actions 构件里。
