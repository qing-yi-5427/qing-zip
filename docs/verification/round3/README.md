# 第三轮 0.12.23 审阅候选

本地起点 0cc51aa，经前两轮候选 b2bf9f3 继续。本轮交付包仅增加直接 ZIP 更新的进程控制和嵌套写回提示两项产品改动。安装迁移和 Explorer 一级菜单验证分别审阅。[组合源码](https://github.com/qing-yi-5427/qing-zip/tree/fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6) · [更新修复](https://github.com/qing-yi-5427/qing-zip/pull/28) · [任务反馈](https://github.com/qing-yi-5427/qing-zip/pull/29)

| Windows 便携包 | 字节 | SHA-256 |
| --- | ---: | --- |
| [x64 构建与下载](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128040411) | 32,633,048 | `b09d5089eaa0e8bcb5f376598c03be99e5773a716f008e17d49f696450bee6d5` |
| [ARM64 构建与下载](https://github.com/qing-yi-5427/qing-zip/actions/runs/45128040730) | 31,126,536 | `cc0e06dafef5d4caf7f8905ee87c5396f8e3f022c03c2a6f382dada4321f3bbe` |

两个下载包逐条 CRC 检查通过。与手头 R2 交付包对比，x64 的 731 条和 ARM64 的 727 条路径完全相同。x64 有 656 个文本文件仅换行不同，另有三份产品可执行文件内容变化。ARM64 除三份产品可执行文件，还重建了 7z.sfx 与 obz-zpaq.exe，这两文件仅在 PE 与调试时间戳位置各有 4 个字节变化。相应的来源和校验记录同步变化。[下载依据、文件摘要与架构](package-comparison.json)

- 用交付的 x64 CLI 实际中断三轮直接 ZIP 更新，引擎在父进程退出采样后 0.8–1.5 毫秒内退出，写入增长均为零。原包、来源、并行作业和重试通过，每轮仍留下 2–3 MiB 本次已写出的候选。[独立包检查](acceptance.json) · [逐轮结果](zip-interruption.jsonl)
- 同一包在已满和余 6 MiB 的 96 MiB NTFS 私有盘上更新 ZIP。两次失败都保住原文件并能原命令重试。[完整哈希和诊断](update-zip-space-receipt.json) · [测试盘与清理](update-isolated-vhd.json) · [托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45130443977)
- 嵌套保存失败后，原盘仍剩余 0 字节时用原生选择器另存到另一卷。然后释放空间，在原标签保存到外层，三份内包 SHA-256 一致。[原生窗口与交付包实测](https://github.com/qing-yi-5427/qing-zip/tree/65b0298/docs/verification/nested-save-capacity/integrated)
- 在 10 个输入上分别比较工具实际输出，qing-zip 10/10。前九个输入与 7-Zip、Bandizip 均一致。旧式 RAR 存储三卷上，qing-zip 与官方 UnRAR 通过，另外两款工具退出码均为 2。另有普通 ZIP/7z 双向互读通过。[34 个工具与样本组合的原始状态](format-comparison.jsonl) · [双向互读](interop.json)
- 用交付的同一 ARM64 包实际切换普通、高对比度、减少动画并恢复，菜单均有 7 项。[原生窗口验收](https://github.com/qing-yi-5427/qing-zip/pull/31)

真实窗口的满盘保存计时、两档普通 ZIP 同进程并发更新，以及安装与 Explorer 技术验证分别保留了测试入口和逐轮结果。[性能报告](../../PERFORMANCE_REPORT.md) · [交互升级](https://github.com/qing-yi-5427/qing-zip/pull/27) · [移动外置目录](https://github.com/qing-yi-5427/qing-zip/pull/33) · [真实一级菜单动作](https://github.com/qing-yi-5427/qing-zip/pull/34)

GitHub 运行的产物有保留期。可使用对应源码和普通 Windows 工作流重建，复测时记录新包的 SHA-256。
