# 普通归档解压后的修改时间

Windows 11 x64（build 26300）上的已安装 qing-zip 0.12.23 通过五种普通归档的整包和单文件解压，共 10 项。独立 7-Zip 26.00 在同一个 ZIP 上通过两项对照。 [逐项回执](installed-x64.json)

已安装的 `obz.exe` SHA-256 为 `4b8a992064ad4d5aa80ab421cf0e2baae8305cab9dd46832d1a4efa0a33d3d33`，与主线 `be86c3560cb56e83efaa66b3a18a8d18943754e1` 的托管产物内 `target/hardened/x86_64-pc-windows-msvc/release/hardened-build.json` 中 `obz.exe` 一致。便携 ZIP SHA-256 为 `97a2fc4e963ae910609e3a9eb53a7ad8e0abe699b3dc4e512f5fee70d6b7ec66`。 [主线](https://github.com/qing-yi-5427/qing-zip/commit/be86c3560cb56e83efaa66b3a18a8d18943754e1) · [托管运行](https://github.com/qing-yi-5427/qing-zip/actions/runs/45277441627) · [包含硬化清单的 x64 产物](https://github.com/qing-yi-5427/qing-zip/actions/runs/45277441627/artifacts/14868574888)

| 归档 | 实际整包与选取结果 |
| --- | --- |
| ZIP | 两个普通文件、显式父目录和空目录按 ZIP 记录的本地时间匹配至秒。单选只发布 `folder/notes.txt` 及必要的父目录，选中文件时间匹配至秒。7-Zip 对照通过同样的断言。 |
| TAR、TAR.GZ、TAR.BZ2、TAR.XZ | 四种 USTAR 归档均以整秒 Unix 时间验证相同成员和目录。单选只发布同一文件及必要的父目录，选中文件时间匹配至秒。 |

四个成员的时间各不相同。每次核对实际文件与目录的完整名单、所有已输出文件的 SHA-256 以及解压前后的原包 SHA-256。单选时只核对父目录存在，修改时间检查仅覆盖选中的成员。此次只核对到秒，未测亚秒精度、归档创建时间、访问时间、夏令时转换、合并现有目录或加密归档。

## 复跑

从仓库根目录运行。使用 Windows Python 标准库、已安装的 `obz.exe` 和独立 `7z.exe`。脚本在独立临时目录生成普通夹具，结束时清理，JSON 输出应是新路径。

```powershell
python scripts/check-ordinary-extract-mtime.py --cli $installedObz --sevenzip $sevenZipExe --source be86c3560cb56e83efaa66b3a18a8d18943754e1 --out $newReceipt
```
