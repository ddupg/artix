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

如果你在脚本里调用，或者只想保留旧的文本总览输出，可以设置：

```bash
ARTIX_PLAIN=1 cargo run --quiet -- /path/to/workspace
```

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
  入口。交互终端默认进 TUI，`ARTIX_PLAIN=1` 走纯文本 fallback。
- [src/ui/mod.rs](/Users/bytedance/opensource/artix/src/ui/mod.rs)
  TUI 状态、过滤模式、渲染和交互主循环。
- [src/delete_flow.rs](/Users/bytedance/opensource/artix/src/delete_flow.rs)
  删除状态机和删除动作执行。
- [src/classify/git.rs](/Users/bytedance/opensource/artix/src/classify/git.rs)
  Git/worktree 上下文解析，以及目录的 Git 状态分类。
- [src/scan/mod.rs](/Users/bytedance/opensource/artix/src/scan/mod.rs)
  扫描、项目汇总，以及目录浏览条目生成。
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
