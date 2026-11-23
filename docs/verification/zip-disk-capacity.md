# Windows x64 普通 ZIP 真实磁盘耗尽验收

工作流 `Disposable Windows ZIP disk capacity` 在一次性 GitHub 托管 Windows x64 runner 上运行。PowerShell 和 Python 两个入口都会先检查托管环境。实际填充仅在上述托管卷执行。

每个作业独占 `RUNNER_TEMP` 下新建的目录和 96 MiB 固定 VHD，在系统盘与 backing file 所在盘以外，选择由该文件映射的新盘。脚本在初始化和格式化前核对映射目标、容量、初始状态及启动盘和系统盘标记。卷上只用本轮目录及唯一卷标和归属标记，写入前再核对已打开填充文件的卷序列号。固定容量限制了填充写入。程序、输入与临时数据同样放在本轮私有目录内。脚本最后按 backing file 路径解除映射并验证移除。准备失败见 `isolated-vhd.json` 的 `blocked` 状态。

## 用例与判定

- 创建给定 16 MiB 普通文件的 Store ZIP。验证满卷时退出码为 1，并给出明确空间不足提示。应保留输入与已有哨兵、目录清单，禁止发布半成品。移除本轮填充文件后用同一命令重试，使用 Python ZIP 独立读取成员和核对哈希。
- 用同一数据的独立 Python Store ZIP 向满卷中已有同名文件执行覆盖解压。核对相同错误条件、已存在目标与无关文件、输入归档的 SHA-256，以及目录清单。释放后用同一命令重试并核对输出字节。
- 每个用例分别记录开始时、填充后、产品返回后、释放后的卷可用字节。要求填充写入遇到原生空间不足且剩余空间不多于一块 4,096 字节簇。确认环境后才进行产品断言。
- `zip-space-receipt.json` 保存两项产品调用的退出码、错误提示、文件树及哈希和重试结果。单次操作只有在全部断言通过后才标为 `passed`。已进入产品调用并有实际断言失败的用例为 `failed`。环境未提供可核实的独立满卷时记为 `blocked`。

## 产物与复跑

| 受测组 | 预设来源提交 | 原始加固构建 |
| --- | --- | --- |
| 基线 | `51a68cc32bafcd535eac6cef8eee5169a54adcbe` | [Windows regression 45100520858](https://github.com/qing-yi-5427/qing-zip/actions/runs/45100520858) |
| 组合 | `e9eeecd414fa7379fc959967fb1645ced24eeb62` | [Windows regression 45116763549](https://github.com/qing-yi-5427/qing-zip/actions/runs/45116763549) |
| 最终交付源码 | `b2bf9f3f4eaddcbfefd3144ad0fdb8769fa1824b` | [Windows regression 45118493619](https://github.com/qing-yi-5427/qing-zip/actions/runs/45118493619) |

工作流从上一表的成功构建读取 `qing-zip-windows-x64-trial`，检查源码提交和目标架构，并据构建记录核对包内 CLI、worker 和 GUI 的 SHA-256。便携 ZIP 的 SHA-256 另记入 `isolated-vhd.json`。组合和最终交付组使用托管重建产物，二进制 SHA-256 与其他环境构建的同源码产物可能不同。

产物有保留期限。如已过期，从指向同一完整提交的远端分支或标签执行 `gh workflow run windows-ci.yml --ref <分支或标签>`，沿用默认的非扩展测试设置。取得成功运行的编号后，在已可手动调度的验收工作流按选择的组填入 `baseline_build_run`、`combined_build_run` 或 `round2_build_run`。替换只影响运行编号，固定的源码提交、工作流名称和产物验证仍适用。托管方案未覆盖其他压缩格式、真实主机系统盘、取消、安装器或特殊文件系统。

常规 PR 继续跑基线和组合两组。最终交付组按需单独启动。`gh workflow run zip-disk-capacity.yml --ref <验收分支> -f trial=round2`。

## 托管实测

基线、组合候选和最终交付组均通过。验收使用 Windows Server 2025 x64 build 26100。固定 VHD 容量为 100,663,296 字节，NTFS 卷报告总容量 99,545,088 字节。组合的首次作业等待加固包超时，未创建测试卷或启动产品。加固包就绪后只复跑该作业。[基线作业](https://github.com/qing-yi-5427/qing-zip/actions/runs/45117484883/job/137503524191) · [组合候选复跑](https://github.com/qing-yi-5427/qing-zip/actions/runs/45117484883/job/137507055057) · [最终交付组](https://github.com/qing-yi-5427/qing-zip/actions/runs/45119946039)

组合候选的精确打包工作流也已通过普通提取冲突、包内归档操作、原生 ZIP 预读和取消回归。满盘提取用例使用普通单文件覆盖。[组合构建和包内回归](https://github.com/qing-yi-5427/qing-zip/actions/runs/45116763549)

| 受测组 | 操作 | 开始可用字节 | CLI 调用前 | CLI 返回后 | 移除填充后 | 结果 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 基线 | 创建 Store ZIP | 84,779,008 | 0 | 0 | 84,779,008 | PASS |
| 基线 | 覆盖提取 | 67,997,696 | 0 | 0 | 67,997,696 | PASS |
| 组合 | 创建 Store ZIP | 87,183,360 | 0 | 0 | 87,183,360 | PASS |
| 组合 | 覆盖提取 | 70,402,048 | 0 | 0 | 70,402,048 | PASS |
| 最终交付源码 | 创建 Store ZIP | 87,183,360 | 0 | 0 | 87,183,360 | PASS |
| 最终交付源码 | 覆盖提取 | 70,402,048 | 0 | 0 | 70,402,048 | PASS |

填充写入返回操作系统空间不足（`errno 28`）。创建和覆盖提取均以退出码 1 结束并给出明确磁盘空间不足提示，覆盖提取的错误附带 Windows `os error 112`。三个受测组的所有错误均保全已有内容、输入及目录项。释放后六次同命令重试退出码为 0，输出内容与 16 MiB 输入的 SHA-256 一致。三组均已解除映射并移除 VHD 文件。

| 受测组 | 便携 ZIP SHA-256 | CLI SHA-256 | worker SHA-256 |
| --- | --- | --- | --- |
| 基线 | `871851051b0efc06f3aba6ba3b0c84cc018773cc8ca7b52a94380a1a3beb0b6d` | `738b2917a84da07d00caeb0e5292ed599c6b2ae8980b6eab8c3eb00add2cd44c` | `1033cefd2a0821b5d8776edc0b16a07b2ce668dccb9adc3ccbefc226acd02e9f` |
| 组合 | `d6e9bfe4c6a1b2c166ac2946bc242c037fd2b8d94103f23c4e521951adf74a7d` | `f975ee361a676885a13137aeb2f3d7c2ee96534957e0add1e0dc86d082e7f7e4` | `f018a69a411886bad342cdde1a05951555074bc1ed8bd712b334775cc9106c89` |
| 最终交付源码 | `c11730874d507890aaa64266563dc2a93c8d9da2c14bc64109605d435f96b98d` | `a6ec1728422d1515f510d687cb82c3de075758c940d1d974bfe7747917b2d3e0` | `1ae672ee6eb5bc623bb75a9955fa04d61e49ac3f1bda1ca79d866031b78fa708` |

## 暂存写入期间的磁盘空间耗尽

最终交付源码组还独立运行了一项覆盖提取测试。32 MiB Store ZIP 的输入置于私有运行目录，覆盖目标和程序 TEMP 都在同一隔离 NTFS 卷，启动时由独占占位文件留下约 28 MiB 空间。释放后确认至少可用 68 MiB，以便原命令完成解码和发布。[中途写入验收及收据](https://github.com/qing-yi-5427/qing-zip/actions/runs/45121013391)

监控使用只读、共享读写和删除的句柄打开卷内私有暂存文件。记录实际文件长度、卷可用空间与读出的 64 字节前缀 SHA-256。通过判定要求读到与输入相同的实际字节、观察到未完成且剩余负载大于可用空间、CLI 返回明确原生空间错误。仅记录到目录创建、看到低可用空间或收到启动前拒绝都不能通过此项。

| 字段 | 该次结果 |
| --- | --- |
| 启动时卷可用空间 | 29,360,128 字节 |
| 非零暂存数据 | 22 次增长样本，从 917,504 到 29,327,360 字节。64 字节前缀逐次与原始成员匹配 |
| 最后观测时卷可用空间 | 24,576 字节，仍有 4,227,072 字节的成员负载未写入 |
| 原生错误 | `System ERROR: There is not enough space on the disk.`，CLI 退出码 1 |
| 失败后卷可用空间 | 29,356,032 字节 |
| 释放占位后卷可用空间 | 84,774,912 字节 |
| 同命令重试 | CLI 退出码 0，完整 33,554,432 字节输出，SHA-256 `8b181db0c05ace72bd9bb05e5bdbee4661d34c509d1fbbb7d07e41a2f2ab0d79` |

失败后的原 ZIP、目标原有文件、无关哨兵及 TEMP 树与开始前完全一致，私有暂存文件未留下。释放本轮独占的占位后，原命令重试，仅预定目标变为正确的 32 MiB 内容。VHD 已卸载并移除。24,576 字节是最后一次异步观察值，实际失败依据原生错误而确认。

此模式只手动运行。使用 `gh workflow run zip-disk-capacity.yml --ref <验收分支> -f trial=midwrite`，产物过期后沿用同源码的 `round2_build_run`。原先六项启动时已满用例独立保留。
