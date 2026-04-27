# 发布规范

当前发布版本为 `v0.1.0`。Cargo workspace 版本保持 `0.1.0`，GitHub tag 和 Release 名称统一使用 `v0.1.0`。

## 发布顺序

1. 确认 `main` 分支 CI 通过。
2. 确认 `Cargo.toml` 中的 workspace version 为 `0.1.0`。
3. 确认 `CHANGELOG.md` 已把 `0.1.0` 标记为正式发布版本。
4. 在 GitHub 创建并发布 Release：`v0.1.0`。
5. Release 发布完成后，`.github/workflows/release.yml` 自动开始编译。
6. 编译完成后，workflow 将二进制压缩包上传回同一个 GitHub Release。

也就是说，发布规范要求先发布 `v0.1.0` Release，再由 release 事件触发二进制构建；不要在 Release 发布前手工上传二进制。

## 自动生成的二进制

`release` workflow 会构建 `sradb` CLI，并生成这些 Release assets：

- `sradb-v0.1.0-x86_64-unknown-linux-musl.tar.gz`
- `sradb-v0.1.0-aarch64-unknown-linux-musl.tar.gz`
- `sradb-v0.1.0-x86_64-apple-darwin.tar.gz`
- `sradb-v0.1.0-aarch64-apple-darwin.tar.gz`
- `sradb-v0.1.0-x86_64-pc-windows-msvc.zip`

Linux assets 使用 MUSL target 构建，避免依赖发布 runner 的 glibc 版本。不要发布 `*-unknown-linux-gnu` 资产，除非后续有明确的最低 glibc 兼容策略。

每个压缩包包含：

- `sradb` 或 `sradb.exe`
- `README.md`
- `CHANGELOG.md`

## 本地发布前检查

发布前本地至少运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --all-targets
cargo test --workspace
```

如果需要确认 release 构建命令本身可用：

```bash
cargo build --release --locked -p sradb-cli
./target/release/sradb info
```

## 失败处理

如果 `release` workflow 在 Release 发布后失败：

1. 修复 `main` 上的问题。
2. 通过 `workflow_dispatch` 重新运行 release workflow，并传入对应 tag，例如 `v0.1.0`。
3. workflow 会使用 `gh release upload --clobber` 覆盖同名 asset。

不要删除并重建 `v0.1.0` tag，除非该 tag 指向了错误 commit。
