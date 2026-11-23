# 普通解压冲突验收

以下步骤使用 Windows x64 便携版、PowerShell 7 和 Python 3。先关闭其他 qing-zip 窗口，使测试归档由独立的便携版进程打开。

## 准备测试归档

将 `$portable` 改为解包后的便携版目录，在同一个 PowerShell 窗口执行。

```powershell
$portable = 'C:\path\to\qing-zip-0.12.23-portable'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('qing-zip-conflicts-' + [guid]::NewGuid().ToString('N'))
@'
from pathlib import Path
import sys
from zipfile import ZIP_DEFLATED, ZipFile

root = Path(sys.argv[1])
(root / 'rename').mkdir(parents=True)
(root / 'blocked' / 'photo.txt').mkdir(parents=True)
(root / 'rename' / 'photo.txt').write_bytes(b'local photo')
(root / 'blocked' / 'photo.txt' / 'local.txt').write_bytes(b'local photo')
with ZipFile(root / 'album.zip', 'w', ZIP_DEFLATED) as archive:
    archive.writestr('photo.txt', b'base photo')
    archive.writestr('PHOTO (1).txt', b'numbered photo')
    archive.writestr('photo (2).txt/cover.txt', b'cover')
'@ | python - $fixture
if ($LASTEXITCODE) { throw '测试归档创建失败' }
$album = Join-Path $fixture 'album.zip'
$sourceHash = (Get-FileHash -LiteralPath $album).Hash
$start = [Diagnostics.ProcessStartInfo]::new((Join-Path $portable 'qing-zip.exe'))
$start.UseShellExecute = $false
$start.WorkingDirectory = $fixture
$start.Environment['LOCALAPPDATA'] = Join-Path $fixture 'local-profile'
$start.Environment['WEBVIEW2_USER_DATA_FOLDER'] = Join-Path $fixture 'webview-profile'
$start.Environment['TEMP'] = $fixture
$start.Environment['TMP'] = $fixture
$start.ArgumentList.Add($album)
[Diagnostics.Process]::Start($start) | Out-Null
$fixture
```

## 在便携版中验收

1. 在工具栏展开解压菜单，选择「解压到…」。范围选「整个压缩包」，取消「智能解压」，保持「成功后回收」关闭。将目标设为 `$fixture\rename`，将同名处理设为「自动重命名」，开始解压。任务应显示已提取 3 个文件。本地 `photo.txt` 仍为 `local photo`，新增 `photo (3).txt` 为 `base photo`，归档中的 `PHOTO (1).txt` 和 `photo (2).txt/cover.txt` 保留原名及内容。
2. 再打开「解压到…」，将目标设为 `$fixture\blocked`，同名处理设为「停止并报告冲突」，智能解压和回收保持关闭。任务应报告「目标位置已有同名项目」，给出以 `blocked\photo.txt` 结尾的路径。该目录中仍只有本地 `photo.txt\local.txt`。
3. 从失败任务选择「处理冲突并重试」，将同名处理改为「跳过已有文件」。任务应显示已提取 2 个文件。`blocked` 下应有 `PHOTO (1).txt`、`photo (2).txt\cover.txt` 和原有的 `photo.txt\local.txt`，三项内容与准备步骤一致。
4. 回到 PowerShell，执行 `(Get-FileHash -LiteralPath $album).Hash -eq $sourceHash`，结果应为 `True`。

Windows 11 x64 build 26300 上使用 0.12.23 x64 便携构建完成了上述三次操作。回执记录各步骤的输出相对路径、文件 SHA-256 和任务文案。截图中的目录为此次验收创建的公共临时路径。 [验收回执](extraction-conflicts/receipt.json) · [重试完成截图](extraction-conflicts/retry-completed.png)
