# 第十轮已安装版 ZIP 性能基线

2026-11-24 在 Windows 11 x64（26300）测量已安装的 qing-zip 0.12.23，产品源码为 `be86c3560cb56e83efaa66b3a18a8d18943754e1`。硬件为 Ryzen 7 9800X3D（8 核 16 线程）、48 GiB 内存，普通临时文件位于 C: NTFS 上的 SOLIDIGM SSDPFKKW020X7 2 TB NVMe。运行脚本使用 Python 3.14.0。在测量前后分别核对与发行包对应的 739 个已安装文件。 [Windows x64 加固构建](https://github.com/qing-yi-5427/qing-zip/actions/runs/45277441627)

## 吞吐与内存

下表各值为五轮中位数，括号内为最小值至最大值。吞吐用原始文件总字节数除以从 CLI 启动到退出的时间。应用以 ZIP Deflate 级别 5 创建包，两条路径均传入 4 线程。工作集为一次采样中已观测进程的工作集之和，再取此轮最高值。

| 样本 | 操作 | 耗时 ms | 吞吐 MiB/s | Job 私有提交峰值 MiB | 采样工作集峰值 MiB |
| --- | --- | ---: | ---: | ---: | ---: |
| 1,024 个小文件，3.99 MiB | 创建 | 155.61（152.07–169.74） | 25.62（23.49–26.22） | 276.92（276.78–276.96） | 37.32（37.27–37.36） |
| 同上 | 解压 | 757.18（753.51–775.57） | 5.27（5.14–5.29） | 7.28（7.26–7.33） | 20.16（20.15–20.19） |
| 64 个混合文件，6.69 MiB | 创建 | 120.70（113.91–125.41） | 55.39（53.31–58.69） | 275.90（275.89–275.91） | 41.30（40.97–41.89） |
| 同上 | 解压 | 148.15（145.96–152.01） | 45.13（43.98–45.80） | 6.35（6.35–6.37） | 19.34（19.05–19.37） |
| 4 个 8 MiB 随机文件，共 32 MiB | 创建 | 253.51（251.45–265.36） | 126.23（120.59–127.26） | 276.36（276.32–276.39） | 67.27（67.14–68.28） |
| 同上 | 解压 | 95.08（90.04–97.20） | 336.58（329.21–355.39） | 5.39（5.38–5.41） | 18.83（18.81–19.21） |
| 深层树，780 个文件、445 个目录，共 3.72 MiB | 创建 | 214.75（208.72–226.56） | 17.34（16.43–17.84） | 277.46（277.35–277.52） | 41.66（41.40–41.98） |
| 同上 | 解压 | 1,093.01（1,061.60–1,114.24） | 3.41（3.34–3.51） | 8.11（8.09–8.12） | 20.84（20.84–20.89） |

小文件与深层树的解压分别完成约 1,352 和 714 个文件每秒。32 MiB 档的吞吐仅适用于这组四文件、低压缩比、热缓存的样本。深层树含中文路径、分支间同名文件、12 个空文件及 48 个空目录。前三档沿用互操作性测试的合成数据，混合档本次测 ZIP，历史基线同档数据使用 7z。[合成样本来源](../../../../scripts/check-archive-interop.py)

## 测量边界

- 每档各预热一次，随后顺序执行五轮。每个操作启动独立 CLI 进程，每轮使用新归档、新解压目录和隔离的临时配置。未清空系统文件缓存，创建后独立读取归档也会使随后的解压受益于缓存。计时覆盖创建进程、引擎工作和发布输出，生成数据、独立校验与试验清理均在计时之外。样本期间的进程快照未见常用编译器，整机 CPU 仅作背景，详见原始记录。
- 创建后用 Python `zipfile` 独立打开并逐项读完，通过 SHA-256 和相对目录结构比对源数据。解压后逐项核对成员字节、空目录及 CLI 报告的文件数，并复查原归档和源目录。四类共 48 个操作全部成功，其中 8 个是预热。进程树 Job 对照另行执行。
- 计时前在相同混合样本分别执行普通启动与纳入 Windows Job 的启动。两种启动的完整归档 SHA-256、内部阶段名及采样到的引擎进程一致。正式运行在 CLI 执行前已将其纳入无资源限制的 Job。每次创建的 Job 生命周期累计 4 个进程，每次解压累计 3 个。扫描中看到了创建的 NanaZip 控制台进程及解压的 `obz-worker.exe`。同包的备用 7-Zip 版本为 26.06。
- Windows 内核持续追踪 Job 的提交内存峰值 `PeakJobMemoryUsed`。进程工作集和私有工作集则按时点采样，所见峰值仅为同口径的下界。目标间隔为 8 ms，正式轮次的相邻采样间隔最大 9.09 ms。通过快照发现子进程的首条样本最迟在启动后约 135 ms 出现，可能漏掉短命进程和短暂峰值。工作集求和可能重复计算共享页，脱敏收据也保留了各进程的 `PrivateWorkingSetSize`。CPU 采样和进程观察的开销包含在测时环境中。[Windows Job 内存字段](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information) · [进程工作集及私有工作集字段](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex2) · [Windows 工作集定义](https://learn.microsoft.com/en-us/windows/win32/memory/working-set)

## 复跑与逐轮数据

安装版 `obz.exe` SHA-256 为 `4b8a992064ad4d5aa80ab421cf0e2baae8305cab9dd46832d1a4efa0a33d3d33`，`obz-worker.exe` 为 `0be714dc3c02a3094f24a5b8a6e51baf7e26d5470d904d0c921115be15fbf246`。二者对应便携发行包 SHA-256 `97a2fc4e963ae910609e3a9eb53a7ad8e0abe699b3dc4e512f5fee70d6b7ec66`。实际观测到的 NanaZip 7.0.1882.0 控制台 SHA-256 为 `06070ca08b3d10e760e8e394a1b6c8ad751aff89f759227ec111b878c12db139`。

从源码库根目录运行以下命令，下载该主线提交的官方 x64 托管产物，验证便携 ZIP 的 SHA-256 后将其展开至新目录。逐件清单和构建绑定收据随仓库提供，包含 ZIP 内的 739 个相对路径及 SHA-256。若机器已安装与该清单吻合的产品，也可直接令 `$cli` 指向安装目录中的 `obz.exe`。[公开 Actions 产物](https://github.com/qing-yi-5427/qing-zip/actions/runs/45277441627/artifacts/14868574888) · [构建绑定](verified-staging.json) · [逐文件清单](candidate-manifest.json)

```powershell
$receipts = 'docs/verification/round10/installed-zip-performance'
$build = Get-Content (Join-Path $receipts 'verified-staging.json') -Raw | ConvertFrom-Json
$work = Join-Path 'target' "installed-zip-bench-$([guid]::NewGuid())"
gh run download 45277441627 --repo qing-yi-5427/qing-zip `
    --name qing-zip-windows-x64-trial --dir (Join-Path $work 'download')
if ($LASTEXITCODE -ne 0) { throw 'Artifact download failed' }
$packages = @(Get-ChildItem (Join-Path $work 'download') -Recurse -File -Filter '*-portable.zip')
if ($packages.Count -ne 1 -or (Get-FileHash $packages[0].FullName).Hash -ne $build.inner_portable_sha256) {
    throw 'Portable artifact SHA-256 mismatch'
}
$unpacked = Join-Path $work 'portable'
Expand-Archive -LiteralPath $packages[0].FullName -DestinationPath $unpacked
$cli = Join-Path $unpacked 'obz.exe'
python scripts/measure-installed-zip.py --cli $cli `
    --manifest (Join-Path $receipts 'candidate-manifest.json') `
    --build-receipt (Join-Path $receipts 'verified-staging.json') `
    --source be86c3560cb56e83efaa66b3a18a8d18943754e1 `
    --out (Join-Path $work 'measurement.json') --cpu 'AMD Ryzen 7 9800X3D' `
    --ram-gib 48 --drive 'C: NTFS, SOLIDIGM SSDPFKKW020X7 2 TB NVMe'
```

请按实填写复跑机器的硬件参数。如 Job 对照无法确认一致，脚本会记录原因，并只采集可观测的进程树工作集。Actions 产物保留 21 天。过期后可复用仍与清单逐件匹配的安装版或便携版。同源码重新构建所得文件若有哈希变化，需单独登记新产物、更新两份输入收据，并按新构建记录结果。

逐轮收据包括预热、五次正式耗时、每项 Job 峰值、进程采样和产物校验。此次范围为已安装版的普通 ZIP 创建和全量解压，GUI、其他格式、已有目标覆盖、冷缓存及 ARM64 另需实测。[脱敏原始收据](measurement.json) · [测量脚本](../../../../scripts/measure-installed-zip.py)
