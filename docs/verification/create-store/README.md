# 新建 ZIP 的“仅存储”验收

在 Windows 11 x64 实际运行 Tauri/WebView2 窗口，以相同的 225,000 字节测试文件调用已有的 `--compress-files` 入口。在新建窗口将高级方法设为 Deflate，再选择“0 · 仅存储”，点击创建，等待窗口显示完整性检查通过。随后用 Python 标准库读取实际 ZIP 方法及文件内容。[复跑脚本](../../../scripts/check-create-store-gui.py)

| 构建 | 当时窗口中的方法 | 最终 ZIP 方法 | 打包后的条目字节 |
| --- | --- | --- | ---: |
| 改动前 | Deflate | 8，Deflate | 628 |
| 改动后 | Store | 0，Store | 225,000 |

两次解压均还原同一内容，SHA-256 为 `46d23027c8be96f2d6e2b025ba26dc080dae5ced2cf5e47409ede3683cfdda3e`。测试使用此前验收的组合版本与包含本次应用改动的组合候选，完整 GUI 二进制哈希已记录。[改动前截图](before.png) · [改动后截图](after.png) · [创建完成](completed.png) · [原版实测记录](before.json) · [候选实测记录](after.json)

命令行与 JSON 沿用既有约定，显式压缩方法决定可用级别的含义。图形界面的级别和方法则随用户选择联动。使用 Python Playwright 连接程序自带的 WebView2 即可复跑，创建文件与用户配置各自在本次临时目录内。

```powershell
python scripts/check-create-store-gui.py --exe <原版便携目录>/qing-zip.exe --out <新证据目录> --baseline
python scripts/check-create-store-gui.py --exe <候选便携目录>/qing-zip.exe --out <另一新证据目录>
```

## 折叠高级设置后从“仅存储”切回“标准”

高级设置保持折叠，只把可见的压缩级别从“0 · 仅存储”改回“5 · 标准（推荐）”，然后创建。改动前，隐藏的方法仍为 Store（ZIP）或 Copy（7z），压缩包实际未压缩；改动后方法回到“自动”。ZIP 用 Python `zipfile` 读取条目方法，7z 用 py7zr 读取编码器并解压，两者均不经过 qing-zip 或其引擎。

| 构建 | 格式 | 窗口显示级别 | 隐藏方法 | 实际方法 | 条目或包字节 |
| --- | --- | ---: | --- | --- | ---: |
| 改动前 `03ee431` | ZIP | 5 | Store | 0，Store | 225,000 |
| 改动前 `03ee431` | 7z | 5 | Copy | COPY | 225,130 |
| 改动后 | ZIP | 5 | 自动 | 8，Deflate | 628 |
| 改动后 | 7z | 5 | 自动 | LZMA2 | 265 |

四次解压内容 SHA-256 均为 `46d2…da3e`，与来源一致。显式选择 Store / Copy 仍会把级别设为 0，并实际写出未压缩条目，已用同一构建复跑上节场景（ZIP 与 7z）。[改动前窗口](roundtrip-before.png) · [改动后窗口](roundtrip-after.png) · 实测记录：[改动前 ZIP](roundtrip-before-zip.json)、[改动前 7z](roundtrip-before-7z.json)、[改动后 ZIP](roundtrip-after-zip.json)、[改动后 7z](roundtrip-after-7z.json)

```powershell
python scripts/check-create-store-gui.py --exe <便携目录>/qing-zip.exe --out <新证据目录> --scenario roundtrip --format zip [--baseline]
python scripts/check-create-store-gui.py --exe <便携目录>/qing-zip.exe --out <新证据目录> --scenario roundtrip --format 7z [--baseline]
```
