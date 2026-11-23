# 主线安装版的 ZIP 更新与选择性解压

Windows 11 x64 build 26300。本机原位安装主线 `9c9258cc05c87b0061f8670d3abd248c7d706beb`。安装后的桌面程序、CLI、worker 和随包 NanaZip 的六个二进制均与该提交的托管 x64 产物逐项核对。[构建与哈希](package.json) · [托管产物来源](https://github.com/qing-yi-5427/qing-zip/actions/runs/45216259741)

| 更新操作 | 输入规模与轮次 | 重开和重试后，原包旁的候选数据 | 原包与来源、独立并发任务、重试成品 |
| --- | --- | --- | --- |
| 改名 | 256 MiB ZIP，三轮 | 0、0、0 字节 | 均通过。[逐轮数据](rename.jsonl) |
| 删除 | 256 MiB ZIP，三轮 | 0、0、0 字节 | 均通过。[逐轮数据](delete.jsonl) |
| 新增 | 128 MiB 新成员，三轮 | 3、3、2 MiB | 均通过。[逐轮数据](add.jsonl) |

每轮在独立临时目录中确认指定引擎正在写入，再强制结束该操作的控制进程。重新打开、核对哈希、重试，并在两个时点检查残留。回执分别记录原包旁的候选和系统临时目录的小型诊断文件。本组覆盖本地 NTFS、随包 NanaZip、普通单卷明文 ZIP 的写入阶段。写完到提交时中断及实际断电尚待验收。[复跑脚本](../../../../scripts/check-interrupted-archive-update.py) · [压缩方式与兼容范围](../../zip-update-owned-job/disposable-zip-edits.md)

安装版另做了 11 个选择性解压真实窗口用例，覆盖同名文件、同名目录、四种冲突策略、提交前取消及改选保存位置重试。另一次对 384 MiB 成员在任务运行中点击取消，比较了取消后的目标清单和文件哈希。[原始窗口回执](selected-extraction.json) · [换位置重试](parent-retry.png) · [运行中取消](selected-cancel.json) · [修复前窗口与复跑方法](../selected-extract-conflicts/README.md)
