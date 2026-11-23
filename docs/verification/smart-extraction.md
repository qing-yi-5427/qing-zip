# 智能解压输出目录验收

在 Windows 11 x64 build 26300 上，用同一个临时 `bundle.zip` 对比修复前后两个 0.12.23 便携构建。包内只有 `BUNDLE/photos/a.txt`，文件内容与哈希一致。普通智能解压预告均为 `out/BUNDLE`。修复前任务记录指向 `out`，修复后指向 `out/BUNDLE`。

- [修复前任务截图](smart-extraction/before-task.png)
- [修复后任务截图](smart-extraction/after-task.png)
- [首页文件选择后的深度解压预告](smart-extraction/home-picker-preview.png)
- [相对路径及 SHA-256 回执](smart-extraction/receipt.json)

## 复跑

准备 `01dabc4` 和 `c620aa1` 的 Windows x64 便携构建，完整解包至不同目录。构建与打包命令参见项目构建说明。 [README](../../README.md#构建)

关闭其他 qing-zip 窗口后运行以下命令。将两个可执行文件位置改为本机路径。脚本为三次启动分别创建临时配置，通过 WebView2 调试连接执行真实界面输入，并通过 Windows 文件选择器为首页选取临时文件。

```powershell
python -m pip install websockets pywinauto
python scripts/check-smart-extraction-gui.py `
  --baseline 'C:\path\to\before\qing-zip.exe' `
  --candidate 'C:\path\to\after\qing-zip.exe' `
  --out 'target\smart-extraction-verification' `
  --scratch-parent (Join-Path $env:PUBLIC 'Documents')
```

输出包含双方预告与任务截图，以及包含相对路径与哈希的 JSON。从首页选择后缀改为 `.dat` 的普通 ZIP 会打开预填的深度解压对话框，预告与任务均指向 `home`。脚本还核对工具箱批量输出、深度同名目录编号和 `--smart-extract` 参数转交。真实 Explorer 右键菜单与 OLE 拖放属于单独的验收范围。
