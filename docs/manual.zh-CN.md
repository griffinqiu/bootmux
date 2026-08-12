# bootmux 用户手册

简体中文 | [English](manual.md)

这是 bootmux 的完整使用手册。内容从安装开始，依次介绍常规项目生命周期、
常见的 tmux/Herdr 工作流、安全停止、迁移和故障排查。

需要精确查阅某项内容时，请使用范围更明确的参考页：

- [CLI 参考](cli.md)
- [项目配置](configuration.md)
- [后端与生命周期](backends.md)
- [mux 兼容性](mux-compatibility.md)

## 目录

- 基础：[模型](#1-bootmux-模型)、
  [安装](#2-安装并验证)、[文件位置](#3-了解文件位置)、
  [第一个项目](#4-创建第一个项目)和
  [预检](#5-启动前预检)
- 日常使用：[启动/停止](#6-启动检查与停止)、
  [后端选择](#7-选择后端)、
  [附着/焦点](#8-控制附着与焦点)和
  [窗口/窗格](#9-编写窗口与窗格)
- 配置：[布局](#10-使用布局)、[钩子](#11-添加生命周期钩子)、
  [模板](#12-安全地使用项目模板)和
  [套接字](#13-使用套接字与命名会话)
- 工作流：[追加](#14-追加另一个项目)、
  [选择器](#15-使用选择器)、[导入 tmux](#16-导入现有-tmux-会话)、
  [文件管理](#17-管理项目文件)和
  [安全停止](#18-安全停止)
- 运维：[重启/恢复](#19-为重启与进程恢复做准备)、
  [迁移](#20-迁移-tmuxinator-或-mux-项目)、
  [故障排查](#21-故障排查)、
  [环境变量参考](#22-环境变量速查)和
  [命令速查](#23-命令速查表)

首次使用时，请阅读第 1–7 节。编写项目配置时，请继续阅读第 8–13 节。
第 14–21 节涵盖日常运维、迁移、恢复和故障排查。

## 1. bootmux 模型

一个 YAML 项目描述：

- 项目名称和工作目录；
- 一个或多个窗口；
- 每个窗口中可选的窗格；
- 每个窗格要执行的命令序列；
- 可选的钩子、焦点、布局、附着和套接字设置。

bootmux 会把这些意图原生转换到对应后端：

| 项目概念 | tmux | Herdr | zellij |
|---|---|---|---|
| 项目 | 会话 | 工作区 | 会话 |
| 窗口 | 窗口 | 标签页 | 标签页 |
| 窗格 | 窗格 | PTY 窗格 | 窗格 |
| 窗格命令 | `send-keys` | `pane run` | `write-chars` + `send-keys Enter` |
| 布局 | tmux 布局 | Herdr BSP 分割 | KDL 布局 |

普通启动会创建或复用一个容器。`--append` 是例外：它把窗口/标签页添加到
当前容器中，而不是创建一个独立项目。

YAML 可移植并不代表多路复用器的行为完全相同。tmux 专用选项仍然只适用于
tmux，Herdr 的所有权检查更严格，zellij 仅凭会话名称识别项目，而布局转换
可以保留拓扑，却不一定能保留精确的单元格几何尺寸。

## 2. 安装并验证

要求：

- 类 Unix 操作系统
- Rust 和 Cargo 1.89 或更新版本
- tmux 2.6 或更新版本、Herdr 0.7.5/protocol 17 或 19，或两者都安装
- `$SHELL` 和 `$EDITOR`
- 可选的 `fzf`

### 安装预编译可执行文件

每个带 tag 的 release 都附带 `aarch64-apple-darwin`、`x86_64-apple-darwin`、
`aarch64-unknown-linux-musl` 和 `x86_64-unknown-linux-musl` 的预编译归档，
以及 `SHA256SUMS` 校验清单。每个归档都包含 `bootmux` 可执行文件和 Bash、Zsh、
Fish 补全脚本。这些安装方式都不需要 Rust 工具链。

Homebrew 和 [mise 的 `ubi` 后端](https://mise.jdx.dev/dev-tools/backends/ubi.html)
都使用这些归档：

```sh
brew install griffinqiu/tap/bootmux
mise use -g ubi:griffinqiu/bootmux
```

如需手动安装，请从
[releases 页面](https://github.com/griffinqiu/bootmux/releases)下载归档，
用 `SHA256SUMS` 校验后把可执行文件放到 `PATH` 中：

```sh
shasum -a 256 --check --ignore-missing SHA256SUMS
tar -xzf bootmux-aarch64-apple-darwin.tar.gz
install -m 0755 bootmux /usr/local/bin/bootmux
```

### 从 crates.io 安装

从源码构建需要 Rust 和 Cargo >= 1.89。

```sh
rustc --version
cargo --version
cargo install bootmux --locked
bootmux version
```

Cargo 通常会把二进制文件安装到
`${CARGO_HOME:-$HOME/.cargo}/bin`。如果 shell 找不到 `bootmux`，请把这个
目录加入 `PATH`。

### 使用 mise 安装

[mise 的 Cargo 后端](https://mise.jdx.dev/dev-tools/backends/cargo.html)
会用 Cargo 构建 crate，并把所选版本写入 mise 的全局配置：

```sh
mise use -g rust
mise use -g cargo:bootmux@0.1.5
bootmux version
```

如果只想为某个项目固定 bootmux，请在项目中运行不带 `-g` 的
`mise use cargo:bootmux@0.1.5`。

mise 默认会对 `latest` 这样的模糊版本请求应用 24 小时的最短发布年龄。
因此刚发布的 crate 使用 `cargo:bootmux@latest` 时可能出现
“no versions found”。建议使用上面的精确版本，而不是关闭这层安全延迟。
发布满 24 小时后，`mise use -g cargo:bootmux` 会选择最新的可用版本。

### 使用 Homebrew 安装

项目 tap 会安装当前平台的预编译可执行文件，并附带 Bash、Zsh 和 Fish 补全。
稳定版 Formula 不声明任何依赖，因此不会连带拉取编译工具链：

```sh
brew install griffinqiu/tap/bootmux
bootmux --version
```

例外是 `brew install --HEAD griffinqiu/tap/bootmux`：它会从源码构建 `main`
分支，因此依赖 `rust`。

升级或卸载：

```sh
brew upgrade griffinqiu/tap/bootmux
brew uninstall griffinqiu/tap/bootmux
```

### 从源码 checkout 安装

在仓库根目录运行：

```sh
cargo install --path . --locked
bootmux version
```

### 升级或卸载

```sh
cargo install bootmux --locked --force
cargo uninstall bootmux
```

`cargo uninstall` 只会删除 Cargo 安装的可执行文件，不会删除项目 YAML、
bootmux 设置、Herdr 所有权状态或手动复制的补全脚本。

### 检查每个后端

```sh
bootmux --backend tmux doctor
bootmux --backend herdr doctor
bootmux --backend zellij doctor
```

`doctor` 会检查所选的多路复用器、可选的 `fzf`、`$EDITOR` 和 `$SHELL`。

使用 Herdr 时，也请直接验证其二进制文件：

```sh
herdr --version
```

bootmux 要求 Herdr 客户端和服务器相互兼容。它不会静默降级到其他协议。

使用 zellij 时，同样请直接确认版本：

```sh
zellij --version
```

bootmux 要求 zellij >= 0.44——这是第一个能从会话外部构建并驱动会话的版本。

### Shell 补全

Cargo 和 mise 只会安装可执行文件。Homebrew 会安装全部三种补全。仓库/源码
归档提供了相同的面向项目和子命令的静态文件：

```text
completion/bootmux.bash
completion/bootmux.zsh
completion/bootmux.fish
```

Bash 可以直接 source 对应文件：

```sh
source /path/to/bootmux/completion/bootmux.bash
```

Fish 可以 source 对应文件，也可以把它放入 Fish 的常规补全目录：

```fish
source /path/to/bootmux/completion/bootmux.fish
```

Zsh 需要在 `compinit` 后显式注册：

```zsh
autoload -Uz compinit && compinit
source /path/to/bootmux/completion/bootmux.zsh
compdef _bootmux bootmux
```

这些文件不会尝试穷举每个标志或自由形式模板参数的补全。

## 3. 了解文件位置

### 项目文件

命名项目按以下顺序查找：

1. `$TMUXINATOR_CONFIG`
2. `${XDG_CONFIG_HOME:-$HOME/.config}/tmuxinator`
3. `$HOME/.tmuxinator`

直接查找时同时接受 `.yml` 和 `.yaml`。在每个目录中，会先检查直接相对路径，
然后按基本文件名递归查找。

示例：

```text
~/.config/tmuxinator/api.yml              -> bootmux start api
~/.config/tmuxinator/team/api.yml         -> bootmux start team/api
~/.tmuxinator/legacy.yaml                 -> bootmux start legacy
```

递归查找时如果基本文件名冲突，将采用排序后的第一个匹配项。建议使用
`team/api` 这样的相对名称来消除歧义。

设置 `$TMUXINATOR_CONFIG` 会使其成为最高优先级的项目目录。如果该变量已设置
但目录不存在，bootmux 会创建它。

为兼容 tmuxinator，项目枚举（`list`、选择器、补全以及 tmux `stop-all`）
只包含 `.yml` 文件。直接启动一个已知的 `.yaml` 项目仍然可用。

### 仓库本地项目

bootmux 识别：

```text
./.tmuxinator.yml
./.tmuxinator.yaml
```

如果两者同时存在，优先使用 `.yml`。

### bootmux 全局设置

```text
${XDG_CONFIG_HOME:-$HOME/.config}/bootmux/config.toml
```

目前支持：

```toml
default_backend = "herdr"
```

请使用 CLI，而不是手动编辑：

```sh
bootmux config set default-backend herdr
bootmux config get default-backend
bootmux config path
```

### Herdr 所有权状态

```text
${XDG_STATE_HOME:-$HOME/.local/state}/bootmux/herdr-workspaces.json
```

这个文件不是用户项目配置。它记录受管理工作区的精确身份以及渲染后的停止
钩子快照。不要随意编辑。

## 4. 创建第一个项目

```sh
bootmux new myapp
```

这会在所选项目目录中创建 `myapp.yml`，并用 `$EDITOR` 打开。

可以从一个同时适用于所有后端的项目开始：

```yaml
name: myapp
root: ~/code/myapp
attach: false

windows:
  - editor:
      panes:
        - nvim
        - git status
  - server: npm run dev
  - logs: tail -f logs/development.log
```

启动前，根目录应当已经存在。首次运行时使用 `attach: false` 很有帮助，因为
它能避免在检查结果时切换当前客户端。

## 5. 启动前预检

对于新建或迁移的项目，应始终在每个计划使用的后端上进行调试：

```sh
bootmux --backend tmux debug myapp
bootmux --backend herdr debug myapp
```

tmux debug 会打印生成的 shell 脚本。为了读取以下信息，它可能会启动或查询
一个 tmux 服务器：

- `base-index`；
- `pane-base-index`；
- 目标会话是否已经存在。

它不会执行项目创建脚本。

Herdr debug 完全离线。它会打印：

- 选定的端点；
- 项目和配置身份；
- 附着/追加策略；
- 静态所有权操作；
- 是否存在钩子；
- 布局分割、窗格数量和命令数量；
- 启动时选择。

它不会打印命令正文，也不会检查实时所有权或服务器状态。

请区分警告和错误：

- Herdr 警告表示某个求值为真的、仅适用于 tmux 的外观或 CLI 字段将被忽略；
- 错误表示配置声明的行为无法得到保留，例如启用了 `synchronize`，或者序列化
  布局无效。

## 6. 启动、检查与停止

### 显式首次运行

tmux：

```sh
bootmux --backend tmux start myapp
bootmux --backend tmux list --active
bootmux --backend tmux stop myapp
```

Herdr：

```sh
bootmux --backend herdr start myapp
bootmux --backend herdr list --active
bootmux --backend herdr stop myapp
```

Herdr 和 zellij 后端在执行命令的终端之外生效，因此 `start`、`local` 和 `stop`
成功后各会打印一行结果，说明是创建、复用、追加还是停止了哪个 workspace 或
session：

```text
bootmux: created herdr workspace "myapp" (socket:/Users/me/.config/herdr/herdr.sock)
bootmux: reused zellij session "myapp"
```

tmux 后端不打印这一行，因为 tmux 启动会接管终端，结果本身就可见。

### 项目简写

如果项目名称可被发现，以下命令：

```sh
bootmux myapp
```

等价于：

```sh
bootmux start myapp
```

命令名称和别名的优先级高于项目简写。如果项目名为 `l`、`st` 或其他别名，
请显式使用 `bootmux start NAME`。

### 本地项目

在包含 `.tmuxinator.yml` 的仓库中：

```sh
bootmux local
bootmux .
```

如果存在本地项目，直接运行 `bootmux` 会启动它；否则会启动 `fzf` 选择器。

`local` 不接受自由形式模板参数。对于带模板的本地文件，请使用：

```sh
bootmux start -p ./.tmuxinator.yml root=/work/myapp
```

### 重复启动

重复启动会复用匹配的会话/工作区：

```text
on_project_start
on_project_restart
附着/焦点行为
on_project_exit
```

它不会重新创建窗口/标签页，也不会再次执行窗格命令。因此，从拓扑角度看，
重复调用是幂等的，但 bootmux 并不是进程监督器。

如果容器仍然存在时也需要恢复某个进程，请使用幂等的
`on_project_restart` 钩子，或者停止并重新创建项目。

### 运行替代实例

`start -n NAME` 适合临时启动，但 `stop` 没有对应的 `--name` 选项。对于必须
以声明式方式启动和停止的实例，请将项目名称模板化：

```yaml
name: {{ settings.instance | default('myapp') }}
root: ~/code/myapp
windows:
  - shell:
```

```sh
bootmux start myapp instance=myapp-review
bootmux stop myapp instance=myapp-review
```

如果没有这种可复现身份，通过 tmux `-n` 启动的会话可能需要直接用 tmux
清理。Herdr 的替代工作区仍会被记录，并可由基于所有权的 `stop-all` 处理，
但普通的 stop 无法根据未变更的配置重建其身份。

## 7. 选择后端

解析顺序：

1. `--backend tmux|herdr|zellij`
2. 当前活动的多路复用器环境
3. 全局 `default_backend`
4. tmux

示例：

```sh
bootmux --backend tmux start myapp
bootmux --backend herdr start myapp
bootmux --backend zellij start myapp
```

zellij 环境通过 `ZELLIJ`、`ZELLIJ_SESSION_NAME` 或 `ZELLIJ_PANE_ID` 识别。
zellij 会把 `ZELLIJ` 设为字符串 `0`，因此 bootmux 判断的是变量是否存在，
而不是它是否为真值。

活动的 Herdr 弹窗使用 `HERDR_ACTIVE_*`，其优先级高于继承的 `TMUX` 值。对于
确实运行在 Herdr 内部的 tmux 进程，bootmux 会询问 Herdr 当前窗格的前台进程
归属。如果无法判定嵌套情况，它会列出所有候选后端并报错，而不是猜测；
tmux 与 zellij 嵌套时没有可用的判定手段，因此必须显式指定 `--backend`。

只在 Herdr 外部设置 `HERDR_SESSION` 会选择一个端点，但不会选择后端。请添加
`--backend herdr`，或配置默认后端。

## 8. 控制附着与焦点

YAML 默认会附着：

```yaml
attach: true
```

可以在单次调用中覆盖：

```sh
bootmux start myapp --attach
bootmux start myapp --no-attach
```

优先级：

1. `--attach`
2. `--no-attach`
3. YAML `attach`
4. 默认值 true

为了兼容 mux，请在 YAML 中使用小写 `false`。其他标量写法可能产生意外结果：
只有精确的 `false` 和 `0`（无论是否加引号）会禁用附着。

最终选择：

```yaml
startup_window: editor
startup_pane: shell

windows:
  - editor:
      focused_pane: shell
      panes:
        - editor: nvim
        - shell:
```

- `focused_pane` 保存窗口/标签页内的首选窗格。
- `startup_window` 选择最终的窗口/标签页。
- `startup_pane` 会覆盖该启动窗口的首选窗格。

在跨后端配置中，建议为 `startup_window` 使用名称。数字窗口选择会受到 tmux
`base-index` 的影响，而 Herdr 使用从零开始的逻辑索引。

窗格索引是从零开始的逻辑值。tmux 会使用 `pane-base-index` 对其进行调整。

## 9. 编写窗口与窗格

### 一个窗格中的一条命令

```yaml
windows:
  - server: npm run dev
```

### 一个窗格中的多条命令

```yaml
windows:
  - setup:
      - git fetch
      - git status
```

### 多个窗格

```yaml
windows:
  - editor:
      panes:
        - editor: nvim
        - shell:
        - tests:
            - cargo test
            - cargo watch -x test
```

空值会创建一个 shell 窗格。

### 每个窗口的根目录与准备命令

```yaml
root: ~/code/myapp
pre_window: export APP_ENV=development

windows:
  - api:
      root: services/api
      pre: source .env
      panes:
        - server: cargo run
        - tests: cargo test
```

`services/api` 会解析为项目根目录下的路径。每个窗格中的命令按以下顺序执行：

1. 顶层 `pre_window`
2. 窗口 `pre`
3. 窗格命令

`--no-pre-window` 只会跳过第一步。

## 10. 使用布局

可移植的预设：

```yaml
layout: tiled
layout: even-horizontal
layout: even-vertical
layout: main-horizontal
layout: main-vertical
```

tmux 会原生应用预设。Herdr 会构建一个确定性的二叉分割方案。

同时也支持序列化的 tmux 布局字符串。Herdr 会检查校验和、树几何结构、
窗格数量以及比例是否可表示。即使 tmux 字符串有效，如果无法安全转换，
仍可能被拒绝。

### 显式分割链

如果希望直接从 YAML 看出布局：

```yaml
windows:
  - app:
      panes:
        - editor:
            command: nvim
        - server:
            split: right
            ratio: 0.65
            command: npm run dev
        - logs:
            split: down
            ratio: 0.5
            command: tail -f logs/development.log
```

后续每个窗格都会分割它前面的那个窗格。

规则：

- `split`：`right` 或 `down`
- `ratio`：现有窗格所占比例，范围为 0.1 到 0.9
- `command` 和 `commands`：二选一
- 第一个窗格不能设置 `split`/`ratio`
- 窗口 `layout` 不能与窗格链同时使用

这种语法在所有后端上均可使用。tmux 和 zellij 会把百分比舍入为整数；Herdr 接收
浮点比例，因此两者的比例可能略有不同。

## 11. 添加生命周期钩子

```yaml
on_project_start: docker compose up -d
on_project_first_start: bin/setup
on_project_restart: bin/recover-dev-processes
on_project_exit: echo "start operation exited"
on_project_stop: docker compose down
```

钩子也可以是列表：

```yaml
on_project_stop:
  - docker compose down
  - rm -f tmp/dev.pid
```

生命周期：

```text
每次启动：
  on_project_start

容器不存在：
  on_project_first_start
  创建拓扑并派发窗格命令

容器已存在：
  on_project_restart

启动操作退出：
  on_project_exit

停止：
  on_project_stop
  关闭容器
```

每当要创建缺失的拓扑时，都会运行 `on_project_first_start`。它不是永久性的
安装标记。所有后端也都会在创建追加的窗口/标签页之前运行它。

不同后端处理钩子失败的方式不同：

- tmux start 是一个 `/bin/sh -e` 脚本，但派发到窗格中的命令是异步的；
- tmux stop 刻意不采用快速失败，因此即使 `cd` 或钩子失败，仍会尝试关闭
  会话；
- Herdr 从项目根目录运行钩子、检查其退出状态，并在停止钩子失败时阻止停止。

请让钩子保持幂等。如果项目根目录已被删除，不要依赖 tmux 停止钩子仍能在
预期目录中运行。

钩子和窗格命令都是 shell 命令。安全的模板渲染并不会让不受信任的项目文件
变得可以安全执行；运行前请审查 YAML。

## 12. 安全地使用项目模板

### MiniJinja 变量

```yaml
name: {{ settings.session | default('myapp') }}
root: "{{ env.HOME }}/code/{{ args[0] }}"
windows:
  - server: npm run {{ settings.task | default('dev') }}
```

```sh
bootmux start myapp frontend session=myapp-dev task=serve
```

可用值：

- `settings`：包含非空 `key=` 的 CLI 参数
- `args`：所有其他位置参数
- `env`：环境变量

解析规则：

- `a=b=c` 会变为键 `a`、值 `b=c`；
- 设置项重复时，采用第一个；
- `=value` 仍是位置参数；
- 未定义的值会渲染为空文本。

渲染结果仍必须是有效的 YAML。如果模板表达式的值可能包含 `:`、`#`、
大括号或其他对 YAML 有特殊意义的字符，请给表达式加引号。

### 受限的 mux 占位符

支持以下 willfish/mux 形式，且不会执行 Ruby：

```erb
root: <%= @settings["root"] %>
```

只接受这个形式完全一致、使用双引号的 `@settings["key"]` 表达式。对于参数、
环境变量、条件和循环，请使用 MiniJinja。

### 使用相同的值停止

如果模板控制项目名称、根目录、套接字或停止钩子，停止时必须复现它的所有
输入：`key=value` 设置项、位置参数以及模板引用的环境变量。

```sh
bootmux start myapp root=/work/myapp env=staging
bootmux stop myapp root=/work/myapp env=staging
```

Herdr 会比较渲染后的身份和停止钩子快照。发现不匹配时会拒绝操作，以免关闭
错误的工作区，或执行非预期的钩子。

## 13. 使用套接字与命名会话

### tmux

```yaml
socket_name: myapp
# or:
socket_path: /tmp/myapp-tmux.sock
```

它们会转换为 tmux 的 `-L` 或 `-S`。`socket_path` 优先。
在跨后端共享的项目中，请使用绝对 `socket_path`。

额外的 tmux CLI 设置：

```yaml
tmux_options: -f ~/.tmux.custom.conf
tmux_command: tmux
```

`tmux_options` 的旧别名是 `cli_args`。tmux 后端会原样传递 `wemux` 这样的
自定义命令。

### Herdr

相同的 YAML 键用于选择 Herdr 端点：

```yaml
socket_name: myapp
# or:
socket_path: /tmp/myapp-herdr.sock
```

端点优先级：

1. YAML `socket_path`
2. YAML `socket_name`
3. `HERDR_SOCKET_PATH`
4. `HERDR_SESSION`
5. 默认 Herdr 端点

命名会话允许 1–64 个 ASCII 字母、数字以及 `.`、`_`、`-`。

Herdr 会对 `tmux_options`、`tmux_command` 和窗格边框设置发出警告并忽略。
如果这些值不是仅影响外观，而是不可或缺，请分别维护后端专用项目文件。

## 14. 追加另一个项目

### tmux 追加

在目标 tmux 会话内部运行：

```sh
bootmux --backend tmux start tools --append
```

项目窗口会添加到当前会话的最后一个窗口之后。不会创建新的项目会话，也不会
持久化追加记录；`bootmux stop tools` 不会移除这些追加的窗口。请将它们作为
接收会话的一部分来管理。

### Herdr 追加

在目标 Herdr 工作区或弹窗内部运行：

```sh
bootmux --backend herdr start tools --append
```

所选端点必须正是包含活动工作区的端点。项目标签页会添加到该工作区。

Herdr append 不会创建单独的受管理工作区记录。之后运行
`bootmux stop tools` 无法只移除这些追加的标签页。请将它们作为接收工作区的
一部分来管理。

如果发生部分失败，bootmux 会尝试回滚刚追加的标签页。

## 15. 使用选择器

安装 `fzf`，然后运行：

```sh
bootmux picker
```

取消选择属于正常操作，不会生成项目。

打印一段安全的多路复用器配置：

```sh
bootmux bindings tmux
bootmux bindings herdr
```

默认值：

- tmux 前缀键 + `F`，为兼容 tmux 2.6 而打开普通窗口；
- Herdr `prefix+shift+f`，打开一个占 80% 大小的弹窗。

自定义按键：

```sh
bootmux bindings tmux --key C-f
bootmux bindings herdr --key prefix+alt+f
```

将生成的内容粘贴到相应多路复用器的配置中，然后重新加载该配置。

## 16. 导入现有 tmux 会话

```sh
bootmux --backend tmux new imported existing-session
```

该命令会检查现有 tmux 会话，并写入一个初始项目，其中包含：

- 窗口名称；
- 序列化布局；
- 窗格工作目录；
- 推导出的项目根目录。

它不会捕获完整的实时进程状态或命令历史。请审查生成的 YAML，并用声明式
命令替换 `cd` 占位符。

这种形式仅适用于 tmux，不能导入 Herdr 工作区。

## 17. 管理项目文件

```sh
bootmux open myapp
bootmux edit myapp
bootmux copy myapp myapp-alt
bootmux delete myapp-alt
bootmux list
bootmux list --newline
```

如果项目不存在，`open` 会创建它。`edit` 只会打开现有项目。

这些编辑器命令只针对 bootmux 当前写入目录中的 `NAME.yml`，不会递归查找
优先级较低的副本。该目录依次为：`$TMUXINATOR_CONFIG`；否则是现有的 XDG
目录；否则是现有的 `~/.tmuxinator`；如果这些都不存在，则是新创建的 XDG
目录。因此，即使较低优先级位置已有副本，`open NAME` 也可能创建一个更高
优先级的重复项目，而 `edit NAME` 也可能报告文件不存在。请有意识地设置
`TMUXINATOR_CONFIG`，或直接编辑目标文件。

本地形式：

```sh
bootmux new myapp --local
bootmux open myapp --local
bootmux edit --local
```

破坏性命令：

```sh
bootmux delete project-a project-b
bootmux implode
```

`delete` 会在删除每个文件前询问。`implode` 只询问一次，然后递归删除所选的
项目配置目录，而不只是已知的 YAML 文件。确认前，请检查
`TMUXINATOR_CONFIG` 以及 XDG/home 下的 tmuxinator 目录。

## 18. 安全停止

### 停止一个 tmux 项目

```sh
bootmux --backend tmux stop myapp
```

tmux stop 会从当前配置渲染会话/套接字。它没有所有权证明。请确认渲染后的
名称确实指向预期会话。

停止脚本刻意不采用快速失败，因此根目录消失也不会阻止 `kill-session`。
请确保即使 `cd` 失败，停止钩子仍然安全。

### 停止一个 Herdr 项目

```sh
bootmux --backend herdr stop myapp
```

Herdr 要求端点、配置路径、名称、标签、根目录、工作区身份和渲染后的停止
钩子均与持久化记录匹配。遇到歧义或漂移时，操作会以拒绝为默认安全策略。

如果记录中的服务器已停止，bootmux 可以启动它，以便验证并关闭工作区。
之后不会停止该服务器。

### 全部停止

预览确认列表：

```sh
bootmux --backend tmux stop-all
bootmux --backend herdr stop-all
```

仅在受控环境中跳过确认：

```sh
bootmux --backend herdr stop-all --noconfirm
```

tmux `stop-all` 使用启发式规则：它将可发现的 `.yml` 项目基本文件名与当前
环境所指向的 tmux 服务器中的名称匹配（在 tmux 内部时是当前服务器，在外部
时通常是默认服务器）。然后，它会渲染每份候选配置自己的 tmux/套接字命令，
而该命令可能指向与发现阶段不同的服务器。它可能漏掉自定义名称、模板名称、
`.yaml`、外部 `-p` 配置和只使用自定义套接字的会话。确认列表只显示名称，
不显示端点身份，也无法证明同名会话是由 bootmux 创建的。

Herdr `stop-all` 会遍历 bootmux 的所有权记录，验证每个工作区，运行持久化的
停止钩子快照（如果没有快照，则采用旧版配置回退），并且只关闭工作区。它
绝不会停止 Herdr 服务器。操作会在第一个验证、钩子或关闭错误处停止；解决
错误并再次运行 `stop-all` 之前，后续记录不会被处理。

新的所有权记录包含渲染后的停止钩子，因此即使配置已被删除，`stop-all`
仍可工作。该快照以纯文本保存在权限模式为 0600 的私有状态文件中；不要在
钩子源代码中放置秘密信息。

tmux `stop-all` 会在子停止脚本返回非零状态后继续，并且不会汇总这些状态。
因此，顶层命令成功返回并不能证明每个候选会话都已关闭；请检查剩余会话列表。

## 19. 为重启与进程恢复做准备

tmux 和 [Herdr 会话恢复](https://herdr.dev/docs/session-state/)可以用不同方式保留
或恢复终端拓扑，但窗格中的普通进程并不等同于受监督的服务。

bootmux 只会在创建或追加拓扑时运行窗格命令。如果匹配的会话/工作区仍然存在
或已恢复，重复启动会进入重启路径，不会重放窗格命令。

建议采用以下方式：

- 使用 `on_project_restart` 探测并重启缺失的开发进程；
- 让重启钩子保持幂等；
- 对长期运行的关键服务使用真正的服务管理器；
- 需要干净地完整重放时，停止并重新创建项目；
- 更改布局后，先运行 `debug`，再重新创建。

示例：

```yaml
on_project_restart: |
  pgrep -f 'npm run dev' >/dev/null || npm run dev >tmp/dev.log 2>&1 &
```

探测条件应足够具体，以免匹配到其他项目。

## 20. 迁移 tmuxinator 或 mux 项目

建议流程：

1. 将 YAML 留在现有 tmuxinator 目录中；
2. 运行 tmux debug；
3. 运行 Herdr debug；
4. 将不支持的 ERB 迁移到 MiniJinja；
5. 处理 Herdr 警告和已启用的 `synchronize`；
6. 使用 `--no-attach` 启动；
7. 检查实际窗格和工作目录；
8. 使用完全相同的模板输入停止；
9. 只有在理解各后端的路径后，才设置默认后端。

兼容别名：

```text
project_name -> name
project_root -> root
tabs         -> windows
cli_args     -> tmux_options
```

如果别名冲突，将采用 YAML 文档顺序中最后一个类型有效的字段。

ERB 中只保留以下不可执行的 mux 形式：

```erb
<%= @settings["key"] %>
```

请参阅精确的 [19 文件兼容性矩阵](mux-compatibility.md#fixture-matrix)及其预期的
安全拒绝项。

## 21. 故障排查

### “both tmux and Herdr environments are active”

无法安全判定嵌套关系：

```sh
bootmux --backend tmux start myapp
# or
bootmux --backend herdr start myapp
```

### “Project … doesn't exist”

请检查：

```sh
bootmux list --newline
printf '%s\n' "$TMUXINATOR_CONFIG"
bootmux config path
```

请记住，`list` 只枚举 `.yml` 文件。要使用显式文件，请指定 `-p`：

```sh
bootmux start -p /absolute/path/project.yaml
```

### “Your project file should include some windows”

`windows`/`tabs` 必须解析为非空序列：

```yaml
windows:
  - shell:
```

### 在项目根目录处启动失败

请创建该目录，或修正渲染后的模板：

```sh
bootmux debug myapp root=/expected/path
```

tmux start 使用快速失败的 shell 执行。Herdr 需要预期的工作目录来构建窗格。

### Herdr 拒绝 `synchronize`

Herdr 没有对应的同步输入行为。请移除该字段、将它设为真正的 YAML 布尔值
`false`，或维护一个 tmux 专用项目变体：

```yaml
synchronize: false
```

字符串 `"false"` 求值为真，因此会被拒绝。

### Herdr 版本/协议不匹配

```sh
herdr --version
bootmux --backend herdr doctor
```

请升级 Herdr 客户端/服务器组合，使两者都使用受支持的 protocol（17 或 19）
并满足最低版本要求。

### Herdr stop 报告端点或身份不匹配

请使用与启动时相同的模板和套接字设置：

```sh
bootmux --backend herdr stop myapp root=/same/root socket=/same/socket
```

不要通过删除状态或猜测工作区标签来绕过检查。请先检查配置和 `debug` 输出。

### Herdr stop 报告停止钩子已更改

当前渲染的 `on_project_stop` 与受管理快照不同。请恢复先前的配置/设置，或者
先正确识别项目并启动一次以刷新其受管理记录，然后再停止。

不要仅为绕过拒绝而执行新改成的破坏性钩子。

### Herdr append 报告端点错误

请从接收方 Herdr 工作区/弹窗中运行，并移除有冲突的 YAML/环境变量套接字
选择器，或者选择完全相同的端点：

```sh
bootmux --backend herdr debug tools --append
```

### Herdr 拒绝序列化布局

请检查校验和与窗格数量。如果无法转换精确的 tmux 几何结构，请将序列化
字符串替换为可移植预设或显式窗格链。

### 现有工作区没有重新运行命令

这是复用行为。请把恢复逻辑添加到 `on_project_restart`，或者停止后重新启动，
以创建全新拓扑。

### 选择器提示缺少 `fzf`

请安装 `fzf`，或者直接启动项目。除选择器外，所有工作流都不需要它。

### ERB 被拒绝

只保留：

```erb
<%= @settings["key"] %>
```

迁移其他形式：

```text
<%= @args[0] %>  -> {{ args[0] }}
<%= ENV["V"] %>  -> {{ env.V }}
```

### tmux `stop-all` 漏掉了项目

请使用项目的实际配置、名称和套接字显式停止。`stop-all` 不拥有也不枚举每个
可能存在的 tmux 会话。

## 22. 环境变量速查

| 变量 | 用途 |
|---|---|
| `HOME` | 默认配置、项目和状态根目录 |
| `XDG_CONFIG_HOME` | bootmux 设置和 XDG tmuxinator 项目 |
| `XDG_STATE_HOME` | 为绝对路径时，指定 Herdr 所有权状态目录 |
| `TMUXINATOR_CONFIG` | 最高优先级的项目目录 |
| `EDITOR` | 用于 `new`、`open` 和 `edit` |
| `SHELL` | 生成的 shell/钩子环境 |
| `TMUX` | 检测活动 tmux |
| `HERDR_ACTIVE_*` | Herdr 弹窗/当前上下文 |
| `HERDR_CONFIG_PATH` | 覆盖 bootmux 调用 Herdr 命令时使用的 Herdr 配置 |
| `HERDR_ENV` | 检测活动 Herdr |
| `HERDR_WORKSPACE_ID` | 当前 Herdr 工作区 |
| `HERDR_TAB_ID` | 当前 Herdr 标签页 |
| `HERDR_PANE_ID` | 当前 Herdr 窗格/前台进程分类 |
| `HERDR_SOCKET_PATH` | Herdr 端点和活动环境信号 |
| `HERDR_CLIENT_SOCKET_PATH` | 已附着客户端的端点比较 |
| `HERDR_SESSION` | 环境中的 Herdr 命名端点 |

## 23. 命令速查表

```sh
# Validate
bootmux --backend tmux doctor
bootmux --backend herdr doctor
bootmux --backend tmux debug PROJECT
bootmux --backend herdr debug PROJECT

# Lifecycle
bootmux start PROJECT [key=value] [args...]
bootmux stop PROJECT [key=value] [args...]
bootmux stop-all
bootmux local

# Overrides
bootmux start PROJECT --attach
bootmux start PROJECT --no-attach
bootmux start PROJECT --name ALT
bootmux start -p PATH
bootmux start PROJECT --append
bootmux start PROJECT --no-pre-window

# Projects
bootmux new NAME
bootmux --backend tmux new NAME SESSION
bootmux open NAME
bootmux edit NAME
bootmux copy OLD NEW
bootmux delete NAME
bootmux implode
bootmux list --newline

# Picker/settings
bootmux picker
bootmux bindings tmux
bootmux bindings herdr
bootmux config set default-backend herdr
bootmux config get default-backend
bootmux config path

# Diagnostics/shell integration
bootmux version
bootmux commands [SHELL]
bootmux completions ARG
```

每个选项和别名的说明，请参阅 [CLI 参考](cli.md)。每个 YAML 字段和窗格形式的
说明，请参阅[项目配置](configuration.md)。
