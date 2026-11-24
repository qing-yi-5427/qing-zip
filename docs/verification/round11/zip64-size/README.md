# 普通 ZIP64 的 4 GiB 边界

Windows 11 x64 26300、NTFS、Python 3.14.0。使用 0.12.23 的已安装 CLI 和 worker。安装文件匹配产品提交 `3bd150c` 的 x64 构建，CLI 的 SHA-256 为 `8b2c94be2232b91089502d18310d17b76c08326182055e1b955c09d610545cf2`。所有输入和目标都是临时目录中的普通文件。[构建记录](https://github.com/qing-yi-5427/qing-zip/actions/runs/45290034274)

| 实际操作 | 检查结果 |
| --- | --- |
| 列出、校验、解压一个大成员 | **PASS**。用 Python zipfile 创建三个单成员 Deflate 归档，成员分别为 4,294,901,759、4,294,967,296 和 4,295,032,833 字节。检查 ZIP64 尺寸字段，将实际输出与源文件、独立读取器做全流 SHA-256、CRC-32、长度对照，并复查源文件及归档。 [原始回执](member-x64.json) |
| 只提取本地头位于 4 GiB 之后的小文件 | **PASS**。两个 STORE 成员自身的大小均可用 32 位表示。第二个成员的本地头偏移为 4,294,967,303，中央目录偏移为 4,294,967,402。程序正确列出了两项，单选只产出预期的 51 字节，源归档哈希保持。独立读取器读到了两个成员末尾。 [原始回执](offset-x64.json) |
| 创建含超大成员的归档，再追加普通文件 | **PASS**。使用程序创建含 4,294,967,330 字节文件的 STORE 归档，再加入 39 字节文件。每次写入后，Python 重新读取全部内容并核对 CRC、SHA-256 和成员集合。追加后两个源文件保持原样。 [原始回执](writer-x64.json) |

第一组的“略小于 4 GiB”样本同样由独立写入器生成 ZIP64 尺寸字段。本组验收普通、单卷、未加密的归档。加密、分卷、写回期间异常中断和断电另行验收。回执中的耗时为当次观测。

## 复跑

使用 64 位 Windows、Python 3.14 和完整的 x64 安装或便携目录。在已有的本地测试目录预留至少 24 GiB，依次运行以下三项。脚本各自创建唯一临时子目录。将三个新回执写在临时样本之外。

```powershell
$cli = Join-Path $env:LOCALAPPDATA 'Programs/qing-zip/obz.exe'
$scratch = (Get-Item .).FullName
python member.py --cli $cli --temporary-parent $scratch --out member-run.json
python offset.py --cli $cli --work-root $scratch --source '<实际构建提交>' --out offset-run.json
python writer.py --cli $cli --scratch-parent $scratch --receipt writer-run.json
```
