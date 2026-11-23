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

输出包含双方预告与任务截图，以及包含相对路径与哈希的 JSON。从首页选择后缀改为 `.dat` 的普通 ZIP 会打开预填的深度解压对话框，预告与任务均指向 `home`。脚本还核对工具箱批量输出、深度同名目录编号和 `--smart-extract` 参数转交。已注册的 Explorer 右键菜单需独立验收。

## 从资源管理器实际拖入首页

托管 Windows x64 桌面上，将临时文件夹中的 `parcel.dat` 从原生 Explorer 实际拖入 qing-zip。驱动核对了鼠标按下和落点所属的窗口。悬停时，首页显示「松开以识别文件」。松开后应用识别出 ZIP，显示预期输出目录。点击开始后，任务记录指向 `parcel`，磁盘中的 `parcel/item.txt` 与生成的内容相同，原包 SHA-256 保持一致。桌面为 1024 × 768。应用使用真实 Tauri 文件拖放接口。[托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45109164716) · [鼠标和文件回执](smart-extraction/explorer-drop/receipt.json)

[悬停](smart-extraction/explorer-drop/hover.png) · [识别与预告](smart-extraction/explorer-drop/preview.png) · [完成结果](smart-extraction/explorer-drop/result.png)
