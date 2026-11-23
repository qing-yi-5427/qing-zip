# 嵌套 ZIP 满盘写回及原位重试

在一次性 Windows x64 桌面中打开合成外层 ZIP，双击内层 ZIP，将 before.bin 改为 after.bin。外层所在的 96 MiB NTFS 测试盘余量降至 0 字节后，点击「保存至外层」。读取原包、同目录文件和内层工作副本的 SHA-256，释放测试盘空间，在同一标签再次保存。Python 独立读取外层和内层 ZIP，逐成员比较内容。

| 实际窗口 | 原版 | 候选 |
| --- | --- | --- |
| 写回失败 | [临时路径和重复的系统提示](before/nested-insufficient-space.png) | [用户打开的归档路径和一条中文提示](after/nested-insufficient-space.png) |
| 在同一标签重试成功 | [原版窗口](before/nested-save-retried.png) | [候选窗口](after/nested-save-retried.png) |
| 输入、工作副本、最终归档的 SHA-256 | [原版](before/zip-space-receipt.json) | [候选](after/zip-space-receipt.json) |

两版均保留了原包和内层工作副本。候选失败窗口显示实际的 outer.zip › inner.zip，提示空间不足、更改已保留、可重试或另存为。释放 80,580,608 字节后，保存进外层的内层 ZIP 与此前的编辑副本逐字节一致。此流程验证一次内层改名和原位重试。点击「另存为」和更多嵌套层级留给独立用例。

[原版实测](https://github.com/qing-yi-5427/qing-zip/actions/runs/45125468779) · [带界面断言的候选实测](https://github.com/qing-yi-5427/qing-zip/actions/runs/45127164577) · [候选加固构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45126120466) · [产品修改](https://github.com/qing-yi-5427/qing-zip/pull/29)
