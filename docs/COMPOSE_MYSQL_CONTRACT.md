# Compose 多库部署、配置保留与回滚

本说明对应 `mysql-schema-manifest-v1` 能力。CLI 在停服务、替换文件和修改数据库之前，校验候选包、合并后的用户配置、应用连接映射、initdb 挂载顺序、产物完整性及交付身份。MySQL 就绪后始终执行幂等建库、精确授权和各库 Live Diff；首次安装数据由 MySQL 的空数据目录初始化机制控制，不在升级时重放。

## 部署上下文

自动包部署的实际解压根为当前工作目录的 `docker/`，Compose 文件必须是 `docker/docker-compose.yml`。暂不支持把自动解压包应用到另一个自定义 Compose 文件；这类覆盖会在下载/停服或解压之前明确拒绝。

`config.toml` 的 `[docker].env_file` 可以指向独立的普通配置文件，例如 `docker/secrets/operator.env` 或外部目录。该路径会在版本保存和回滚后保留。预检、配置合并和实际运行都使用这一选定文件；不会从 env 文件的父目录推断包根。

用户已有配置完整保留，只补充新包未定义的键。shell 对普通变量的覆盖优先于 env 文件，显式空值同样是覆盖。必填语法遵循 Compose：`${KEY?}` 允许已定义的空值，`${KEY:?}` 拒绝空值，`$$` 及未选中的条件分支不会被误判。CLI 管理的设备字段仍以选定文件为准。

环境文件通过现有原子文件写入器替换，保留权限及可支持的 symlink，不会先删除有效目标。用户现有的 BOM、多行引号、裸键和冒号赋值由共享解析器识别。包内多行 quoted 默认值暂不支持自动补键，会在停服前拒绝；打包时请改用单行转义值。离线目录整体替换不支持存放在包根内或通过文件/父目录 alias 指向包根内的 env symlink，同样提前拒绝；普通 env 文件可支持，外部 alias 的真实目标必须位于将被移动的包根之外，包根上方的系统目录 alias 不受影响。

## 完整包和增量包

存在 schema manifest 时，离线入口也按 `schemas` 声明的路径与数据库身份验证，不依赖固定两个 SQL 文件名。非法或不支持的 manifest 立即失败，不回退 legacy。

ZIP 增量包必须携带完整的关键交付上下文：Compose、DELIVERY、schema manifest 及其引用文件，以及 DELIVERY 声明的组件产物。即使 patch operation 清单遗漏这些文件，解压器也会应用已预检的完整关键文件集合。TAR.GZ 目前用于完整包；TAR.GZ 增量包在候选预检时拒绝。

资料库的必要入口是非空普通文件 `repo-collab-app/dist/index.js`；IM 需要两个非空 bootstrap jar。空目录、只有 README、重复普通条目、符号链接入口和与 receipt 不一致的产物不能作为完整交付或完整备份。

## 冷备份与回滚

新的交付清单部署使用 release-aware 冷备份，保存原始 Compose、配置、schema、组件产物、原始 DELIVERY 文件和 `BACKUP_RELEASE.json`。交付清单保持原样，不为混合文件重算或伪造 release 身份。备份记录本地不可变 runtime image ID，不把镜像 tar 再复制进备份。

恢复时先在临时目录校验完整性、文件指纹、配置连接、必要入口、MySQL 数据挂载和本地历史镜像是否可用，成功后才停服务。恢复对应配置/产物与本地镜像标签，用户 env 保留，并保存恢复后的服务版本。历史镜像被清理时，请先载入相应完整包的镜像；CLI 不会替换为其他版本的 latest 或静默拉取替代镜像。

- 默认应用回滚保留当前数据库，要求保存的 MySQL schema/config 文件指纹一致，并且当前与备份的 MySQL 数据挂载路径一致；注释变化也会触发这一保守检查。
- `--rollback-data` 需要匹配的冷数据，恢复相应数据库版本。当前冷备只覆盖包根 `data/` 内的明确 MySQL bind，例如 `./data/mysql` 或 `./data/custom-mysql`；named volume、外部 MySQL 数据路径或指向外部的数据 symlink 会在停服前拒绝。不能用归档中其他数据库/Redis 的文件冒充 MySQL 冷数据。
- 用户 env 不包含在 release snapshot 中。恢复历史 MySQL 冷数据时应使用该数据对应的数据库账户/密码；仅改 env 不会重置数据库内的凭据。
- 有 DELIVERY 的部署不能用缺少完整 release context 的旧应用备份执行混合回滚；请使用匹配完整包及冷备恢复。双方都没有 DELIVERY 的旧部署保留明确 legacy 路径。

当前支持同一 Compose MySQL 服务里的多个 database。业务 Java/Node/前端功能验收仍需使用真实业务发布包；小型隔离 Compose 与 SQL 测试验证的是 CLI 的部署、迁移和回滚控制流程。
