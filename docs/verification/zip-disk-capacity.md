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

工作流从上一表的成功构建读取 `qing-zip-windows-x64-trial`，检查源码提交和目标架构，并据构建记录核对包内 CLI、worker 和 GUI 的 SHA-256。便携 ZIP 的 SHA-256 另记入 `isolated-vhd.json`。组合组使用托管重建产物，二进制与其他环境构建的同源码结果可能不同。

产物有保留期限。如已过期，从指向同一完整提交的远端分支或标签执行 `gh workflow run windows-ci.yml --ref <分支或标签>`，沿用默认的非扩展测试设置。取得成功运行的编号后，在已可手动调度的验收工作流填入 `baseline_build_run` 和 `combined_build_run`。替换只影响运行编号，固定的源码提交、工作流名称和产物验证仍适用。托管方案未覆盖其他压缩格式、真实主机系统盘、取消、安装器或特殊文件系统。

## 托管实测

基线与组合候选均通过。验收使用 Windows Server 2025 x64 build 26100。固定 VHD 容量为 100,663,296 字节，NTFS 卷报告总容量 99,545,088 字节。组合的首次作业等待加固包超时，未创建测试卷或启动产品。加固包就绪后只复跑该作业。[基线作业](https://github.com/qing-yi-5427/qing-zip/actions/runs/45117484883/job/137503524191) · [组合候选复跑](https://github.com/qing-yi-5427/qing-zip/actions/runs/45117484883/job/137507055057)

组合候选的精确打包工作流也已通过普通提取冲突、包内归档操作、原生 ZIP 预读和取消回归。满盘提取用例使用普通单文件覆盖。[组合构建和包内回归](https://github.com/qing-yi-5427/qing-zip/actions/runs/45116763549)

| 受测组 | 操作 | 开始可用字节 | CLI 调用前 | CLI 返回后 | 移除填充后 | 结果 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 基线 | 创建 Store ZIP | 84,779,008 | 0 | 0 | 84,779,008 | PASS |
| 基线 | 覆盖提取 | 67,997,696 | 0 | 0 | 67,997,696 | PASS |
| 组合 | 创建 Store ZIP | 87,183,360 | 0 | 0 | 87,183,360 | PASS |
| 组合 | 覆盖提取 | 70,402,048 | 0 | 0 | 70,402,048 | PASS |

填充写入返回操作系统空间不足（`errno 28`）。创建和覆盖提取均以退出码 1 结束并给出明确磁盘空间不足提示，覆盖提取的错误附带 Windows `os error 112`。两个受测组的所有错误均保全已有内容、输入及目录项。释放后四次同命令重试退出码为 0，输出内容与 16 MiB 输入的 SHA-256 一致。两组均已解除映射并移除 VHD 文件。

| 受测组 | 便携 ZIP SHA-256 | CLI SHA-256 | worker SHA-256 |
| --- | --- | --- | --- |
| 基线 | `871851051b0efc06f3aba6ba3b0c84cc018773cc8ca7b52a94380a1a3beb0b6d` | `738b2917a84da07d00caeb0e5292ed599c6b2ae8980b6eab8c3eb00add2cd44c` | `1033cefd2a0821b5d8776edc0b16a07b2ce668dccb9adc3ccbefc226acd02e9f` |
| 组合 | `d6e9bfe4c6a1b2c166ac2946bc242c037fd2b8d94103f23c4e521951adf74a7d` | `f975ee361a676885a13137aeb2f3d7c2ee96534957e0add1e0dc86d082e7f7e4` | `f018a69a411886bad342cdde1a05951555074bc1ed8bd712b334775cc9106c89` |
