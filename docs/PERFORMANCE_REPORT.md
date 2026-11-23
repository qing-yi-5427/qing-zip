# Windows 性能基线

2026-11-23 · qing-zip 0.12.23 · 源码 `0cc51aa2379321361012f32515edb40b95e2ebf9`。

这份基线记录 Windows x64 加固便携包中 `obz.exe` 的归档操作，以及工具箱所调用的 LZMA 合成基准。四类样本全部完成创建、列出、校验、解压，源目录、源归档与解压结果通过目录树和 SHA-256 比较。

## 环境与计时

- Windows 11 Pro x64 build 26300，AMD Ryzen 7 9800X3D（8 核 / 16 线程），48 GiB RAM，Solidigm SSDPFKKW020X7 2 TB NVMe。便携程序在 `G:`，合成输入与输出由 Python 在同一物理 SSD 的 `%TEMP%` 下创建。
- Release 目标为 `x86_64-pc-windows-msvc`，Rust 1.99.0。宿主 Python 3.14.0，包内 NanaZip 7.0.1882.0，独立互读使用包内官方 7-Zip 26.06。
- 测量用 `obz.exe` SHA-256 为 `b0f30fb6440caf0bd98cb67fc448adfcdaa6cf4903f79c20f310584ce09277f9`。整包 SHA-256、PE 审计范围与实跑命令见 [测试矩阵](TEST_MATRIX.md)。
- 每种数据先预热一轮，再串行测五轮。每项 CLI 操作启动一个进程，使用新归档路径和新解压目录。源数据在各轮复用，Windows 文件缓存保持自然状态，归档哈希在列表前读取。计时从 Python 发起进程到返回并接收完输出，包含子进程启动、引擎、文件 I/O 和 CLI 响应传输。JSON 校验、样本生成和 SHA-256 比较在计时外。
- 这里的每次启动都使用新 CLI 进程。真正的磁盘冷读、前台首次可交互、WebView2 内存和 GUI 预览尚未纳入本轮计时。

## 合成文件的实际归档操作

ZIP 使用 Deflate 等级 5，7z 使用固实 LZMA2 等级 5。创建、校验和解压传入 `--threads 4`，各后端实际并行度由引擎决定。表中耗时为中位数（最小–最大），单位毫秒。首组采样期间有其他 Rust 编译运行，系统并行负载计数中间三次为 51%–67%。

| 样本 | 文件数 / 原始字节 | 包字节 | 创建 | 列出 | 完整性检查 | 全量解压 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 中文、空目录的微小 ZIP | 2 / 896 | 780 | 185（109–938） | 84（76–292） | 236（86–311） | 120（100–418） |
| 可压缩的千份小 CSV ZIP | 1,024 / 4,180,906 | 245,458 | 171（158–675） | 77（72–310） | 107（102–422） | 900（808–1,692） |
| 四份各 8 MiB 伪随机文件 ZIP | 4 / 33,554,432 | 33,555,164 | 319（257–341） | 73（68–77） | 101（90–104） | 99（94–119） |
| 文本与伪随机数据混合的固实 7z | 64 / 7,010,304 | 4,238,959 | 265（253–282） | 67（63–71） | 126（121–133） | 171（169–182） |

千份小文件的解压慢于 32 MiB 四文件样本。千文件首轮预热耗时 3,613 ms，五个后续样本仍跨 808–1,692 ms。先复测多文件解压，再用阶段计时定位每文件发布的开销。微小 ZIP 也受共享主机干扰，当前区间不足以判定微小的版本差异。

## LZMA 工具箱基准

用同一便携包执行 `obz json` 的 `benchmark` 操作。引擎参数为 4 线程、字典指数 20（1 MiB）、每次 1 轮。首次预热从统计中排除，下表为后续 5 次的中位数及范围。速度和评分来自 NanaZip 的 `Avr:` 报告，适用于引擎生成数据。

| 指标 | 中位数（最小–最大） |
| --- | ---: |
| 压缩速度，KiB/s | 39,672（39,004–43,339） |
| 解压速度，KiB/s | 480,904（476,142–484,243） |
| 压缩评分，MIPS | 36,156（35,547–39,498） |
| 解压评分，MIPS | 39,780（39,386–40,056） |
| 引擎用时，秒 | 4.336（4.255–4.346） |

## 复现与后续采样

```powershell
$build = ./scripts/build-hardened.ps1
$output = Join-Path $PWD ('dist/quality-' + [guid]::NewGuid())
python scripts/package.py --binary-directory $build.BinaryDirectory --output-directory $output
$receipt = Join-Path $env:TEMP ('qing-zip-perf-' + [guid]::NewGuid() + '.json')
python scripts/check-archive-interop.py `
    --cli "$output/qing-zip-0.12.23-portable/obz.exe" --rounds 5 --receipt $receipt
```

脚本生成固定数据的普通文件样本，记录每轮耗时、预热、范围、归档大小和互读 SHA-256。省略 `--rounds` 时仅运行互读。查看 [归档互读与测量脚本](../scripts/check-archive-interop.py)。

仓库的前台基准使用 `compare-final-desktops.py` 与 `bind-final-desktops.py`，绑定候选包、参考程序和固定 ZIP，以可见 UI Automation 内容为就绪口径，并采样进程树内存。该脚本限定可丢弃的 hosted Windows 环境，本轮标记 `NOT RUN`。吞吐归档样本和合成引擎输出均使用本机 CLI。查看 [前台测量脚本](../scripts/compare-final-desktops.py) 与 [输入绑定脚本](../scripts/bind-final-desktops.py)。

回归比较时固定数据、线程、档案设置、存储位置与系统负载，交替执行两个精确版本，同时报告范围及预热。GUI 启动、预览与内存单独比较，在相同 DPI、显示状态与 WebView2 版本下记录从操作到可交互的耗时及进程树私有提交内存。
