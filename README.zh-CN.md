# bootmux

[English](README.md) | 简体中文

用同一份 tmuxinator 风格的 YAML 项目，在
[tmux](https://github.com/tmux/tmux)、[Herdr](https://herdr.dev/) 或
[zellij](https://zellij.dev/) 中运行。
bootmux 是一个独立的 Rust 二进制文件：无需 Ruby，也不会把一个终端多路复用器
藏在另一个的 pane 里运行。

```sh
bootmux start myproject
```

## 为什么选择 bootmux？

- **一种项目格式，三个原生后端。** 一个项目会成为 tmux session、Herdr
  workspace 或 zellij session；window 会成为 tmux window、Herdr tab 或
  zellij tab，pane 则始终是所选终端多路复用器中真正的 pane。
- **兼容 tmuxinator 的路径和 schema。** 现有项目可以继续放在
  `~/.config/tmuxinator`、`~/.tmuxinator`、`$TMUXINATOR_CONFIG` 或
  `./.tmuxinator.yml`。具体边界请参阅
  [兼容性说明（英文）](docs/mux-compatibility.md)。
- **模板本身不执行代码。** MiniJinja 提供变量、条件和循环，但不会执行
  Ruby。bootmux 也支持 [willfish/mux](https://github.com/willfish/mux)
  使用的非执行式 settings 占位符。项目中的 pane 命令和生命周期 hook
  仍然是 shell 命令，因此只能运行可信的 YAML。
- **经过测试的兼容性。** 三个具有代表性的 tmux 渲染结果会与源自
  tmuxinator 的 golden snapshot 逐字节比较，同样这三个项目也固定了各自的
  zellij KDL layout；另一套独立的 19 文件矩阵会在三个后端上验证固定版本的
  mux fixture。

## 环境要求

- Unix-like 操作系统
- 至少安装一个终端多路复用器：
  - tmux >= 2.6
  - Herdr >= 0.7.5，并使用 socket protocol 17
  - zellij >= 0.44
- 安装或构建 bootmux 需要 Rust 和 Cargo >= 1.89
- 正常的项目和编辑器工作流需要设置 `$SHELL` 和 `$EDITOR`
- 可选：`bootmux picker` 需要 `fzf`

## 安装

使用 Cargo：

```sh
cargo --version
cargo install bootmux --locked
bootmux version
```

使用 [mise 的 Cargo 后端](https://mise.jdx.dev/dev-tools/backends/cargo.html)：

```sh
mise use -g rust
mise use -g cargo:bootmux@0.1.2
bootmux version
```

显式版本在 mise 针对新发布版本默认设置的 24 小时安全延迟期内也能正常安装。
发布满 24 小时后，`mise use -g cargo:bootmux` 会选择最新的可用版本。

使用 Homebrew：

```sh
brew install griffinqiu/tap/bootmux
bootmux --version
```

也可以构建当前 checkout，而不是安装 crates.io 版本：

```sh
cargo install --path . --locked
bootmux version
```

Cargo 和 mise 只会安装可执行文件。静态 Bash、Zsh 和 Fish 补全脚本位于
[`completion/`](completion/)；请从相同版本的源码中把对应文件复制到 shell
的补全目录。Homebrew Formula 会自动安装这三种补全脚本。

安装后，请检查准备使用的每个后端：

```sh
bootmux --backend tmux doctor
bootmux --backend herdr doctor
```

## 快速上手

创建 `~/.config/tmuxinator/myapp.yml`：

```yaml
name: myapp
root: ~/code/myapp

windows:
  - editor:
      panes:
        - nvim
        - git status
  - server: npm run dev
  - logs: tail -f logs/development.log
```

启动示例前，请把 `root` 替换为一个已经存在的目录，并确认示例中的命令已安装
在本机。

先预览所选后端的计划，再启动和停止项目：

```sh
bootmux --backend tmux debug myapp
bootmux --backend tmux start myapp
bootmux --backend tmux stop myapp

bootmux --backend herdr debug myapp
bootmux --backend herdr start myapp
bootmux --backend herdr stop myapp

bootmux --backend zellij debug myapp
bootmux --backend zellij start myapp
bootmux --backend zellij stop myapp
```

`debug` 会验证并渲染后端计划，但不会创建项目拓扑。Herdr 和 zellij debug 都不会
连接 server，zellij debug 会打印 bootmux 将要使用的完整 KDL layout；tmux debug
为了读取 `base-index`、`pane-base-index` 和活动 session 状态，可能会启动
tmux server。

项目正常工作后，可以不再显式指定后端。bootmux 按以下顺序解析后端：

1. `--backend tmux|herdr|zellij`
2. 当前活动的终端多路复用器环境
3. bootmux 全局设置中的 `default_backend`
4. tmux

```sh
bootmux config set default-backend herdr
bootmux myapp                 # 简写：bootmux start myapp
bootmux                       # 有本地项目时启动它，否则打开 fzf picker
```

活动的 Herdr popup 优先于继承的环境变量。如果多个终端多路复用器确实存在嵌套，
而 bootmux 无法识别前台归属，它会拒绝猜测、列出所有候选，并要求显式指定后端。

## 已经有 tmuxinator 或 mux 项目？

把文件留在原来的目录中，先在每个后端上分别进行预检：

```sh
bootmux --backend tmux debug PROJECT
bootmux --backend herdr debug PROJECT
bootmux --backend zellij debug PROJECT
```

第一次用非 tmux 后端启动前，请先阅读
[兼容性矩阵和迁移步骤（英文）](docs/mux-compatibility.md)。

## 后端概览

| 能力 | tmux | Herdr | zellij |
|---|---|---|---|
| 原生项目容器 | Session | Workspace | Session |
| Window 映射 | Window | Tab | Tab |
| Pane 命令传输 | `send-keys` | `pane run` | `write-chars` + `send-keys Enter` |
| Pane 命令和工作目录 | 支持 | 支持 | 支持 |
| 命名 socket 和自定义 socket | 支持 | 支持 | 不支持：`socket_name`/`socket_path` 会被忽略 |
| tmux 预设 layout | 原生应用 | 转换为 BSP 计划 | 转换为 KDL layout |
| 序列化 tmux layout | 原生应用 | 严格解析后再转换 | 严格解析后再转换 |
| `synchronize` | 支持 | 拒绝 truthy（按真值判断为真）的值：没有等价的输入语义 | 警告后忽略 |
| `tmux_options`、`tmux_command`、pane 边框字段 | 支持 | 警告后忽略 | 警告后忽略 |
| 停止时的身份依据 | 从配置渲染 session/socket；没有所有权状态 | 持久化 endpoint/config/name/root 的精确所有权记录 | session 名称；没有所有权状态 |

[Herdr session restore](https://herdr.dev/docs/session-state/) 会恢复终端拓扑和
工作目录，但不会恢复任意子进程的状态。复用匹配的 workspace 或 session 时会运行
`on_project_restart`，不会重新运行每个 pane 命令。在依赖重启恢复或
`stop-all` 前，请先阅读[后端与生命周期（英文）](docs/backends.md)。

正常的 Herdr 或 zellij 启动会把一个项目映射为一个 workspace 或 session。
`--append` 则会把 tab 添加到当前活动的那一个，不会创建一个可以独立停止的项目。

## 文档

- 中文端到端指南：[完整使用手册](docs/manual.zh-CN.md)
- 快速上手（英文）：[Getting started](docs/getting-started.md)
- 英文参考：[CLI](docs/cli.md)、[项目配置](docs/configuration.md)、
  [后端与生命周期](docs/backends.md)和
  [mux 兼容性](docs/mux-compatibility.md)
- 贡献者文档（英文）：[开发与验证](docs/development.md)
- 维护者文档：[中文发布指南](docs/releasing.zh-CN.md) ·
  [Release guide](docs/releasing.md)

## Picker 快捷键

只有 picker 需要 `fzf`。打印安全的配置片段，再把它粘贴到对应终端多路复用器
的配置中：

```sh
bootmux bindings tmux
bootmux bindings herdr
bootmux bindings zellij
```

tmux 片段使用普通 window，因此仍然兼容 tmux 2.6。Herdr 片段会打开一个
80% 大小的 popup。zellij 片段默认绑定 `Ctrl y`，在除 `locked` 外的所有模式下
用浮动 pane 打开 picker。

## 许可证与致谢

bootmux 使用 MIT 许可证，是一个独立的重新实现。tmuxinator 的版权归其
贡献者所有，并同样使用 MIT 许可证。

Herdr 后端将
[`willfish/mux` 的 `927030b` 版本](https://github.com/willfish/mux/tree/927030bb88e4b16b6671f68610980491ffbd2c81)
作为行为参考，但没有复制其实现。上游 YAML 测试样例已原样纳入本仓库，并保留了
[来源和许可证说明（英文）](tests/fixtures/mux/README.md)。
