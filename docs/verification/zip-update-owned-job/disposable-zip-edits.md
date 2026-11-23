# 普通 ZIP 改名与删除的中断候选回收

## 适用范围

本机 NTFS 上的经典、单卷、明文 ZIP，仅含 Store 或 Deflate 文件成员时，删除或改名可以让随包 NanaZip 将候选写入预先拥有的文件。原归档目录清单仍先检查名称和重名。启动写入前，以发布 provenance 核对控制台和五个依赖的 SHA-256，并固定这六个文件到写入器退出。这个只读核对与原包只读列表并行。备用 7-Zip、不能完成来源核对的引擎、加密 ZIP、显式编码、ZIP64、其他编码方法及带跨会话恢复收据的编辑均沿用原输出路径。

候选文件由本次操作在已固定的私有输出目录中独占新建。写入器只获得这个对象的写入句柄。设置删除标志后才启动既有专属 Job 的写入进程。正常退出且读取完引擎结果后，控制端撤销标志，再执行既有 ZIP 完整性校验和原包提交。取消、写入错误或写入期间强制结束应用，会在最后一个文件句柄关闭时回收该对象。写入成功后若完整性校验失败，正常清理候选并保留原包。 [Windows 文件删除标志](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_disposition_info)

进程强退仍可能留下空的私有目录和小型诊断文件。撤销删除标志至原事务登记之间，若再次强退，已完成的候选仍可能留下。发布、回滚或跨会话嵌套恢复使用各自已有保护。历史遗留目录缺少可核实的早期候选归属，保留原状。 [原设计与保全边界](../abandoned-edit-design.md)

## 真实中断和重试

Windows 11 x64 build 26300、Ryzen 7 9800X3D 与普通 NTFS。基线为本机安装的 `14b9e2a8a5c2de35dea31acf1f222ad10ca354f0`，候选为产品提交 `76feea4535fe1f0b09c6695a8bff1dc126c01872` 的本地 x64 加固 CLI 及 worker，两者通过 PE 门禁。基线和候选均使用 NanaZip 7.0.1882.0，其 SHA-256 为 `06070ca08b3d10e760e8e394a1b6c8ad751aff89f759227ec111b878c12db139`。候选 worker 的 SHA-256 为 `720bf3e744f406791c33a2de8bd462ca134c1bbc8321c48cf6454bcceba3a41e`，CLI 与脚本哈希见逐次回执。

每轮安排真实 256 MiB 普通 ZIP 的改名或删除，和另一项 64 MiB 独立创建。看到指定 ZIP 写入进程已读取源并发出非零写入后，结束其上级 CLI。近实时信息来自精确的 Windows 进程句柄，写入量含该进程所有写入请求。带删除标志的候选在路径查询中通常无法读取当前长度，产物是否残留以写入进程退出后的实际目录为准。每轮比较原包和来源 SHA-256、独立创建任务内容，并从同一原包成功重试并核对成员内容。

| 操作 | 基线三轮原包旁残留 | 候选三轮原包旁残留 |
| --- | --- | --- |
| 改名 | 124,780,544 / 171,966,464 / 176,160,768 字节 | 0 / 0 / 0 字节 |
| 删除 | 156,237,824 / 147,849,216 / 155,189,248 字节 | 0 / 0 / 0 字节 |

原包、输入和独立任务均通过，重试成功，重开及重试后结果一致。六轮候选的写入进程在上级退出采样后的 5.4–6.5 ms 内退出，采样后无写入增长。本机定向 Rust 实测还包括取消重试、只回收本次独占创建的候选、候选校验失败保全原包及嵌套归档第二次保存。 [改名基线](rename-baseline.jsonl) · [改名候选](rename-candidate.jsonl) · [删除基线](delete-baseline.jsonl) · [删除候选](delete-candidate.jsonl)

## 互操作与耗时

原生同一控制台的具名目录与句柄输出在 1 KiB 单成员、16 MiB 多成员的改名样本中逐字节相同。独立 .NET `ZipArchive` 与 Python `zipfile` 逐项验证时间、CRC32、解开长度和 SHA-256。另外比较普通删除、含中文文件名、原包与成员注释、未知标准扩展区、显式 CP936、AES-256 和输入不存在的错误。输入缺失时引擎虽写了 328 字节，非零退出后未留下候选。文件最后一个成员的删除控制也生成同样可读的空 ZIP。 [兼容与计时原始回执](zip-output-timing.json)

`add` 走原具名输出。直接实测无压缩收益的 16 MiB 新成员会由具名路径的 Store（16,777,216 字节）变成顺序输出的 Deflate（16,784,666 字节）。再加 16 字节数据描述符，总包会大 7,466 字节。两种结果均能读回同样内容，顺序输出改变默认方法。候选的三轮 `add` 强退仍分别留 2 / 3 / 3 MiB，保全原包和重试均通过。同一候选使用独立 7-Zip 26.00 时，三轮改名也走原具名输出，保全原包和重试均通过。后续若以同一引擎的可 seek 输出实现加入路径，应先打通原包保留与同一压缩方法的真实差分验收。 [7-Zip ZIP 编码器的输出方法选择](https://github.com/ip7z/7zip/blob/main/CPP/7zip/Archive/Zip/ZipAddCommon.cpp) · [加入控制](add-named-control.jsonl) · [备用 7-Zip 控制](rename-7zip-fallback.jsonl)

在同一台机器的温暖文件缓存中，每种测量先预热一对，再交替五对。每次使用新复制的同一夹具，准备和事后核对在计时外。新 CLI 进程从启动计时至退出，包含六个组件的来源核对、原包列表、写入、候选完整性检查和提交。同进程通过独立编译两版的 Rust 驱动直接计量 `engine::update`，每版复用一个驱动进程，原生子进程照常启动。样本期间未见并发编译或其他验收任务。

| 改名样本 | 基线中位数 | 候选中位数 | 候选减基线的成对中位数及范围 |
| --- | ---: | ---: | ---: |
| 新 CLI 进程，1 KiB | 94.228 ms | 94.666 ms | −1.236 ms（−8.798 至 +13.251） |
| 新 CLI 进程，16 MiB | 145.121 ms | 154.717 ms | +4.222 ms（−4.950 至 +10.636） |
| 同进程，1 KiB | 62.968 ms | 58.628 ms | −4.340 ms（−9.506 至 +11.887） |
| 同进程，16 MiB | 108.591 ms | 116.445 ms | +7.454 ms（−6.101 至 +8.489） |

小包的差异在观察波动内。16 MiB 增加数毫秒，包大小保持相同。只测原生引擎加候选句柄时，大档成对中位数为 +2.080 ms（+0.959 至 +3.597），这一口径只用于定位成本。

## 复跑

在带 NanaZip 运行时和 `obz-worker.exe` 的 x64 便携目录上执行。强退驱动依赖 Python 和 `psutil`。互操作驱动另需实现 `.NET ZipArchiveEntry.Crc32` 的 `pwsh.exe` 及独立 7-Zip。所有回执路径须尚未存在。

```powershell
python scripts/check-interrupted-archive-update.py --cli $candidate --action rename --require-owned --require-reclaimed --out target/rename-candidate.jsonl
python scripts/check-interrupted-archive-update.py --cli $candidate --action delete --require-owned --require-reclaimed --out target/delete-candidate.jsonl
python scripts/probe-zip-update-stdout.py --nana $nanaConsole --reader $independent7z --baseline-cli $baseline --candidate-cli $candidate --out target/zip-output-timing.json
```

同进程计时从对应源码构建两版 Release `obz_core`，用同一 Rust 工具链将 [`probe-zip-rename-process.rs`](../../../scripts/probe-zip-rename-process.rs) 链接至各自的 `libobz_core`，编译时使用 `--extern obz_core=<rlib>`、`-L dependency=<target>/release/deps`（显式指定目标架构时还需架构对应的 `release/deps`）、`-C opt-level=3 -C lto=fat -C target-feature=+crt-static`，把两个探针各放在对应便携目录中。调用 Python 脚本时追加 `--baseline-process` 与 `--candidate-process` 对应文件路径。冷磁盘缓存、ARM64 上的实际强退、掉电、在提交流程中逐阶段强退和特殊文件系统均未在本机实测。
