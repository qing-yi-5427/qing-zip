# Windows 选择性解压冲突矩阵

在 Windows 11 x64 的真实 qing-zip 窗口勾选两个 ZIP 成员，再从解压下拉菜单选择标准解压。包内另有两个未选项目，目标内保留独立文件、未选成员的同名本地文件，以及已经使用的第一个数字后缀。每轮均使用隔离的 WebView 配置和临时目录。测试期间关闭智能与深度解压，选定范围不能启用原包回收。

主线基线为 `14b9e2a8a5c2de35dea31acf1f222ad10ca354f0`，对应的托管产物 `14828101327` 所含便携 ZIP SHA-256 为 `0c00da2912bc7cc5b142dab31161b296ca03bd926e878dc781ba45d0fa0a9d66`。GUI、CLI 和工作进程分别按构建收据核对过。[构建与产物](https://github.com/qing-yi-5427/qing-zip/actions/runs/45191942063/artifacts/14828101327) · [原始回执](baseline.json)

| 实际表单操作 | 同名普通文件 | 同名目录（内含本地文件） |
| --- | --- | --- |
| 停止并报告冲突 | 无出档，有冲突重试入口 | 无出档，有冲突重试入口 |
| 自动重命名 | 保留原文件与 `(1)`，出档到 `(2)`；另一个选中项写入原名 | 保留目录及本地文件，出档到 `(2)`；另一个选中项写入原名 |
| 跳过已有文件 | 保留已有文件，仅出档未冲突选中项 | 保留目录及本地文件，仅出档未冲突选中项 |
| 替换已有文件 | 只替换普通同名文件，同时写入另一选中项 | 无出档，保留目录及本地文件，有冲突重试入口 |
| 按取消关闭表单 | 没有启动任务，目标完全不变 | — |

每个用例核对预计目录、任务状态、成功任务报告的目录和实际文件树。两个未选成员都保持在源包中，目标上已有的同名本地文件不变，未选目录没有生成。原包和目标内外的旁文件通过 SHA-256 核对，ZIP 另经 Python 的 CRC 和内容检查。

[重命名预告](file-rename-preview.png) · [重命名完成](file-rename-result.png) · [同名目录阻止替换](directory-overwrite-result.png)

## 同名文件挡住所需目录

另选中一个普通文件和一个含有文件的包内目录 `picked/`，目标已有普通文件 `out/picked`。跳过和自动重命名均安全停住，留下完整原包、目标文件和旁文件。基线窗口只显示 `create output directory: … (os error 183)`，未给出阻挡路径。调整保存位置后，重试会沿用两个选中项目，未选成员不出档。

修复将阻挡文件的实际路径传给任务，并在任务和重试表单中提示改用其他保存位置。已有的冲突处理与磁盘保护保持原样。[基线错误图](parent-skip-before.png)

## 独立复跑

在 Windows x64、WebView2 已安装的系统中运行。使用 Python 3.11 或以上版本，并在测试 Python 环境里安装 `playwright`、`pywin32`。Playwright 通过本机 WebView2 的调试接口驱动真实应用。测试期间请保持 qing-zip 的其他实例已退出。

```powershell
gh run download 45191942063 --name qing-zip-windows-x64-trial --dir target/selected-extract-package
$artifact = 'target/selected-extract-package'
$zip = (Get-ChildItem -LiteralPath $artifact -Recurse -File -Filter '*-portable.zip').FullName
$build = (Get-ChildItem -LiteralPath $artifact -Recurse -File -Filter 'hardened-build.json').FullName
Expand-Archive -LiteralPath $zip -DestinationPath target/selected-extract-payload
$gui = (Get-ChildItem -LiteralPath target/selected-extract-payload -Recurse -File -Filter qing-zip.exe).FullName
$fixtures = New-Item -ItemType Directory -Path target/selected-extract-fixtures -Force
python scripts/check-selected-extract-conflicts.py --exe $gui --package $zip --hardening $build `
  --source 14b9e2a8a5c2de35dea31acf1f222ad10ca354f0 --expect-legacy-block-message `
  --out target/selected-extract-evidence --fixtures-in $fixtures.FullName
```

对修复版使用与其源码匹配的便携包、收据及提交，并省去 `--expect-legacy-block-message`。脚本在每个用例退出时清理自建的夹具。证据输出目录必须是新目录，其中保存逐项的输入和输出哈希及截图。`--case file-rename` 或 `--case parent-skip` 可单独运行对应的用例。
