# 发布指南

本文面向 bootmux 维护者。Makefile 分别提供稳定版和非稳定版入口，并将当前已经
提交的源码分发到该版本类型对应的全部发布渠道。

## 准备条件

请从干净的 `main` checkout 执行发布，并确保它的历史包含 `origin/main`。需要
安装并完成认证：

- Rust/Cargo，以及 `rustfmt` 和 `clippy` 组件；
- Git 和 GitHub CLI（可用 `gh auth status` 检查）；
- 有权发布 `bootmux` 的 Cargo 凭据；
- `curl`、`perl`、Ruby、`make`、Bash、Zsh 和 Fish（用于补全脚本校验）；
- Homebrew（仅稳定版发布需要）。

可以用 `BOOTMUX_GITHUB_REPO` 和 `BOOTMUX_HOMEBREW_TAP_REPO` 覆盖默认的
GitHub 与 tap 仓库。tap 始终通过一次性的干净 clone 更新。

正式发布前，还应按照[开发与验证](development.md)，针对本次发布声称支持的精确
后端版本，运行真实 tmux、Herdr 与 zellij 的运行时矩阵套件。自动门禁只运行标准
的非 ignored 测试；会启动真实交互后端的测试仍需要维护者显式执行。

## 稳定版

推荐先锁定目标版本，再检查并发布同一个精确版本：

```sh
VERSION="$(make --no-print-directory release-version)"
make release-check VERSION="$VERSION"
make release VERSION="$VERSION" PUBLISH=1
```

常规 patch 发布也可以直接使用一个命令，让入口自行计算目标版本：

```sh
make release PUBLISH=1
```

没有指定 `VERSION` 时，稳定版默认把 SemVer 最小的一位加一：`0.1.0` 会变成
`0.1.1`。如果当前是 `0.1.1-rc.2` 这样的预发布版，则会提升为
`0.1.1`。发布 minor 或 major 版本时请显式覆盖：

```sh
make release-check VERSION=0.2.0
make release VERSION=0.2.0 PUBLISH=1
```

稳定版状态机会依次：

1. 生成目标版本的 `Cargo.toml` 和 `Cargo.lock`，把 `[Unreleased]` 转为带日期
   的 changelog 小节，并更新安装文档中的 mise 精确版本；
2. 运行格式、Clippy、标准测试、补全脚本语法、Ruby 与 Homebrew Formula
   检查，以及允许工作树有版本修改的 Cargo 打包预检；
3. 创建本地 release-preparation commit，再对干净且已经提交的包运行第二次
   Cargo 打包预检；
4. 校验 release commit 和 tag，然后推送 `main` 与 `v<VERSION>`；
5. 创建 **draft 状态**的 GitHub Release；
6. 等待 `Release binaries` workflow 把预编译归档和 `SHA256SUMS` 附加到该
   Release，下载全部归档并逐个与清单校验，然后 clone tap，在发布 crate 前生成
   并校验目标 Formula；
7. 发布到 crates.io，并下载 crate，通过 VCS 元数据确认它来自 release commit；
8. 先更新并推送 `griffinqiu/homebrew-tap`，再更新主仓库内的 Formula 镜像；
9. 只有前面全部成功，才公开 GitHub Release。

第 4 步推送 tag 会触发 `.github/workflows/release.yml`，它构建
`aarch64-apple-darwin`、`x86_64-apple-darwin`、`aarch64-unknown-linux-musl`
和 `x86_64-unknown-linux-musl`，并把归档上传到第 5 步创建的 draft Release。
因此第 6 步会阻塞等待 CI，可能需要几分钟；超过 30 分钟则放弃。

mise 不是一个需要单独上传的发布源。它的 Cargo backend 会发现并安装 crates.io
中已经发布的版本，`ubi` backend 则读取 GitHub Release 上附加的归档。

## 非稳定版

默认预发布 channel 是 `rc`。同样建议先锁定并检查精确目标：

```sh
VERSION="$(make --no-print-directory prerelease-version CHANNEL=rc)"
make prerelease-check VERSION="$VERSION" CHANNEL=rc
make prerelease VERSION="$VERSION" CHANNEL=rc PUBLISH=1
```

只用一个发布命令时：

```sh
make prerelease PUBLISH=1
```

从 `0.1.0` 开始，默认版本是 `0.1.1-rc.1`；该版本完整发布后，下一个默认版本
是 `0.1.1-rc.2`。使用其他 channel 时，检查和发布必须传入同一个 channel：

```sh
VERSION="$(make --no-print-directory prerelease-version CHANNEL=beta)"
make prerelease-check VERSION="$VERSION" CHANNEL=beta
make prerelease VERSION="$VERSION" CHANNEL=beta PUBLISH=1
```

指定精确预发布版本时，也要先检查同一个目标：

```sh
make prerelease-check VERSION=0.2.0-alpha.1
make prerelease VERSION=0.2.0-alpha.1 PUBLISH=1
```

非稳定版会发布到 crates.io，并在 GitHub 中标记为 prerelease，但不会替换
Homebrew 的稳定 Formula。安装时必须写出精确版本：

```sh
cargo install bootmux --version 0.2.0-alpha.1 --locked
mise use -g cargo:bootmux@0.2.0-alpha.1
```

## 失败与恢复

`make release-check` 和 `make prerelease-check` 会在隔离的临时 Git worktree
中运行，不会改动调用者的 checkout，也不会发布任何外部状态。

正式发布流程以 draft GitHub Release 作为事务边界：主要预检在
release-preparation commit 和任何外部写入之前完成；干净包在第一次 push 前
检查；由 tag 生成的 Formula 和干净 tap 在发布到 crates.io 之前检查；公开
GitHub Release 永远是最后一步。

待发布的稳定版说明应写在 `CHANGELOG.md` 的精确 `## [Unreleased]` 标题下。
预发布版会保留这个小节，下一次稳定版发布时才会把它转换为带日期的版本小节。

crates.io 上的版本不能覆盖。如果流程在 release-preparation commit、tag、
draft Release、crate 或 Formula 更新之后中止，请使用原始发布计划中打印的
目标版本重试：

```sh
make release VERSION=0.2.0 PUBLISH=1
# 或
make prerelease VERSION=0.2.0-rc.1 PUBLISH=1
```

如果最初省略了 `VERSION`，直接重跑同一条发布命令也会识别当前未完成的版本并
继续，而不会再次增加 patch 或 rc。跨机器恢复时，显式写出精确 `VERSION`
仍然最清楚。

脚本会校验已存在的元数据并跳过内容一致的步骤。稳定版失败时，GitHub Release
可能保持 draft，流程会先补完 tap 或主仓库 Formula 镜像，再将其公开；若发现
冲突的 tag、crate 或 Release 元数据，流程会停止，不会覆盖。

[开发与验证](development.md) ·
[English release guide](releasing.md)
