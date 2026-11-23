# ZIP 更新在写完后和替换期间中断

Windows 11 x64 build 26300，普通本地 NTFS 临时目录。使用主线产品提交 `c52c4ce` 的已安装 CLI，SHA-256 为 `e5b87919d19ea75f19f93ea04ebb20fb366b5b8176bc6f67b84c54f119faa520`。测试只主动终止刚为用例启动的编辑进程。

| 中断位置 | 原包和新包 | 私有候选 |
| --- | --- | --- |
| 普通 ZIP Add，新的中央目录可独立读出、原包尚未替换 | PASS。三轮实际终止安装版 CLI。每轮原包字节保持，重新校验成功，重试后的所有成员、原始输入和邻近文件正确。 | FAIL。三轮在退出、重开和重试后均留有 1,561,325 字节，全部成员的 SHA-256 与预期新包相符。[三轮实际文件](completed-candidate-c52c4ce.jsonl) |
| 原包刚移到私有目录，或候选已准备公开、还未取得原名 | PASS。两个位置都正式终止了运行产品提交代码的测试子进程。分别用安装版 CLI 打开，原名空闲时自动恢复原包，字节一致。如果另一份 ZIP 已取得原名，则读到那份外部文件，旧原包仍在私有目录。 | 如实保留恢复结果。正常找回后每轮还存有一份私有 ZIP，有独立同名文件时保留两份。此检查验证可恢复性及文件归属。[四种文件结果](commit-recovery-c52c4ce.json) |

## 复跑

`scripts/check-completed-zip-candidate.py` 以普通公开入口更新 ZIP。观察者关闭自己的读取句柄后才终止进程，再独立核对新旧内容。若构建从未公开可读的候选文件，此观察方法无法命中该边界，需要使用其他可验证的阶段信号。加入 `--require-reclaimed` 会记录三轮后按回收要求返回失败或成功。

第二项复用源码已有的测试检查点。测试子进程在运行真实提交代码时停住，随后由测试驱动终止。开包与校验均使用传入的产品 CLI。原包争用用例使用独立生成的普通 ZIP。每个用例使用自己的临时目录。机器断电、整机重启、其他格式和其他文件系统仍需独立验收。

```powershell
python scripts/check-completed-zip-candidate.py --cli $cli --out $newJsonl --require-reclaimed
cargo test --manifest-path obz-rs/Cargo.toml --lib --locked --no-run
python scripts/check-ordinary-commit-recovery.py --cli $cli --harness $obzCoreTestExe --out $newJson
```

`$obzCoreTestExe` 使用上一个 Cargo 命令输出的测试程序路径。两个输出路径必须是新文件。
