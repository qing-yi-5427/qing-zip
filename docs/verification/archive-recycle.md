# 解压后回收原压缩包验收

验收使用 GitHub 托管 Windows x64 桌面和 0.12.23 便携构建。测试先在每个独立临时目录新建一个包含两个普通文件的 ZIP，再从实际应用窗口选择「解压并移入回收站」。

| 操作 | 任务与文件结果 |
| --- | --- |
| 整包解压成功 | 任务显示已提取两个文件并回收原包。系统回收站按唯一文件名和原父目录定位该 ZIP，执行「还原」后原路径上的 SHA-256 与解压前一致。 |
| 只解压选中项目 | 回收选项自动关闭，只输出选中项目，原包 SHA-256 一致。 |
| 有同名文件时选择跳过 | 回收选项自动关闭，既有文件保留，只写入其余文件，原包 SHA-256 一致。 |
| 遇到冲突后停止 | 任务显示冲突，既有文件与原包内容均保留。 |

在工作卷 `D:` 和系统卷 `C:` 分别通过四项验收。同一父目录内的独立 Windows Shell 对照在两卷均新增一个 249 字节的回收项，并成功还原相同字节。两个环境的 Shell 自动化接口读取 SDK 原位置属性均返回空值。原位置列返回字符串，可与唯一文件名一起核对。首次运行只读了属性，未能定位回收项。

[双卷 SHA-256 回执](archive-recycle/receipt.json) · [整包解压选项](archive-recycle/full-options.png) · [同一任务的完成截图](archive-recycle/full-result.png) · [托管运行及原始回执](https://github.com/qing-yi-5427/qing-zip/actions/runs/45112016947)

## 复跑

`Windows regression` 在 GitHub 托管 x64 桌面上用当次生成的便携构建运行 `scripts/check-hosted-recycle.py`，上传 `x64-recycle-acceptance`。在同类 runner 的 PowerShell 步骤中，设置 `$exe` 为解包后 `qing-zip.exe` 的路径，可分别运行下列命令。每条命令都要求一个尚不存在的验收输出目录。

```powershell
python -m pip install pywin32==312 websockets==15.0.1
python scripts/check-hosted-recycle.py --exe $exe --out target/recycle-runner --shell-control
python scripts/check-hosted-recycle.py --exe $exe --out target/recycle-system --fixture-location system --shell-control
```

脚本校验已打开的应用和活动归档，且只还原自己创建的唯一 ZIP。普通 ZIP 的回收和还原由 Windows Shell 及文件哈希独立确认。程序现有回收逻辑仍使用 `PerformOperations` 与 `GetAnyOperationsAborted`，尚未采集每项 `PostDeleteItem` 返回的回收对象。回收站策略禁用、容量不足和其他文件系统条件未纳入本次验收。 [Microsoft 对 PostDeleteItem 的说明](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperationprogresssink-postdeleteitem)
