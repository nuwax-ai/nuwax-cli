# MySQL 在线架构差异（Live Diff）设计方案

> 目标：在自动升级部署（`nuwax-cli auto-upgrade-deploy run`）阶段，基于“在线 MySQL 实际库表结构 vs 最新 SQL 模板”进行差异比对，生成可执行的差异 SQL 并安全执行，从而保证升级后数据库结构与最新模板完全一致。

## 背景与问题
- 现状：当前使用“旧版 SQL 文件 vs 新版 SQL 文件”的方式，通过 `client-core::sql_diff` 模块生成差异 SQL（见 `generator.rs`、`parser.rs`、`differ.rs`）。
- 缺陷：当线上数据库状态与“旧版 SQL 文件”不一致时，生成的差异可能不准确，从而造成升级偏差。
- 你的要求：严格使用 `sqlparser` 进行 SQL 解析；禁止使用正则表达式来解析 SQL（易出错）。本设计完全遵守这一要求。

## 设计目标
- 在线比对：连接 MySQL，拉取当前真实库表结构，与最新 SQL 模板解析结果进行比较。
- 差异生成：复用现有差异生成器，产出可执行的 MySQL DDL（`CREATE TABLE`、`ALTER TABLE ADD/MODIFY/DROP`、索引增删改）。
- 安全执行：在自动升级流程中执行差异 SQL，记录结果并归档差异文件。
- 严格失败策略：在线架构抓取或差异生成失败时，直接报错并终止升级流程。

## 范围与约束
- 范围：表结构与索引（`CREATE TABLE`、列增删改、PRIMARY/UNIQUE/普通索引增删改）。
- 暂不覆盖：视图、触发器、存储过程、函数、事件、跨数据库的权限语句（后续可扩展）。
- 数据库选择：默认仅针对 `.env/compose` 中的 `MYSQL_DATABASE`；模板中的 `USE ...` 多库支持后续迭代。

## 总体方案
- 模板侧解析：使用 `sqlparser` 的 MySQL 方言解析模板 SQL，得到 `to_tables: HashMap<String, TableDefinition>`。
- 在线侧抓取：通过查询 `INFORMATION_SCHEMA`，构建与模板一致的数据结构 `live_tables: HashMap<String, TableDefinition>`。
- 差异比较：复用 `client-core::sql_diff::differ::generate_mysql_diff(&live_tables, &to_tables)` 生成差异 SQL。
- 执行与归档：沿用现有执行器与归档逻辑（重试、日志、重命名差异文件）。

## 关键原则（严格遵守）
- 解析模板 SQL 时，仅使用 `sqlparser`，不使用正则表达式解析 SQL。
- 在线架构侧不做 SQL 解析，而是查询元数据（`INFORMATION_SCHEMA`），并将其映射为现有的结构体类型。

## MySQL 连接逻辑与使用

### 配置来源与解析
- 配置文件路径：
  - `docker-compose.yml`：通过自动升级流程的 `--config` 入参或默认路径 `client_core::constants::docker::get_compose_file_path()`。
  - `.env` 文件：`client_core::constants::docker::get_env_file_path()`。
- 解析流程：
  - 使用 `DockerManager::new(compose_file, env_file)` 加载 Compose 配置并完成环境变量替换。
  - 定位 `services.mysql`，解析端口映射与环境变量。
  - 环境变量：
    - `MYSQL_USER`（默认 `root`）
    - `MYSQL_PASSWORD`（默认 `root`）
    - `MYSQL_DATABASE`（默认 `agent_platform`）
- 端口映射解析（容器端口 `3306`）：
  - 短格式：如 `ports: ["13306:3306"]`，取 `13306` 作为主机端口。
  - 长格式：对象形式，寻找 `target = 3306` 的 `published` 值。

### 连接 URL 生成
- 固定主机：`127.0.0.1`
- 格式：`mysql://{user}:{password}@{host}:{port}/{database}`
- 在 `client-core/src/mysql_executor.rs` 中由 `MySqlConfig::to_url()` 生成。

### 执行器能力
- 构造与连接：
  - `let config = MySqlConfig::for_container(Some(compose), Some(env)).await?;`
  - `let executor = MySqlExecutor::new(config);`
  - 连接测试：`executor.test_connection().await?`（执行 `SELECT 1`）。
- 差异 SQL 执行：
  - `executor.execute_diff_sql_with_retry(sql, retries).await`（逐条执行、失败回滚尝试、带重试日志）。
  - 事务说明：DDL 多为隐式提交，执行器采用逐条执行与重试策略。

### 在自动升级流程中的用法（严格失败策略）
```rust
// 获取 compose/.env 路径
let compose_file = get_compose_file_path(&config_file);
let env_file = client_core::constants::docker::get_env_file_path();

let compose_file_str = compose_file.to_str().ok_or_else(|| anyhow::anyhow!("compose 路径无效"))?;
let env_file_str = env_file.to_str().ok_or_else(|| anyhow::anyhow!(".env 路径无效"))?;

// 构造连接配置与执行器
let config = MySqlConfig::for_container(Some(compose_file_str), Some(env_file_str)).await?;
let executor = MySqlExecutor::new(config);

// 严格失败策略：连接失败直接报错退出
executor.test_connection().await?;

// 生成差异 SQL（live vs 模板），并执行
let diff_sql = /* 通过 generate_mysql_diff(live_tables, to_tables) 生成 */;
executor.execute_diff_sql_with_retry(&diff_sql, 3).await?;
```

### 常见问题定位
- “未找到 `mysql` 服务”：检查 `docker-compose.yml` 中服务名是否为 `mysql`。
- “未找到到容器端口 3306 的映射”：确认 `ports` 映射包含 `3306`。
- 连接失败：确保容器运行、主机端口（如 `13306`）可达，账号密码与数据库名正确。


## 模块变更与接口设计

### 1. 在线架构抓取（新增）
文件：`client-core/src/mysql_executor.rs`

新增接口：
```rust
pub async fn fetch_live_schema(&self) -> Result<HashMap<String, TableDefinition>, anyhow::Error>;
```

实现要点：
- 基于现有 `MySqlExecutor` 连接池，查询以下系统表：
  - `INFORMATION_SCHEMA.TABLES`：`TABLE_NAME`、`ENGINE`、`TABLE_COLLATION`
  - `INFORMATION_SCHEMA.COLUMNS`：`COLUMN_NAME`、`COLUMN_TYPE`、`IS_NULLABLE`、`COLUMN_DEFAULT`、`EXTRA`、`COLUMN_COMMENT`
  - `INFORMATION_SCHEMA.STATISTICS`：`INDEX_NAME`、`NON_UNIQUE`、`SEQ_IN_INDEX`、`COLUMN_NAME`
- 字段映射到 `sql_diff::types`：
  - `TableDefinition { name, columns, indexes, engine, charset }`
  - `TableColumn { name, data_type, nullable, default_value, auto_increment, comment }`
  - `TableIndex { name, columns, is_primary, is_unique }`
- 细节处理：
  - `auto_increment`：当 `EXTRA` 含 `auto_increment` 时为真。
  - `nullable`：`IS_NULLABLE = 'YES'`。
  - `charset`：由 `TABLE_COLLATION` 推断，如 `utf8mb4_0900_ai_ci` → `utf8mb4`（取下划线前缀），失败回退 `utf8mb4`。
  - 索引列顺序：按照 `SEQ_IN_INDEX` 排序。
  - 主键识别：`INDEX_NAME = 'PRIMARY'` → `is_primary = true`。
  - 唯一索引：`NON_UNIQUE = 0` → `is_unique = true`。

### 2. 模板解析（复用）
文件：`client-core/src/sql_diff/parser.rs`
- 使用 `sqlparser::dialect::MySqlDialect` 解析 `CREATE TABLE` 等，得到 `TableDefinition`/`TableColumn`/`TableIndex`。
- 禁止使用正则表达式解析 SQL（现有逻辑已符合）。

### 3. 差异生成（复用）
文件：`client-core/src/sql_diff/differ.rs`
- 复用 `generate_mysql_diff(from_tables, to_tables)`，输出 DDL 迁移脚本：
  - 新增表：`CREATE TABLE ...`。
  - 删除表：`DROP TABLE IF EXISTS ...`。
  - 列新增/删除/变更：`ALTER TABLE ... ADD/MODIFY/DROP COLUMN`。
  - 索引新增/删除/变更：`ALTER TABLE ... ADD/DROP KEY/PRIMARY/UNIQUE`。

### 4. 自动升级流程接入（改造）
文件：`nuwax-cli/src/commands/auto_upgrade_deploy.rs`
- 位置：`execute_sql_diff_upgrade(config_file: &Option<PathBuf>)`。
- 新逻辑（优先在线差异）：
  1. 读取模板 SQL（`temp_sql/init_mysql_new.sql` 或 `docker/config/init_mysql.sql`），调用 `parse_sql_tables(&new_sql_content) -> to_tables`。
  2. 构造连接：`MySqlConfig::for_container(...)` → `MySqlExecutor::new(config)`。
  3. 在线抓取：`let live_tables = executor.fetch_live_schema().await?;`
  4. 生成差异：`let diff_sql = generate_mysql_diff(&live_tables, &to_tables)?;`
  5. 保存并执行：写入 `temp_sql/upgrade_diff.sql`，调用 `execute_diff_sql_with_retry(&diff_sql, 3).await`。
  6. 执行成功后重命名归档（`diff_sql_executed_{timestamp}.sql`）。
 - 失败策略：在线抓取或连接失败 → 直接报错退出，不执行文件差异路径。

### 5. CLI 控制（可选）
文件：`nuwax-cli/src/cli.rs`
- 为 `AutoUpgradeDeployCommand::Run` 增加：`--diff-source <auto|live|file>`，默认 `auto`。
- 为 `diff-sql` 增加：`--live` 开关，支持直接生成“在线 vs 模板”的差异文件。

## 流程说明（文本版）
1. 解析模板 SQL（`sqlparser`）→ `to_tables`。
2. 连接 MySQL 并抓取在线结构（`INFORMATION_SCHEMA`）→ `live_tables`。
3. 调用差异生成器 → `diff_sql` 与描述信息。
4. 差异为空 → 归档空差异文件；否则执行差异 SQL。
5. 执行成功 → 归档差异文件；执行失败 → 报错并保留差异文件以便诊断。

## 幂等与安全策略
- 尽量使用安全语句：`DROP TABLE IF EXISTS`、索引删除前检查。
- 事务注意：MySQL DDL 通常隐式提交；现有执行器的事务回滚不一定生效，执行器使用逐条执行与重试（`execute_diff_sql_with_retry`）。
- 生成器不直接做数据迁移（仅结构变更）；涉及数据迁移的需求后续另行设计。

## 错误处理
- 数据库不可达/权限不足/抓取失败：直接报错并终止升级流程。
- 模板解析失败：直接报错（`sqlparser` 语法错误），不执行升级。
- 差异执行失败：停止并记录失败日志，保留差异文件用于排查。

## 测试计划
- 单元测试：
  - `fetch_live_schema` 映射正确性（列、索引、主键/唯一、charset/engine）。
  - `live vs 模板` 差异：新增/删除表、列类型变更、索引增删改。
- 集成测试：
  - 使用 `client-core/fixtures/docker-compose.yml` 与 `.env` 启动容器，导入旧结构，执行自动升级，最终结构与模板一致。
  - 模拟数据库不可达，验证直接报错并终止流程（不生成文件差异）。
- 语法校验：复用 `sql_syntax_validation_test.rs` 对生成差异做基本校验。

## 开发清单（分步）
1. 在 `mysql_executor.rs` 实现 `fetch_live_schema`（查询 `INFORMATION_SCHEMA` 并映射）。
2. 在 `auto_upgrade_deploy.rs` 接入在线差异生成与执行，失败直接报错。
3. （可选）扩展 CLI：`--diff-source`、`--live` 控制。
4. 增补测试用例与文档（README 与本设计）。

## 里程碑与评估
- 开发与单测：1–2 天。
- 集成与回归：0.5–1 天。
- 后续扩展（视图/触发器/多库）按需排期。

## 结论
- 该方案对现有架构影响小：复用 `sqlparser` 和差异生成器，仅新增在线抓取与流程接入。
- 满足“禁止正则解析 SQL”的要求：模板解析全程使用 `sqlparser`；在线结构来源于元数据查询。
- 能有效解决“线上结构与旧文件不一致”导致的差异不准问题，提升升级的准确性与可靠性。