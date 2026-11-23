# 普通批量解压跨会话继续

适用于工具箱中的普通批量解压。停止后，或在任务运行时确认取消并正常退出后，重新打开程序，在「任务 → 查看批量结果」确认待解压的来源、每项目标目录和同名文件策略。继续只启动未开始的项目，保持原计划顺序、编号和目标。替换已有文件需要再次明确确认，随后读取这些路径上当前的压缩包。

一个 Windows 账户保留一份待继续记录。新的普通解压批次停止并成功保存时替换此前记录。续跑记录不含密码，暂停时报告未能保存的批次可在本次会话继续。出现已识别的损坏记录时，可在任务或批量页备份其原始字节后重新保存；其他版本或无法识别的记录保留原样。意外终止不保证保存正在运行的批次。

## Windows 实物验收

原版是 Windows x64 托管运行 45216259741 中的 9c9258cc05c87b0061f8670d3abd248c7d706beb 便携版。真实任务停止后关闭并重新打开，任务页无待继续记录。[原版 SHA-256 回执](baseline-9c9258c.json) · [构建记录](https://github.com/qing-yi-5427/qing-zip/actions/runs/45216259741)

| 用例 | 核对内容 |
| --- | --- |
| 两次正常退出 | 五项中第一项完成、第二项停止。冷启后仅启动第三项，随后在解压中确认退出。再冷启后第四、第五项依次输出到 4-fourth 和 5-fifth，第四项原有文件仍在，另有自动重命名的输出。第一项文件身份、修改时间和所有来源 SHA-256 保持。全部完成后再次打开无待续记录。 |
| 同一份记录被占用 | 另一测试进程持有普通 Windows 文件锁时，继续按钮提示稍后重试，源、目标和记录保持；释放锁后可以继续。另有 Rust 测试验证过期修订无法消费新记录。 |
| 关闭后目标和来源已变 | 第三项 ZIP 在同一路径被测试脚本替换，目标新增同名文件。确认框返回后，目标、记录和已完成项保持。明确选择「按替换策略继续」才输出当前 ZIP 的内容。 |
| 磁盘记录不可写、损坏或来自未来版本 | 配置目录被普通文件占用时仍可同会话继续。已识别损坏记录只在主动点击备份后移开，备份和原记录字节一致。较新格式文件原样保留。 |
| 保存待续记录期间关窗 | 在 WebView 测试中仅延迟真实的保存请求。当前原生解压任务已经结束，从实际标题栏关闭时仍出现确认，保存后正常退出。重新打开能继续第三项。 |

本机候选是 Windows 11 x64 的实际 Tauri/WebView2 窗口，GUI 来自 a963bcf 的 Debug 构建，CLI、worker 与原版便携包逐字节一致。关窗竞争的修前候选在延迟同一请求时直接退出，修后在同一节点显示确认，并等待封存后正常退出。[两次正常退出](candidate-cold-local.json) · [明确替换](candidate-overwrite-local.json) · [不可写](candidate-unwritable-local.json) · [备份损坏](candidate-damaged-local.json) · [封存间隙](candidate-closing-local.json)

## 复跑

使用 Windows、Python 3.12+、pywin32 和 Playwright。程序目录含 GUI、CLI、worker 及同一便携包中的运行时。夹具父目录必须已存在，输出回执目录应为新目录。关闭自己打开的 qing-zip 窗口后启动测试；脚本会拒绝与已有 qing-zip 实例共用。仅使用脚本生成的 ZIP 和独立测试配置。

```powershell
python -m pip install pywin32==312 playwright==1.55.0
$common = @('scripts/check-batch-resume-desktop.py', '--source', '<source-sha>',
  '--exe', 'C:/QingZipQA/portable/qing-zip.exe', '--package', 'C:/QingZipQA/portable.zip',
  '--hardening', 'C:/QingZipQA/hardened-build.json', '--fixtures-in', 'C:/QingZipQA/fixtures')
python @common --out C:/QingZipQA/evidence/cold
python @common --overwrite-review --out C:/QingZipQA/evidence/overwrite
python @common --write-failure --out C:/QingZipQA/evidence/unwritable
python @common --record-recovery --out C:/QingZipQA/evidence/damaged
python @common --exit-while-saving --out C:/QingZipQA/evidence/closing
```

切换到原版程序、便携包及加固回执，同时传入 --baseline，可重复原版对照。日常 x64 Windows CI 用精确打包程序运行五组候选用例并上传屏幕截图、退出收据和文件 SHA-256。

```powershell
node --test obz-gui/tests/batch-outcome.cjs
cargo test --locked --manifest-path obz-gui/Cargo.toml --bin obz-gui batch_resume::tests
```
