# 单个 ZIP64 大文件的任务反馈与磁盘占用

Windows 11 x64 26300、NTFS、Ryzen 7 9800X3D、48 GiB。使用已安装的 0.12.23，产品提交 `3bd150c`。归档与已独立验收的成员大小上界样本 SHA-256 相同，单个文件的展开尺寸为 4,295,032,833 字节。[样本生成和成员校验](../zip64-size/README.md)

## 已安装桌面程序

在真实 Tauri/WebView2 私有桌面打开归档，选中文件并通过 Alt+Enter 查看属性。在 150% 系统缩放下，原始大小显示 4,295,032,833 字节、压缩后大小显示 120,043,460 字节。关闭属性后，从解压菜单选普通解压并指定空目标目录。任务面板从「等待引擎响应 · 进度待定」变为「验证并解码 · 本阶段 …%」。观测到的 14 个阶段百分比单调上升。完成后出现「已完成 · 100%」和「打开输出位置」。此用例验证按钮可见，尚未点击该按钮验收 Explorer 导航。

实际输出只有一个文件，完整 SHA-256 与命令行和独立解码器一致。程序正常退出，源归档哈希保持，临时配置与输出均已清理。[运行中实拍](installed-running.png) · [完成实拍](installed-complete.png) · [可见文本、属性和文件摘要](desktop-x64.json)

## 临时磁盘空间

使用同一归档在空目录执行已安装 CLI 的单成员解压。一次预热，五次正式测量，保留操作系统缓存。每次启动新进程，输出、工作目录、临时目录及应用配置均位于本次独占目录中。每轮完成后，独立核对最终文件名、长度及全文件哈希，并验证源归档。

| 每次运行中采样到的峰值 | 五轮中位 | 五轮范围 |
| --- | ---: | ---: |
| 测试目录中可见的输出与临时文件逻辑长度之和 | 4,295,032,833 B | 4,295,032,833–4,295,032,833 B |
| 同一卷从启动前到采样最低点的可用空间下降量 | 4,295,041,024 B | 4,295,041,024–4,295,041,024 B |

路径统计排除了运行前已有的合成源文件及测试 ZIP。最终文件以外未采到正长度的临时文件。轮询目标间隔 25 ms，正式轮的最大实际间隔介于 116.0 和 187.1 ms。短命文件、延后更新的文件长度、无目录名的打开文件可能不体现在路径统计中。整卷空间读数也受其他活动和文件系统分配影响。本次记录适用于此普通、未加密且目标为空的单任务。覆盖、取消及多任务的临时空间另行验收。[逐轮峰值和采样条件](disk-x64.json)

## 复跑

在仓库根目录使用 x64 Windows 和完整 x64 程序目录。准备 Python 3.14，桌面验证使用 pywin32、Playwright Python 包和 WebView2 Runtime。使用空闲的专用测试父目录并保留至少 24 GiB 空间。桌面脚本会先确认没有其他 qing-zip 实例。以下路径参数均可替换，用新的文件名和目录保存回执。

```powershell
$repo = (Get-Item .).FullName
$app = Join-Path $env:LOCALAPPDATA 'Programs/qing-zip'
$cases = Join-Path $repo 'docs/verification/round11'
$scratch = Join-Path $env:TEMP 'qing-zip-space-qa'
New-Item -ItemType Directory -Force $scratch | Out-Null
python "$cases/zip64-task/desktop.py" --extract --repo $repo --exe "$app/qing-zip.exe" --scratch $scratch --out desktop-result --member-receipt "$cases/zip64-size/member-x64.json"
python "$cases/zip64-task/disk.py" --cli "$app/obz.exe" --member-script "$cases/zip64-size/member.py" --member-receipt "$cases/zip64-size/member-x64.json" --scratch $scratch --out disk-result.json
```

磁盘采样脚本记录实际 Windows 版本，并绑定传入成员回执的 CLI 和 worker 哈希；其他构建可先用 member.py 在同一程序目录生成新的成员回执。运行耗时包含采样开销。

桌面用例要求观察到等待时的进度待定、解码阶段有变化的有序百分比、运行中截图，以及完成状态和输出按钮。脚本漏采这些状态时会报告验收失败。可用 `python test-feedback.py` 对完整回执和缺失或倒退的观测运行断言测试。
