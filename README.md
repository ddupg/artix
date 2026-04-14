# Artix

`artix` 是一个面向开发者工作区的磁盘清理工具，当前实现的是 Phase 1。

它的目标不是做通用磁盘分析，而是帮你在一堆 Rust、Node.js、Python 项目里，快速找出那些“体积很大、通常可重建、相对安全可删”的目录，比如：

- Rust 的 `target/`
- Node.js 的 `node_modules/`
- Python 的 `.venv/`

当前这版已经能扫描工作区、按项目聚合候选目录、输出一个最小总览，并提供基础删除后端。

## 当前功能

- 扫描一个或多个工作区根目录
- 识别基础规则：
  - `target/`
  - `node_modules/`
  - `.venv/`
- 按最近项目根归属候选目录，支持基础 workspace / monorepo 场景
- 汇总每个项目的：
  - 项目名
  - 可回收空间
  - 候选目录数量
- 默认无参数时扫描当前目录
- 提供两种删除模式：
  - 丢进系统废纸篓
  - 显式确认后的永久删除

## 当前还没做的

这版还是 Phase 1，故意没把范围做散。

- 还没有真正的交互式 TUI 界面
- 还没有项目详情页 / 删除确认页 UI
- 还没有增量扫描刷新
- 还没有更强的 Git ignore 解析
- 还没有 Homebrew 分发
- 还没有删除进度、逐项结果模型、失败重试

## 如何运行

### 1. 构建

```bash
cargo build
```

### 2. 扫描当前目录

```bash
cargo run --quiet
```

输出格式是：

```text
<project_name>\t<reclaimable_bytes>\t<candidate_count>
```

例如：

```text
artix	123456	2
```

### 3. 扫描指定目录

```bash
cargo run --quiet -- /path/to/workspace
```

也可以传多个根目录：

```bash
cargo run --quiet -- /path/to/ws1 /path/to/ws2
```

## 删除能力

当前删除能力已经实现到库层，但还没有接到交互式 UI。

代码入口在：

- [src/delete.rs](/Users/bytedance/opensource/artix/src/delete.rs)

语义如下：

- `DeleteMode::Trash`
  - 走系统废纸篓
- `DeleteMode::Permanent { confirmed: true }`
  - 永久删除
- `DeleteMode::Permanent { confirmed: false }`
  - 直接报错，不允许执行

## 测试

运行全部测试：

```bash
cargo test --all-targets
```

当前测试覆盖了：

- 规则表默认值
- workspace / monorepo 归属
- Rust / Node / Python 的扫描归属主路径
- 删除失败路径
- CLI 默认行为

## 发布

仓库里已经有一个 Phase 1 的 GitHub Actions release workflow：

- [release.yml](/Users/bytedance/opensource/artix/.github/workflows/release.yml)

它会在打 `v*` tag 时：

- 在 Linux / macOS runner 上构建目标产物
- 打包成 `.tar.gz`
- 上传 artifact
- 发布到 GitHub Release

## 已知边界

- Git 状态判断目前比较浅，某些父级或全局 ignore 规则可能只会得到 `Unknown`
- 扫描是全递归实现，重叠 root 还没有做去重
- 当前 CLI 是最小总览，不是最终产品形态

## 后续方向

接下来最自然的两条路是：

1. 做真正的交互式 TUI，总览页、详情页、确认页接起来
2. 先补 Phase 1.1，把 Git ignore 判断和扫描性能做扎实
