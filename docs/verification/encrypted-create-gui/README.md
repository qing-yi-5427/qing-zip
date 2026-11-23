# 实际窗口创建加密归档

Windows 11 x64 build 26300 上，以已安装的实际 Tauri/WebView2 程序分别创建一个 7z 和一个 ZIP；两个测试均以 `--compress-files` 传入普通合成文件、在私有非输入 Win32 桌面启动，并使用独立的应用配置。通过 CDP 操作窗口和截图；没有观察系统前台或物理键盘输入。GUI 的 SHA-256 为 `0200fee7c28d0e5b68bff2f91d6a2f07a43f9e85eece4fd1a875c6cc74a48dbe`，对应产品源码 `be86c3560cb56e83efaa66b3a18a8d18943754e1` 的 [x64 run 45277441627](https://github.com/qing-yi-5427/qing-zip/actions/runs/45277441627)，便携 ZIP SHA-256 为 `97a2fc4e963ae910609e3a9eb53a7ad8e0abe699b3dc4e512f5fee70d6b7ec66`。后续 `f409723` 仅合入验收文档。

在第一窗口先填写随机密码，从 ZIP 切到 TAR，实际提交出现[不能使用密码的提示](unsupported.png)，没有创建 TAR；再切到 7z，密码保留且[加密文件名已勾选](7z-form.png)。第二个新窗口创建 [AES256 ZIP](zip-form.png)。两次均等待各自的“检查通过”，正常关窗，并核验源文件与旁边哨兵哈希不变。截图遮蔽可见路径与密码输入框。

使用独立的 7-Zip 26.00 检查结果：7z 的空密码和错误密码均不能列出名称；正确密码可列出并解出唯一文件。ZIP 文件名在未输入密码时仍可见；其加密标志、方法 99 及 AES 扩展字段 `0x9901` 的 256 位强度均经 ZIP 解析器检查。两种格式的空密码或错误密码提取均失败且没有非空文件；ZIP 在两次失败中各留下一个零字节占位文件。正确密码只得到预期文件且 SHA-256 等于来源。[机器回执](receipt.json)

这仅覆盖一次普通文件的两种归档和所列格式切换，并非全部格式、并发或攻击输入的验收。每次生成新的随机合成密码，不存入回执或截图。独立 7-Zip 读取时将该临时密码作为短时子进程参数传递，驱动不打印其命令或输出；这只适用于此测试，不供真实用户密码使用。临时配置及样本在程序退出后清理。

复跑需要 Windows、Python Playwright、pywin32 和独立 7-Zip，使用已核验的产品包并确认没有其他 qing-zip 进程：

```powershell
python scripts/check-create-encrypted-gui.py --exe <qing-zip.exe> --reference <独立7z.exe> --fixtures-in <独占临时根> --out <不存在的证据目录>
```
