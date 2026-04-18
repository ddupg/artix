# Artix

`artix` 是一个面向开发者工作区的磁盘清理 TUI。

它不是通用磁盘分析器。它要解决的是更窄、更烦、更高频的问题：

- 你本地有很多 repo、很多 `git worktree`
- 里面有 Rust / Node / Python 之类的混合项目
- 真正占空间的往往不是代码，而是 `target/`、`node_modules/`、`.venv/` 这类可重建目录
- 但你又不想手动判断哪些能删，哪些碰了会出事

`artix` 的目标，就是把 `ncdu` 的浏览手感，加上 Git/worktree 语义和安全删除。

## 当前功能

### 交互式 TUI

- 默认在交互终端里启动 TUI
- 左侧是按大小排序的 `目录 + cleanup candidates` 混合列表
- 第一行提供 `..` 返回上级目录
- 右侧显示当前项的上下文信息：
  - Git root
  - worktree root
  - 分支名
  - 目录大小 / 可回收空间
  - 候选目录类型

### Git / worktree 感知

- 进入 Git 目录时，标题会显示当前分支
- 列表中如果某一项本身是 repo root 或 worktree root，也会显示分支
- 支持解析 `git worktree` 使用的 `.git` 文件形式，不只认 `.git/` 目录

### Git-aware 过滤

支持 4 种过滤模式：

- `All`
- `Cleanup Focus`
- `Ignored Only`
- `Untracked + Ignored`

其中 `Cleanup Focus` 会隐藏 tracked 内容，让真正的垃圾目录浮出来。

### 安全删除

- `d` 进入删除确认
- `t` 走系统废纸篓
- `x` 走永久删除
- tracked / unknown 目标会进入更强确认路径
- 删除后会局部失效并刷新当前目录，而不是全盘重扫

### 规则识别

当前内置的候选目录规则：

- Rust 的 `target/`
- Node.js 的 `node_modules/`
- Python 的 `.venv/`

## 配置

`artix` 现在使用 `TOML` 配置文件，而不是把运行时配置散落在环境变量里。

### 配置文件路径

- 默认路径：`~/.config/artix/config.toml`
- 兼容旧路径：`~/.artix/config.toml`

读取顺序：

1. `~/.config/artix/config.toml`
2. `~/.artix/config.toml`

如果命中了 `~/.artix/config.toml`，`artix` 会继续读取，但会输出 warning，提示迁移到 `~/.config/artix/config.toml`。

### 生成默认配置

- 初始化默认配置到平台主路径：`artix init-config`
- 仅把默认配置打印到 stdout：`artix --print-default-config`

例如你想先预览再手动写入：

```bash
artix --print-default-config
```

`init-config` 只会在还没有现有配置文件时写入；如果上述任一路径下已经存在配置文件，它会直接报错，避免覆盖现有配置。

### 配置示例

```toml
version = 1

[ui]
mode = "auto"   # auto | plain | tui
icons = true

[performance]
fs_concurrency = 8
git_concurrency = 4
tui_entry_concurrency = 8

[scan.tui_size_budget]
max_entries = 1000000
timeout_ms = 3000

[delete]
trash_backend = "auto"  # auto | builtin
```

### 当前支持的配置项

- `[ui].mode`
  - `auto`：交互终端里默认进 TUI；非交互终端走纯文本模式
  - `plain`：总是走纯文本模式
  - `tui`：总是尝试进入 TUI
- `[ui].icons`
  - 是否启用带 Nerd Font 的图标
- `[performance].fs_concurrency`
  - 文件系统相关并发度；默认 `available_parallelism * 2`，并 clamp 到 `[2, 16]`
- `[performance].git_concurrency`
  - Git 子进程并发度；默认 `available_parallelism`，并 clamp 到 `[2, 8]`
- `[performance].tui_entry_concurrency`
  - TUI 目录项后台补全并发度；默认 `available_parallelism * 2`，并 clamp 到 `[4, 32]`
- `[scan.tui_size_budget].max_entries`
  - TUI 中单目录 size 预算允许扫描的最大 entry 数；设为 `0` 表示不限制
- `[scan.tui_size_budget].timeout_ms`
  - TUI 中单目录 size 预算超时；设为 `0` 表示不超时
- `[delete].trash_backend`
  - `auto`：优先系统 trash，macOS 上失败时回退到内置 `~/.Trash`
  - `builtin`：直接走内置 `~/.Trash`

## 如何运行

### 1. 构建

```bash
cargo build
```

### 2. 启动 TUI

默认无参数时，从当前目录启动：

```bash
cargo run --quiet
```

也可以指定目录：

```bash
cargo run --quiet -- /path/to/workspace
```

### 3. 纯文本模式

如果你在脚本里调用，或者只想保留旧的文本总览输出，推荐在配置文件里设置：

```toml
[ui]
mode = "plain"
```

也可以在非交互终端下直接运行；此时 `auto` 模式会自动退回纯文本输出。

输出格式仍然是：

```text
<project_name>\t<reclaimable_bytes>\t<candidate_count>
```

### 4. 主要按键

- `j` / `k` 或上下键：移动选择
- `g`：跳到第一条
- `G`：跳到最后一条
- `Enter` / `l` / 右键：进入目录
- `Backspace` / `h` / 左键：返回上级目录
- `f`：切换过滤模式
- `d`：打开删除确认
- `t`：确认移动到废纸篓
- `x`：请求永久删除
- `y`：确认高风险永久删除
- `Esc`：关闭弹窗
- `q`：退出

## 实现概览

核心模块：

- [src/main.rs](/Users/bytedance/opensource/artix/src/main.rs)
  入口。先加载配置文件，再决定进入 TUI 还是纯文本模式。
- [src/config.rs](/Users/bytedance/opensource/artix/src/config.rs)
  统一配置入口，负责默认值、配置文件路径解析和 `TOML` 反序列化。
- [src/ui/mod.rs](/Users/bytedance/opensource/artix/src/ui/mod.rs)
  TUI 状态、过滤模式、渲染和交互主循环；运行时通过 `AppContext` 共享配置与并发控制。
- [src/delete_flow.rs](/Users/bytedance/opensource/artix/src/delete_flow.rs)
  删除状态机和删除动作执行。
- [src/classify/git.rs](/Users/bytedance/opensource/artix/src/classify/git.rs)
  Git/worktree 上下文解析，以及目录的 Git 状态分类。
- [src/scan/mod.rs](/Users/bytedance/opensource/artix/src/scan/mod.rs)
  扫描、项目汇总，以及目录浏览条目生成。
- [src/scan/size.rs](/Users/bytedance/opensource/artix/src/scan/size.rs)
  目录大小计算，以及 TUI 预算化 size 统计。
- [src/model.rs](/Users/bytedance/opensource/artix/src/model.rs)
  领域模型和 `BrowserEntry` 视图模型。

## 测试

运行全部测试：

```bash
cargo test --all-targets
```

当前测试覆盖：

- 规则表默认值
- workspace / monorepo 归属
- Git worktree 分支解析
- 目录浏览排序和 `..` 行为
- 过滤模式行为
- 删除确认风险分支
- 配置文件解析、默认值和兼容路径选择
- 内置 trash backend 配置化回归
- 纯文本 CLI fallback

新增的关键测试文件：

- [tests/git_context_test.rs](/Users/bytedance/opensource/artix/tests/git_context_test.rs)
- [tests/browser_test.rs](/Users/bytedance/opensource/artix/tests/browser_test.rs)
- [tests/ui_state_test.rs](/Users/bytedance/opensource/artix/tests/ui_state_test.rs)

## 发布

仓库里已经有 GitHub Actions release workflow：

- [release.yml](/Users/bytedance/opensource/artix/.github/workflows/release.yml)

它会在打 `v*` tag 时：

- 在 Linux / macOS runner 上构建目标产物
- 打包成 `.tar.gz`
- 上传 artifact
- 发布到 GitHub Release

## 已知边界

- repo / worktree 发现已经用 `gix`，但逐路径的 tracked / ignored 判定目前仍通过本机 `git` 命令完成
- 当前列表只显示目录和 cleanup candidates，不做全文件浏览
- 规则集目前仍然很小，重点先放在常见开发垃圾目录
- 还没有扫描中的增量刷新
- 还没有 Homebrew tap

## 后续方向

最自然的后续增强是：

1. 扩展规则覆盖面，比如更多语言和构建缓存目录
2. 把逐路径 Git 状态判定进一步收成纯 Rust matcher
3. 继续打磨 TUI 视觉层次和右侧 context pane 的信息密度
