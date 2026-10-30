# 修复验证记录（2026-10-08）

## 范围和兼容性

完成设备指纹/.env 审查中的全部 CLI 问题，并对接 build-agent-docker 的生产 Compose。
设备 v1 算法、字段顺序、已有身份、冻结时间、文件位置和发布版本号保持兼容。
修复另覆盖真实验收发现的 BOM、符号链接、YAML 特殊字符密码、二次插值和默认值语义。

## 结果

| 验证 | 结果 |
|---|---|
| cargo nextest run --workspace --locked --no-fail-fast | 330 通过，1 个原有用例忽略 |
| cargo fmt --all -- --check | 通过 |
| cargo clippy --workspace --locked -- -D warnings | 通过，无警告 |
| cargo check --workspace --locked | 通过 |
| 与原生 Docker Compose 插值对比 | 40 种 unset/empty/default/required/alternate/nested/literal 场景一致 |
| 生产 Compose 完整类型解析 | 实际 docker-compose-types 0.24 与原生 Compose 均解析 21 个服务 |
| 最终 CLI 与真实 DockerManager 容器验收 | 12 项断言通过，15 个设备变量正确 |
| 原子写入故障 | 部分写入失败后旧指纹可读且字节未变，临时文件清理 |

实际容器验收直接使用最终 CLI 二进制和生产 client-core 依赖，不使用类型/采集替身。
验证了 BOM 与 symlink 保留、非托管原文不变、旧字段清除、重复注入稳定、旧进程设备值被覆盖、
非托管宿主覆盖继续生效，以及包含美元符号、占位符字面量、引号、#、冒号和反斜线的密码完整到达容器。
验收项目最终剩余容器、网络和卷均为 0；测试 MySQL 使用 TEST_MYSQL_CLEANUP=1 清理。

## 跨平台与发布

新增只读 Linux/macOS/Windows 测试工作流，可独立运行或作为 reusable workflow 调用。
稳定版、Beta 和 crates 发布流程都在版本检查/发布操作之前依赖该检查；未使用发布密钥。
本机完成 macOS 验收；GitHub-hosted Linux/Windows runner 尚未执行，必须在发布前通过门禁。
本轮未增加发布版本号，正式发布前单独递增补丁号并先走 beta。

## 重跑

```bash
cargo fmt --all -- --check
cargo clippy --workspace --locked -- -D warnings
cargo check --workspace --locked
TEST_MYSQL_CLEANUP=1 cargo nextest run --workspace --locked --no-fail-fast
```

原有被忽略用例要求 TEST_MYSQL_URL 指向一次性实例，属于 DDL 失败恢复测试；本次正常 MySQL 集成测试已通过。
相关部署、MC 镜像同步和 MQ 实测记录位于 build-agent-docker 的 docs/FIXES_VALIDATION_2026-10-08.md。
