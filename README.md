# Roc Request Bin

使用 Rust + [GPUI Kit](https://github.com/longbridge/gpui-kit) 构建的原生 HTTP 请求收集工具，支持 Windows、macOS Intel 和 Apple Silicon。

## 使用

从 [Releases](https://github.com/roc198419/roc-request-bin/releases) 下载对应系统的压缩包。Windows 解压后运行 `roc-request-bin.exe`；macOS 下载 `Roc-Request-Bin-*.zip`，将 `.app` 移到应用程序目录。

应用标题为 **Roc Request Bin**。点击工具栏右侧地址可修改 IP 和端口，默认 `127.0.0.1:8787`，点击「启动监听」后发送请求：

```sh
curl -X POST http://127.0.0.1:8787/webhooks/payment \
  -H 'Content-Type: application/json' \
  -d '{"event":"payment.completed","id":42}'
```

左栏新增规则可填写名称、请求路径前缀和多个 HTTP Methods。规则列表将名称、Methods、路径前缀及请求数量放在同一行。

匹配时忽略 URL 的 scheme、host 和端口，仅比较路径与查询字符串。例如 `/api/orders` 可匹配任意主机的 `/api/orders?state=paid`。输入完整 HTTP URL 时会自动提取路径及查询字符串；去除首尾空白后的这部分前缀必须唯一，Methods 不参与去重。匹配按字面、区分大小写，不将 `%` 或 `_` 视为通配符。

规则创建时也会匹配历史请求。一条请求可以命中多条规则。「未匹配」统计未命中任何现存规则的请求。删除规则保留请求；批量删除仅作用于勾选项。旧数据库会自动移除 `description` 字段并将原来的完整 URL 前缀迁移为路径前缀；若多个旧规则的路径相同，将合并 Methods 并保留最早规则的名称和 ID，重新计算历史匹配，请求数据保留。

右栏每页 50 条，默认按请求时间倒序排列，相同时间按 ID 倒序。列表、详情和复制内容均显示 UTC+8（北京时间），日期和时间在同一行；数据库保持 UTC 存储。点击请求打开右侧抽屉，支持文本选择及复制完整请求。列表只加载摘要，打开详情时才读取 Body。原始 Body 以 BLOB 保存，二进制内容以 Base64 展示；重复请求头保留，头值按 Base64 存储以避免字节丢失。Body 上限 16 MiB，超出返回 413。HTTP 解析会规范化头名称，不保留线上报文的原始大小写和字节排版。

分隔栏可拖动，默认左栏占 30%，比例与监听配置均持久化至用户目录 `~/.roc-request-bin/data.db`。布局有最小面板宽度，窗口过窄时会约束比例。规则和请求也存储在同一 SQLite 数据库。

这是接收直接发送到监听地址的 HTTP 请求的工具，返回 200；不自动抓取系统网络流量，不转发上游，也不解密 HTTPS。默认只监听本机，可改为 `0.0.0.0` 或其他本机 IP 供其他设备访问。

## 系统托盘

Windows 和 macOS 启动时会尝试创建系统托盘/菜单栏图标，菜单支持显示窗口、收起窗口、启动/停止监听和退出。Windows 左键单击图标显示主窗口，右键打开菜单；macOS 点击菜单栏图标打开菜单。托盘菜单和工具栏的监听状态同步。

托盘可用时，关闭窗口会收起到托盘，监听继续运行；通过托盘「退出 Roc Request Bin」彻底退出。托盘创建失败时显示错误提示，保留普通窗口关闭行为。

## 自动更新

每次启动在后台检查本仓库的 GitHub Releases，发现更高正式版本即下载对应架构的更新包并替换可执行文件，下次启动生效。底部状态栏显示当前版本与更新状态。网络错误不会阻止收集请求。macOS 应用需要放在用户可写的位置；压缩包采用临时签名，尚未配置 Apple Developer 公证或 Windows 代码签名，首次运行可能需要操作系统手动放行。更新当前依赖 GitHub HTTPS 分发，尚无独立的发布签名校验。

## 开发

需要 Rust 1.96+、Windows MSVC C++ 构建工具/Windows SDK，或 macOS Xcode Command Line Tools。

```sh
cargo run
cargo test --no-default-features
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

`cargo test --no-default-features` 无需 GUI，验证实际 HTTP 收集、二进制 Body、重复头、规则去重、历史回填、删除级联、分页及重启持久化。

图标源文件为 `assets/app.svg`；PNG、ICO、ICNS 已提交。修改图标后可安装 Pillow 并运行 `python scripts/generate_icons.py` 重新生成原生图标容器。

## 发布

GitHub Actions 在推送 main / PR 时执行格式、测试、Clippy 和 release 构建，覆盖 Windows x64、macOS x64、macOS arm64。打 `v` 开头的版本标签会在所有构建成功后创建 GitHub Release。

1. 修改 `Cargo.toml` 的版本号并更新 `Cargo.lock`。
2. 提交并推送 main。
3. 创建与 Cargo 版本一致的标签，例如 `git tag v0.2.0`，然后 `git push origin v0.2.0`。

`roc-request-bin-<target>.zip` 是自动更新包；`Roc-Request-Bin-<target>.zip` 是 macOS `.app` 分发包。
