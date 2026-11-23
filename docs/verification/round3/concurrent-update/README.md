# 普通 ZIP 同进程并发更新

主界面的常规任务用 `s.active` 阻止同时提交，Tauri 原生任务注册器最多允许两个运行项，并为每项启动工作线程。此次抽查由一个原生进程中的两个线程同时调用生产 `obz_core::engine::update`。线程采用现有并发测试的屏障模式。两个任务使用不同的普通 ZIP，避免同一归档路径的写锁让它们串行。 [主界面与任务注册器](https://github.com/qing-yi-5427/qing-zip/tree/fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6/obz-gui) · [现有同归档回归](https://github.com/qing-yi-5427/qing-zip/blob/fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6/obz-rs/tests/engine_integration.rs#L458)

| 每包新增量 | 配对数 | R2 中位数与范围 | R3 中位数与范围 | 配对差中位数与范围（R3 减 R2） |
| --- | ---: | ---: | ---: | ---: |
| 1 KiB | 10 | 65.428 ms，63.441–69.432 ms | 64.954 ms，63.492–70.241 ms | −0.387 ms，−2.344 至 +5.236 ms |
| 8 MiB | 8 | 236.202 ms，230.559–246.342 ms | 237.965 ms，227.740–252.706 ms | −1.474 ms，−16.446 至 +20.700 ms |

Windows 11 x64 build 26300、Ryzen 7 9800X3D，使用 Rust 1.99.0 的两套独立加固 release 构建。各组先各预热一次，再交替 R2/R3 顺序。计时从释放屏障前开始，到两个 API 完成并收回工作线程为止。外层进程启动、样本准备和输出核对在计时之外。逐次重开两个输出，核对成员集合、原成员、追加成员和输入文件的 SHA-256。

8 MiB 的两次预热分别在一个父 PID 下捕获两个同时存在的 NanaZip `a` 子进程，命令参数分别匹配各自合成输入。1 KiB 的预热未同时采到两子进程。正式计时轮次省去进程采样，以免干扰。配对差有正有负，结论限于这两组普通 ZIP 工作量。抽查没有单独测量启动锁等待，也没有测量前端交互。 [原始逐轮回执](receipt.json) · [Rust 入口](concurrent_zip.rs) · [Python 驱动](paired.py)

## 从独立工作树复跑

在 Windows x64 的 PowerShell 7 中，从含本目录的仓库根目录运行。准备 Rust 1.99.0 x64 MSVC、Visual Studio C++ 生成工具、Windows SDK 与 Python 3.14。若克隆里缺少历史提交，先执行 `git fetch origin review/quality-r3-0.12.23`。脚本为两个提交分配各自的空 Cargo target，给二者复制同一套源码随附的 NanaZip，使用单独的 Python 虚拟环境。首次共享 Cargo target 曾生成两个摘要相同的缓存产物，已弃用。复跑驱动会拒绝摘要相同的两个测试程序以及六个运行时文件中任一摘要不一致。

```powershell
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path -LiteralPath .).Path
$proof = Join-Path $repo 'docs/verification/round3/concurrent-update'
if ((rustc --version) -notmatch '^rustc 1\.99\.0\b') {
    throw 'Select Rust 1.99.0 x64 MSVC before building'
}
if ($env:RUSTFLAGS -or $env:CARGO_ENCODED_RUSTFLAGS) {
    throw 'Remove external Rust flags from this shell before building'
}

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('qing-zip-pair-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$sources = [ordered]@{
    r2 = 'b2bf9f3f4eaddcbfefd3144ad0fdb8769fa1824b'
    r3 = 'fa5ecd5de3c3826a5422a54c3bbc8f038481f6d6'
}
$target = 'x86_64-pc-windows-msvc'
$config = Join-Path $scratch 'hardening.toml'
@'
[target.x86_64-pc-windows-msvc]
rustflags=["-C","target-feature=+crt-static","-C","control-flow-guard=yes","-C","link-arg=/CETCOMPAT","-C","link-arg=/DYNAMICBASE","-C","link-arg=/HIGHENTROPYVA","-C","link-arg=/NXCOMPAT"]
'@ | Set-Content -LiteralPath $config -Encoding utf8NoBOM

$executables = @{}
foreach ($label in $sources.Keys) {
    $tree = Join-Path $scratch "src-$label"
    git -C $repo worktree add --detach $tree $sources[$label]
    if ($LASTEXITCODE) { throw "Worktree checkout failed for $label" }

    $examples = Join-Path $tree 'obz-rs/examples'
    [IO.Directory]::CreateDirectory($examples) | Out-Null
    Copy-Item -LiteralPath (Join-Path $proof 'concurrent_zip.rs') -Destination $examples

    $build = Join-Path $scratch "build-$label"
    cargo --config $config build --release --locked --target $target --target-dir $build --manifest-path (Join-Path $tree 'obz-rs/Cargo.toml') --example concurrent_zip
    if ($LASTEXITCODE) { throw "Cargo build failed for $label" }

    $staged = Join-Path $scratch "run-$label"
    $runtime = Join-Path $staged 'runtime'
    [IO.Directory]::CreateDirectory($runtime) | Out-Null
    Copy-Item -LiteralPath (Join-Path $scratch 'src-r2/runtime/nanazip') -Destination $runtime -Recurse
    Copy-Item -LiteralPath (Join-Path $build "$target/release/examples/concurrent_zip.exe") -Destination $staged
    $executables[$label] = Join-Path $staged 'concurrent_zip.exe'
}

python -m venv (Join-Path $scratch 'venv')
if ($LASTEXITCODE) { throw 'Python virtual environment creation failed' }
$python = Join-Path $scratch 'venv/Scripts/python.exe'
& $python -m pip install --disable-pip-version-check --no-cache-dir 'psutil==7.2.2'
if ($LASTEXITCODE) { throw 'Installing psutil failed' }
& $python (Join-Path $repo 'scripts/audit_pe_security.py') --require-hardened --expect-machine 0x8664 --output (Join-Path $scratch 'pe-audit.json') $executables.r2 $executables.r3
if ($LASTEXITCODE) { throw 'PE audit failed' }

# 等本机的其他 Rust 编译结束后采样。
& $python (Join-Path $proof 'paired.py') --r2 $executables.r2 --r3 $executables.r3 --out (Join-Path $scratch 'receipt.json')
if ($LASTEXITCODE) { throw 'Benchmark failed' }
Write-Output (Join-Path $scratch 'receipt.json')
```

仓库随附 NanaZip Console 的 SHA-256 为 `06070ca08b3d10e760e8e394a1b6c8ad751aff89f759227ec111b878c12db139`。PE 结果和新回执写入刚建立的临时目录。初次留下的 `receipt.json` 是报告所述两组计时的原始回执。
