# 普通 ZIP 更新异常退出时停止写入

基线为 R2 的 `b2bf9f3`。候选为 [PR #28](https://github.com/qing-yi-5427/qing-zip/pull/28) 的 x64 加固 CLI，SHA-256 为 `07837c957b93577bb1336fd80f3fb2243e573dd9217293f8a30cadbe6bd3ffeb`。双方使用同一 NanaZip Console，SHA-256 为 `06070ca08b3d10e760e8e394a1b6c8ad751aff89f759227ec111b878c12db139`。

## 适用范围

- Windows x64 和 ARM64 版本在创建普通 ZIP 的候选写入进程时，以 `PROC_THREAD_ATTRIBUTE_JOB_LIST` 将进程加入本次更新的无名 Job。应用进程退出并关闭最后一个 Job 句柄时，Windows 终止关联进程。Job 句柄不可继承。若创建或绑定失败，本次更新返回错误。[Microsoft 进程属性](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute) · [Microsoft Job 对象](https://learn.microsoft.com/windows/win32/procthread/job-objects)
- 范围是现有 `EditSource::direct_zip` 识别的候选写入，包括普通 ZIP 增加、同步、更新、删除、改名，以及使用同一更新入口的归档内新增和移动加入。嵌套归档写回至外层的独立引擎调用、其他格式和编辑意图恢复均沿用既有路径。
- `HANDLE_LIST` 只把三个临时标准输入输出句柄交给这次 ZIP 引擎。同一进程中走 Rust `Command::spawn` 的归档引擎、辅助进程和窗口的分卷确认、文件定位共享一把只在启动期间持有的锁。使用自身句柄清单的原生工作器继续使用各自清单。WebView2 等第三方代码若在同一进程中另行发起允许全面继承的启动，仍可能取得当时临时可继承的标准输入输出句柄。操作的 Job 句柄在此期间也不可继承。 [Microsoft CreateProcessW 句柄继承规则](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
- 微软文档列出 Windows 10 即支持 `JOB_LIST`，覆盖项目标明的 x64 最低版本 Windows 10 2004（19041）。本次本机复测用 Windows 11 build 26300。19041 未实机验收。[项目系统要求](../../../README.md) · [Microsoft 进程属性](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)

## 真实 Windows 中断对照

每轮用普通临时 ZIP、128 MiB 可重复输入和 64 MiB 独立 ZIP 创建任务。等待两项普通引擎都在写入后，终止正在更新的应用进程。比较父进程退出后首个采样与写入进程退出时的累计写入量。数值来自同一个 Windows 进程句柄，包含该进程所有写入请求。

| 构建 | 轮次 | 采样后的进程退出用时 | 采样后写入增长 | 原包旁保留的候选 |
| --- | ---: | ---: | ---: | ---: |
| [R2 基线](baseline.jsonl) | 3 | 2.0044–2.0503 s | 261,096,869–262,145,445 字节 | 每轮 134,218,001 字节 |
| [R3 候选](candidate.jsonl) | 3 | 1.0–1.5 ms | 0 字节 | 2 MiB |
| [R3 候选加普通外层 Job](candidate-outer-job.jsonl) | 3 | 1.4–1.7 ms | 0 字节 | 2–3 MiB |

每轮重新打开原包均只见原有成员，原 ZIP 与输入 SHA-256 保持一致。并行创建的 ZIP 内容校验通过，重试后的原成员与新增成员均匹配预期。基线启用严格门禁时另有一轮因写入进程多写 263,194,021 字节而失败。

本机用真实引擎的定向取消测试确认写入进程属于专属 Job、Job 句柄不可继承、同进程另一线程可在写入引擎回收前获取启动锁。取消返回时已回收写入进程，原 ZIP 和输入保持完整，取消的工作区已清除。含中文和空格的同一路径重试成功。现有 ZIP、AES ZIP 和 7z 更新模式矩阵通过。

强制退出仍留下已经写出的候选。此次试验没有覆盖历史候选清理、提交与回滚交接、机器断电、Windows 10 实机或特殊文件系统。 [早期编辑残留设计](../abandoned-edit-design.md)

## 普通更新耗时

两个 x64 加固 CLI 使用同一外部引擎。每一组先预热，配对的执行顺序交替。计时覆盖完整的普通 ZIP `add` 命令，输入准备和 ZIP 成员校验在计时外。所有轮次均核对输出成员和来源 SHA-256。[配对原始回执](timings.json)

| 新增成员 | 配对数 | R2 中位数 | R3 中位数 | R3 减 R2 的配对中位数 |
| --- | ---: | ---: | ---: | ---: |
| 1 KiB | 12 | 93.910 ms | 93.678 ms | +0.537 ms |
| 16 MiB | 8 | 400.567 ms | 400.990 ms | +1.282 ms |

这两组样本的配对差值分别为 −4.152 至 +9.676 ms 和 −8.753 至 +15.487 ms。结果仅代表所述普通 ZIP 更新负载。

## 复跑

在 PowerShell 中运行。提供带完整 NanaZip 运行时的两份便携目录，以及尚未存在的回执路径。本次使用 Python 3.14.0、psutil 7.2.2。严格门禁要求在父进程退出采样后无写入增长，并在 1 秒内回收已观测到的引擎进程。

```powershell
python scripts/check-interrupted-archive-update.py --cli $oldPortableCli --out target/r2.jsonl
python scripts/check-interrupted-archive-update.py --cli $newPortableCli --require-owned --out target/r3.jsonl
cargo test --locked --manifest-path obz-rs/Cargo.toml --lib engine::owned_update::tests::ordinary_zip_cancel_reaps_its_bound_engine_before_returning -- --exact
```
