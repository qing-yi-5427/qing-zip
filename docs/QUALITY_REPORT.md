# 0.12.23 质量报告

2026-11-23 以本地提交 `0cc51aa` 建立基线。复测使用独立工作树、便携构建以及临时数据。以下优先级依据用户能观察到的结果。一个样本通过，只计入该样本覆盖的格式、行为与平台。

## 需要改进的已实现功能

| 优先级 | 用户能观察到的结果 | 复现与验收 |
| --- | --- | --- |
| P2 · 解压自动改名 | 目标已有 `photo.txt`，包内同时有 `photo.txt` 和 `photo (1).txt`。旧版前一项抢占编号名，后一项变为 `photo (1) (1).txt`。 | 真实 0.12.23 CLI 已复现。验收时检查原有文件、每个输出的内容、选定范围和隐式父目录。 |
| P2 · 同名目录 | 目标已有目录 `photo.txt` 时，用户选择跳过或自动改名，旧版仍以冲突中止，包内其他文件也无法落盘。 | 在已有目录中预置文件，分别检查跳过、改名、覆盖和遇错停止的结果；最后核对目录内原有内容和源 ZIP。 |
| P2 · Explorer 打包 | x64 组件能被打成标记 ARM64 的稀疏 MSIX，构建工具报告成功。此包进入签名或安装阶段会浪费验证时间。 | 已在 x64 本机重现，候选修复分别检查 DLL 和 host。x64、原生 ARM64 均完成组件测试与正反向打包验收。[修复 PR](https://github.com/qing-yi-5427/qing-zip/pull/1) · [双架构运行记录](https://github.com/qing-yi-5427/qing-zip/actions/runs/45096359747) |
| P3 · 短时菜单材质 | 实际 Acrylic 窗口上，主菜单在浅色模式为不透明白底，和邻近浮层的材质层级不一致。 | 以同一便携应用、主题、窗口尺寸和输入对照修改前后，查看可读性、键盘、实色回退和菜单响应。 |

前两项在候选版本中已修复。两条新增 CLI 集成测试均在原版失败、候选通过。在真实 Windows 应用里，自动改名的样本完成后任务显示 3 个文件；另一个样本先提示目标冲突，再从任务中选择跳过重试，成功提取其余 2 个文件。核对了目标原有内容、提取文件的 SHA-256 和源 ZIP。覆盖目录和将文件当作父目录仍会安全停止。[解压修复及测试](https://github.com/qing-yi-5427/qing-zip/pull/2)

## 普通归档的跨工具对照

**PASS，7 个格式 × 3 个程序。** Windows 11 x64 build 26300，qing-zip 0.12.23 加固便携构建、系统已安装的 7-Zip 26.00 x64 及 Bandizip 7.36 Beta x64。每次解压到空的临时目录。比较相对文件名和每个文件的 SHA-256，每次读取后也复核输入归档的 SHA-256。

| 样本 | 结果 |
| --- | --- |
| ZIP Deflate、7z、POSIX TAR | 三种工具均得到两个目录下的同名文件和一个空文件；中文目录、名称和内容均吻合。 |
| GZIP、BZ2、XZ | 三种工具都将普通单文件流还原为 `payload.txt`，内容为已知输入字节。 |
| RAR5 m3 | 使用仓库附来源与许可的普通测试包。三个程序均输出 `bigtext_64k.bin`，65,536 字节，SHA-256 为 `ed70a3812333daa70dfdbc167cb009049fd6aed3aeef0902ad5326d245892209`。 |

复跑示例，最后一个参数可省略。脚本只在新建临时目录生成和提取测试材料。

```powershell
python scripts/compare-common-extraction.py --cli <便携目录>/obz.exe --sevenzip <7-Zip目录>/7z.exe --bandizip <Bandizip目录>/bz.x64.exe
```

**PASS · 经典 RAR 单卷和两卷。** 同一台 Windows 11 x64 上另取有来源记录的 RAR 1.5 压缩样本和 RAR 3.00 两卷样本。从首卷 `.rar` 提取，三款程序均自动读取旁边的 `.r00`。单卷得到 21 字节的 `payload.txt`，两卷得到 65,536 字节的 `bigtext_64k.bin`。两者都比对了上游已知的原文或原文件 SHA-256，每款程序退出后还逐卷核对来源哈希。本轮总计 9 组正常样本、27 次提取。[样本及许可来源](../obz-rs/tests/fixtures/deep/rar15/SOURCE.json) · [逐样本结果](verification/common-rar-results.jsonl)

**FAIL · 已确认的经典 RAR 分卷兼容差异。** 上游 `rars_generated/split-store` 三卷可由 RARLAB 官方 UnRAR 7.30 beta 2 x64 正常还原为已知的 21 字节原文，退出码为 0。qing-zip 返回 1，目标目录为空。7-Zip 26.00 报 CRC 错误并留下 21 字节文件，Bandizip 7.36 Beta 返回 2 且目标为空。qing-zip 随包的两个解码器在各自独立的完整性检查中也报告了 CRC 错误。该样本单列为待修差异；修复需要验证分卷完整性与输出 CRC。原包各卷哈希在每次运行后均一致。[上游样本](https://github.com/bitplane/rars/tree/c07b3e68b564489202afc0ac9fa1ab5ff82db6a6/crates/rars/tests/fixtures/rar15_40/rars_generated) · [四程序实测结果](verification/rar-reference-difference.jsonl)

参考工具取自 RARLAB 免费 Windows UnRAR 包，下载包的 Authenticode 状态为 Valid，签名者为 win.rar GmbH，解包后的 UnRAR.exe SHA-256 为 `ad9f3ae3210f987fed43fbc7dc343ee95e9ce6b03ce24c635bb8d3162f0b9308`。对照工具独立运行。增加 `--unrar <UnRAR.exe>` 可验证前述三个常规 RAR 样本（共增加 3 次 PASS）。同时传入 `--include-rar15-split` 可复跑有差异的三卷，脚本记录每款工具的退出码、已写出文件和状态，有失败时最终返回 1。[官方工具来源](https://www.rarlab.com/rar_add.htm)

**SKIP** · WinRAR 桌面程序本轮未安装。各格式的其他压缩方法、旧 RAR 加密、缺卷及其他命名方式仍需分别验证。[测试矩阵](TEST_MATRIX.md)

**PASS，未完成下载。** 便携包冒烟测试从正常 ZIP 和 7z 各复制前一半字节到新文件。完整性检查以及选了覆盖的解压操作均返回可处理的错误，已有目标文件、目录清单、原包及未下载完的副本逐字节保全。[可复跑的包内测试](../scripts/check-public-package.py)

## 文件安全审查范围

源代码中已有相对路径校验、包括隐式目录在内的名称冲突检查，以及解压输出前的文件系统检查。M2 的名称保留逻辑使用既有名称比较规则，目标仍经过原有检查。深度解压按层数、归档数、文件数和解码量记账，原始流另有写入时的空间检查。上述属于代码审查和普通回归覆盖。涉及链接、特殊文件系统或资源耗尽时的实际磁盘行为仍按测试矩阵标记，尤其不能从内存预算测试推出所有引擎临时写入量的上限。[路径校验](../obz-rs/src/extract.rs) · [解压发布](../obz-rs/src/engine.rs) · [预算与恢复边界](deep-extraction.md)

## 下一项高价值验收

同一组样本分别经主窗口、首页、批量、深度解压及 Explorer 进入，记录预告路径、实际路径、冲突后的可恢复内容、任务中心可点击的下一步。优先覆盖单顶层同名目录、已存在的目标和选择性提取。签名一级菜单、Windows 10 桌面回退、真实系统高对比度以及存储故障应在具备对应条件时单列验收。[产品路线](OPTIMIZATION_ROADMAP.md)
