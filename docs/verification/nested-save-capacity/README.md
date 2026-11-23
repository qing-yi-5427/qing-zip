# 嵌套 ZIP 满盘写回及原位重试

在一次性 Windows x64 桌面中打开合成外层 ZIP，双击内层 ZIP，将 before.bin 改为 after.bin。外层所在的 96 MiB NTFS 测试盘余量降至 0 字节后，点击「保存至外层」。读取原包、同目录文件和内层工作副本的 SHA-256，释放测试盘空间，在同一标签再次保存。Python 独立读取外层和内层 ZIP，逐成员比较内容。

| 实际窗口 | 原版 | 候选 |
| --- | --- | --- |
| 写回失败 | [临时路径和重复的系统提示](before/nested-insufficient-space.png) | [用户打开的归档路径和一条中文提示](after/nested-insufficient-space.png) |
| 在同一标签重试成功 | [原版窗口](before/nested-save-retried.png) | [候选窗口](after/nested-save-retried.png) |
| 输入、工作副本、最终归档的 SHA-256 | [原版](before/zip-space-receipt.json) | [候选](after/zip-space-receipt.json) |

两版均保留了原包和内层工作副本。候选失败窗口显示实际的 outer.zip › inner.zip，提示空间不足、更改已保留、可重试或另存为。释放 80,580,608 字节后，保存进外层的内层 ZIP 与此前的编辑副本逐字节一致。此流程验证一次内层改名和原位重试。本组覆盖一层内包。

[原版实测](https://github.com/qing-yi-5427/qing-zip/actions/runs/45125468779) · [带界面断言的候选实测](https://github.com/qing-yi-5427/qing-zip/actions/runs/45127164577) · [候选加固构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45126120466) · [产品修改](https://github.com/qing-yi-5427/qing-zip/pull/29)

## 原盘仍满时另存至另一卷

在同一真实会话中，原盘仍为 0 可用字节，打开 Windows 原生「另存为」，选择测试盘以外的独立目录，用键盘输入 saved-inner.zip 并点击保存。副本的成员和整个文件 SHA-256 都与改名后的内包一致。外层 ZIP、相邻文件及当前工作副本保全，原标签仍可保存至外层。释放 82,984,960 字节后，原标签再次保存，写入的内包与另存副本逐字节一致。

[原生文件选择器](integrated/nested-save-as-dialog.png) · [另存成功窗口](integrated/nested-save-as-complete.png) · [重试成功窗口](integrated/nested-save-retried.png) · [工作副本、另存与最终写回哈希](integrated/zip-space-receipt.json) · [合并候选实测](https://github.com/qing-yi-5427/qing-zip/actions/runs/45130104050) · [独立产品分支复测](https://github.com/qing-yi-5427/qing-zip/actions/runs/45129576104)

复跑时选择 zip-disk-capacity.yml 的 nested-feedback，提供同一仓库 Windows regression 构建的 candidate_build_run 和完整 candidate_source。也可用 update 选择直接 ZIP 更新的 0 字节和 6 MiB 两组测试。

## 真实窗口错误反馈耗时

托管 Windows x64 build 26100、WebView2 157、1028 × 779 视口、100% 缩放。以原版、候选、候选、原版的顺序启动四次应用，每次先触发一次满盘错误预热，再点五次保存。计时从实际点击事件到 WebView 第一次可见失败卡片的动画帧，包含真实写回请求。原版十次中位 60.75 毫秒（56.5–67.7），候选十次中位 58.90 毫秒（54.9–63.0）。按轮次和会话内序号对齐后，差值中位 −1.80 毫秒，范围 −12.6 至 +0.1 毫秒。四个会话都保全原包和工作副本，释放空间后成功写回。此测量覆盖热会话中的该错误反馈，未测冷启动、物理屏幕呈现或 GPU 耗时。[20 个样本](dialog-timings.json) · [同机配对运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45131491561)
