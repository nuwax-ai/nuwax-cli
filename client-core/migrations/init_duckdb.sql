-- DuckDB 数据库初始化脚本
-- 支持Duck Client的自动备份、自动升级部署等功能
-- 针对DuckDB并发特性优化设计

-- ========================================
-- 统一配置表（合并 config 和 ui_settings，读多写少，适合并发读取）
-- ========================================
CREATE TABLE IF NOT EXISTS app_config (
    config_key VARCHAR PRIMARY KEY,
    config_value JSON NOT NULL, -- 使用JSON支持复杂数据类型
    config_type VARCHAR NOT NULL, -- STRING/NUMBER/BOOLEAN/OBJECT/ARRAY
    category VARCHAR NOT NULL DEFAULT 'general', -- system/ui/backup/upgrade/docker/network等
    description TEXT, -- 配置项描述
    is_system_config BOOLEAN DEFAULT FALSE, -- 是否为系统配置（不可删除）
    is_user_editable BOOLEAN DEFAULT TRUE, -- 用户是否可编辑
    validation_rule TEXT, -- 验证规则（JSON Schema或正则表达式）
    default_value JSON, -- 默认值
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 添加索引提高并发读取性能
CREATE INDEX IF NOT EXISTS idx_app_config_category ON app_config(category);
CREATE INDEX IF NOT EXISTS idx_app_config_system ON app_config(is_system_config);
CREATE INDEX IF NOT EXISTS idx_app_config_editable ON app_config(is_user_editable);

-- ========================================
-- UI 应用状态管理表（简化版，只记录状态不更新进度）
-- ========================================

-- 应用状态表（单例模式，使用UPSERT避免并发冲突）
CREATE TABLE IF NOT EXISTS app_state (
    id INTEGER PRIMARY KEY DEFAULT 1, -- 只有一条记录
    current_state VARCHAR NOT NULL DEFAULT 'UNINITIALIZED', -- 状态枚举
    state_data JSON, -- 状态相关数据（JSON格式，灵活存储）
    last_error TEXT, -- 最后一次错误信息
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    
    -- 确保只有一条记录
    CHECK (id = 1)
);

-- 初始化默认状态
INSERT OR REPLACE INTO app_state (id, current_state) VALUES (1, 'UNINITIALIZED');

-- ========================================
-- 下载任务管理表（支持断点续传）
-- ========================================

-- 创建下载任务ID序列
CREATE SEQUENCE IF NOT EXISTS download_tasks_seq;

-- 下载任务表（移除实时更新字段，避免写冲突）
CREATE TABLE IF NOT EXISTS download_tasks (
    id INTEGER PRIMARY KEY DEFAULT nextval('download_tasks_seq'),
    task_name VARCHAR NOT NULL, -- 任务名称（如：docker-service-v1.2.0）
    download_url VARCHAR NOT NULL, -- 下载地址
    total_size BIGINT NOT NULL, -- 总文件大小（字节）
    downloaded_size BIGINT DEFAULT 0, -- 已下载大小（仅在关键节点更新）
    target_path VARCHAR NOT NULL, -- 目标保存路径
    file_hash VARCHAR, -- 文件哈希值，用于校验
    status VARCHAR NOT NULL DEFAULT 'PENDING', -- PENDING/DOWNLOADING/PAUSED/COMPLETED/FAILED
    error_message TEXT, -- 错误信息
    retry_count INTEGER DEFAULT 0, -- 重试次数
    max_retry_count INTEGER DEFAULT 3, -- 最大重试次数
    
    -- 下载统计信息（只记录最终统计，不频繁更新）
    average_speed BIGINT DEFAULT 0, -- 平均下载速度（完成后计算）
    total_duration_seconds INTEGER DEFAULT 0, -- 总下载时长（完成后记录）
    
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP
);

-- 分片下载表（支持多线程下载和断点续传）
-- 创建下载分片ID序列
CREATE SEQUENCE IF NOT EXISTS download_chunks_seq;

CREATE TABLE IF NOT EXISTS download_chunks (
    id INTEGER PRIMARY KEY DEFAULT nextval('download_chunks_seq'),
    task_id INTEGER NOT NULL,
    chunk_index INTEGER NOT NULL, -- 分片索引
    start_byte BIGINT NOT NULL, -- 起始字节位置
    end_byte BIGINT NOT NULL, -- 结束字节位置
    downloaded_bytes BIGINT DEFAULT 0, -- 已下载字节数
    status VARCHAR NOT NULL DEFAULT 'PENDING', -- PENDING/DOWNLOADING/COMPLETED/FAILED
    error_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(task_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_download_chunks_task_id ON download_chunks(task_id);
CREATE INDEX IF NOT EXISTS idx_download_tasks_status ON download_tasks(status);

-- ========================================
-- 系统检查表（平台兼容性检查）
-- ========================================

-- 创建系统检查ID序列
CREATE SEQUENCE IF NOT EXISTS system_checks_seq;

CREATE TABLE IF NOT EXISTS system_checks (
    id INTEGER PRIMARY KEY DEFAULT nextval('system_checks_seq'),
    check_type VARCHAR NOT NULL, -- STORAGE_SPACE/DOCKER_STATUS/NETWORK/PERMISSIONS等
    check_name VARCHAR NOT NULL, -- 检查项名称
    platform VARCHAR NOT NULL, -- windows/macos/linux
    required_value VARCHAR, -- 要求的值
    actual_value VARCHAR, -- 实际检测到的值
    status VARCHAR NOT NULL, -- PASS/FAIL/WARNING/SKIPPED
    message TEXT, -- 详细信息或建议
    checked_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_system_checks_type_platform ON system_checks(check_type, platform);

-- ========================================
-- Docker 服务状态管理
-- ========================================

-- 服务状态历史表（时序数据，适合批量插入）
-- 创建服务状态历史ID序列
CREATE SEQUENCE IF NOT EXISTS service_status_history_seq;

CREATE TABLE IF NOT EXISTS service_status_history (
    id INTEGER PRIMARY KEY DEFAULT nextval('service_status_history_seq'),
    service_name VARCHAR NOT NULL,
    container_id VARCHAR,
    status VARCHAR NOT NULL, -- running/stopped/error/starting/stopping
    cpu_usage DOUBLE, -- CPU使用率
    memory_usage BIGINT, -- 内存使用量（字节）
    network_io JSON, -- 网络IO统计
    health_status VARCHAR, -- healthy/unhealthy/unknown
    error_message TEXT,
    recorded_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 时序数据索引优化
CREATE INDEX IF NOT EXISTS idx_service_status_service_time ON service_status_history(service_name, recorded_at);

-- 当前服务状态表（实时状态，减少查询开销）
CREATE TABLE IF NOT EXISTS current_service_status (
    service_name VARCHAR PRIMARY KEY,
    container_id VARCHAR,
    status VARCHAR NOT NULL,
    health_status VARCHAR,
    last_updated TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    uptime_seconds INTEGER DEFAULT 0,
    restart_count INTEGER DEFAULT 0
);

-- ========================================
-- 备份管理表
-- ========================================

-- 创建备份记录ID序列
CREATE SEQUENCE IF NOT EXISTS backup_records_seq;

CREATE TABLE IF NOT EXISTS backup_records (
    id INTEGER PRIMARY KEY DEFAULT nextval('backup_records_seq'),
    backup_name VARCHAR NOT NULL UNIQUE,
    backup_type VARCHAR NOT NULL, -- FULL/INCREMENTAL/DATA_ONLY
    source_version VARCHAR, -- 备份时的服务版本
    backup_path VARCHAR NOT NULL, -- 备份文件路径
    backup_size BIGINT, -- 备份文件大小
    file_count INTEGER, -- 备份文件数量
    compression_type VARCHAR DEFAULT 'gzip', -- 压缩类型
    backup_hash VARCHAR, -- 备份文件哈希值
    description TEXT, -- 备份描述
    
    -- 备份元数据
    backup_metadata JSON, -- 额外的备份元信息
    
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP -- 过期时间（可选）
);

CREATE INDEX IF NOT EXISTS idx_backup_records_created_at ON backup_records(created_at);
CREATE INDEX IF NOT EXISTS idx_backup_records_type ON backup_records(backup_type);

-- ========================================
-- 升级管理表
-- ========================================

-- 创建升级历史ID序列
CREATE SEQUENCE IF NOT EXISTS upgrade_history_seq;

CREATE TABLE IF NOT EXISTS upgrade_history (
    id INTEGER PRIMARY KEY DEFAULT nextval('upgrade_history_seq'),
    upgrade_id VARCHAR NOT NULL UNIQUE, -- UUID
    from_version VARCHAR NOT NULL,
    to_version VARCHAR NOT NULL,
    upgrade_type VARCHAR NOT NULL, -- FULL/INCREMENTAL/HOTFIX
    status VARCHAR NOT NULL, -- PENDING/RUNNING/SUCCESS/FAILED/ROLLED_BACK
    
    -- 时间跟踪
    scheduled_at TIMESTAMP, -- 预约时间（如果是预约升级）
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    
    -- 备份信息
    backup_id INTEGER, -- 关联的备份记录ID
    
    -- 结果信息
    error_message TEXT,
    rollback_reason TEXT,
    upgrade_log TEXT, -- 升级过程日志
    
    -- 文件信息
    download_size BIGINT, -- 下载的升级包大小
    download_time_seconds INTEGER, -- 下载耗时
    installation_time_seconds INTEGER, -- 安装耗时
    
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    
    FOREIGN KEY (backup_id) REFERENCES backup_records(id)
);

CREATE INDEX IF NOT EXISTS idx_upgrade_history_status ON upgrade_history(status);
CREATE INDEX IF NOT EXISTS idx_upgrade_history_versions ON upgrade_history(from_version, to_version);

-- 自动升级任务表
-- 创建自动升级任务ID序列
CREATE SEQUENCE IF NOT EXISTS auto_upgrade_tasks_seq;

CREATE TABLE IF NOT EXISTS auto_upgrade_tasks (
    id INTEGER PRIMARY KEY DEFAULT nextval('auto_upgrade_tasks_seq'),
    task_id VARCHAR NOT NULL UNIQUE,
    task_name VARCHAR NOT NULL,
    schedule_time TIMESTAMP NOT NULL,
    upgrade_type VARCHAR NOT NULL, -- FULL/INCREMENTAL/HOTFIX
    target_version VARCHAR,
    status VARCHAR NOT NULL DEFAULT 'pending', -- pending/in_progress/completed/failed/cancelled
    progress INTEGER DEFAULT 0, -- 进度百分比 0-100
    error_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_auto_upgrade_tasks_status ON auto_upgrade_tasks(status);
CREATE INDEX IF NOT EXISTS idx_auto_upgrade_tasks_schedule ON auto_upgrade_tasks(schedule_time);

-- ========================================
-- 初始化默认配置数据
-- ========================================

-- 系统配置 (使用 INSERT OR IGNORE 避免覆盖已有配置)
INSERT OR IGNORE INTO app_config (config_key, config_value, config_type, category, description, is_system_config, is_user_editable, default_value) VALUES
-- 应用基础配置
('app.version', '"0.1.0"', 'STRING', 'system', 'Duck Client应用版本', TRUE, FALSE, '"0.1.0"'),
('app.working_directory', '""', 'STRING', 'system', '当前工作目录', TRUE, TRUE, '""'),
('app.first_run', 'true', 'BOOLEAN', 'system', '是否首次运行', TRUE, FALSE, 'true'),

-- UI 界面配置
('ui.theme', '"auto"', 'STRING', 'ui', '主题设置：light/dark/auto', FALSE, TRUE, '"auto"'),
('ui.language', '"zh-CN"', 'STRING', 'ui', '界面语言', FALSE, TRUE, '"zh-CN"'),
('ui.window_width', '1200', 'NUMBER', 'ui', '窗口宽度', FALSE, TRUE, '1200'),
('ui.window_height', '800', 'NUMBER', 'ui', '窗口高度', FALSE, TRUE, '800'),
('ui.window_maximized', 'false', 'BOOLEAN', 'ui', '是否最大化窗口', FALSE, TRUE, 'false'),
('ui.minimize_to_tray', 'true', 'BOOLEAN', 'ui', '最小化到系统托盘', FALSE, TRUE, 'true'),
('ui.confirm_dangerous_actions', 'true', 'BOOLEAN', 'ui', '危险操作二次确认', FALSE, TRUE, 'true'),
('ui.show_detailed_progress', 'true', 'BOOLEAN', 'ui', '显示详细进度信息', FALSE, TRUE, 'true'),

-- Docker 服务配置
('docker.compose_file_path', '"./docker/docker-compose.yml"', 'STRING', 'docker', 'Docker Compose文件路径', TRUE, TRUE, '"./docker/docker-compose.yml"'),
('docker.auto_start_on_boot', 'false', 'BOOLEAN', 'docker', '开机自动启动服务', FALSE, TRUE, 'false'),
('docker.health_check_interval', '30', 'NUMBER', 'docker', '健康检查间隔（秒）', FALSE, TRUE, '30'),

-- 下载配置
('download.chunk_size', '1048576', 'NUMBER', 'download', '下载分片大小（字节）', FALSE, TRUE, '1048576'),
('download.max_concurrent_chunks', '4', 'NUMBER', 'download', '最大并发下载分片数', FALSE, TRUE, '4'),
('download.retry_count', '3', 'NUMBER', 'download', '下载重试次数', FALSE, TRUE, '3'),
('download.timeout_seconds', '300', 'NUMBER', 'download', '下载超时时间（秒）', FALSE, TRUE, '300'),
('download.auto_resume', 'true', 'BOOLEAN', 'download', '自动断点续传', FALSE, TRUE, 'true'),

-- 备份配置
('backup.retention_days', '30', 'NUMBER', 'backup', '备份保留天数', FALSE, TRUE, '30'),
('backup.auto_cleanup', 'true', 'BOOLEAN', 'backup', '自动清理过期备份', FALSE, TRUE, 'true'),
('backup.compression_enabled', 'true', 'BOOLEAN', 'backup', '备份文件压缩', FALSE, TRUE, 'true'),
('backup.verify_after_backup', 'true', 'BOOLEAN', 'backup', '备份后文件校验', FALSE, TRUE, 'true'),

-- 自动备份配置
('auto_backup_enabled', 'false', 'BOOLEAN', 'backup', '自动备份开关', FALSE, TRUE, 'false'),
('auto_backup_schedule', '"0 2 * * *"', 'STRING', 'backup', '自动备份计划(cron表达式)', FALSE, TRUE, '"0 2 * * *"'),
('auto_backup_retention_days', '7', 'NUMBER', 'backup', '自动备份保留天数', FALSE, TRUE, '7'),
('auto_backup_directory', '"./backups"', 'STRING', 'backup', '自动备份目录', FALSE, TRUE, '"./backups"'),
('auto_backup_last_time', '""', 'STRING', 'backup', '上次自动备份时间', FALSE, FALSE, '""'),
('auto_backup_last_status', '""', 'STRING', 'backup', '上次自动备份状态', FALSE, FALSE, '""'),

-- 升级配置
('upgrade.auto_backup_enabled', 'true', 'BOOLEAN', 'upgrade', '升级前自动备份', FALSE, TRUE, 'true'),
('upgrade.rollback_enabled', 'true', 'BOOLEAN', 'upgrade', '升级失败自动回滚', FALSE, TRUE, 'true'),
('upgrade.auto_check_update', 'true', 'BOOLEAN', 'upgrade', '自动检查更新', FALSE, TRUE, 'true'),
('upgrade.check_interval_hours', '24', 'NUMBER', 'upgrade', '更新检查间隔（小时）', FALSE, TRUE, '24'),

-- 网络配置
('network.proxy_enabled', 'false', 'BOOLEAN', 'network', '是否启用代理', FALSE, TRUE, 'false'),
('network.proxy_config', '{}', 'OBJECT', 'network', '代理配置', FALSE, TRUE, '{}'),
('network.connection_timeout', '30', 'NUMBER', 'network', '网络连接超时（秒）', FALSE, TRUE, '30'),

-- 日志配置
('logging.level', '"INFO"', 'STRING', 'logging', '日志级别：DEBUG/INFO/WARN/ERROR', FALSE, TRUE, '"INFO"'),
('logging.max_file_size', '10485760', 'NUMBER', 'logging', '单个日志文件最大大小（字节）', FALSE, TRUE, '10485760'),
('logging.max_files', '5', 'NUMBER', 'logging', '最大日志文件数量', FALSE, TRUE, '5'),

-- 安全配置
('security.allow_insecure_downloads', 'false', 'BOOLEAN', 'security', '允许不安全的下载连接', FALSE, TRUE, 'false'),
('security.verify_ssl_certificates', 'true', 'BOOLEAN', 'security', '验证SSL证书', FALSE, TRUE, 'true');

-- ========================================
-- 用户操作历史表（审计日志）
-- ========================================

-- 创建用户操作ID序列
CREATE SEQUENCE IF NOT EXISTS user_actions_seq;

CREATE TABLE IF NOT EXISTS user_actions (
    id INTEGER PRIMARY KEY DEFAULT nextval('user_actions_seq'),
    action_type VARCHAR NOT NULL, -- UPGRADE/BACKUP/RESTORE/CONFIG_CHANGE等
    action_description TEXT NOT NULL,
    action_params JSON, -- 操作参数
    
    -- 结果信息
    status VARCHAR NOT NULL, -- SUCCESS/FAILED/CANCELLED
    result_message TEXT,
    
    -- 时间信息
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    duration_seconds INTEGER,
    
    -- 上下文信息
    session_id VARCHAR, -- 会话ID（如果需要）
    client_version VARCHAR, -- 客户端版本
    platform_info VARCHAR -- 平台信息
);

-- 审计日志索引
CREATE INDEX IF NOT EXISTS idx_user_actions_type_time ON user_actions(action_type, started_at);
CREATE INDEX IF NOT EXISTS idx_user_actions_status ON user_actions(status);

-- ========================================
-- 性能监控表（可选，用于性能调优）
-- ========================================

-- 创建性能监控ID序列
CREATE SEQUENCE IF NOT EXISTS performance_metrics_seq;

CREATE TABLE IF NOT EXISTS performance_metrics (
    id INTEGER PRIMARY KEY DEFAULT nextval('performance_metrics_seq'),
    metric_name VARCHAR NOT NULL,
    metric_value DOUBLE NOT NULL,
    metric_unit VARCHAR, -- ms/MB/count等
    tags JSON, -- 额外的标签信息
    recorded_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_performance_metrics_name_time ON performance_metrics(metric_name, recorded_at);

-- ========================================
-- 数据库维护和优化
-- ========================================

-- 数据库维护配置 (使用 INSERT OR IGNORE 避免覆盖已有配置)
INSERT OR IGNORE INTO app_config (config_key, config_value, config_type, category, description, is_system_config, is_user_editable, default_value) VALUES
('maintenance.cleanup_service_history_days', '7', 'NUMBER', 'maintenance', '服务状态历史保留天数', TRUE, TRUE, '7'),
('maintenance.cleanup_user_actions_days', '90', 'NUMBER', 'maintenance', '用户操作历史保留天数', TRUE, TRUE, '90'),
('maintenance.cleanup_performance_metrics_days', '30', 'NUMBER', 'maintenance', '性能监控数据保留天数', TRUE, TRUE, '30'),
('maintenance.vacuum_schedule', '"0 2 * * 0"', 'STRING', 'maintenance', '数据库VACUUM计划（每周日凌晨2点）', TRUE, TRUE, '"0 2 * * 0"'),
('maintenance.auto_optimize_enabled', 'true', 'BOOLEAN', 'maintenance', '自动数据库优化', TRUE, TRUE, 'true');

-- ========================================
-- 数据库版本管理
-- ========================================

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    description TEXT,
    applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 记录当前数据库版本
INSERT OR REPLACE INTO schema_version (version, description) VALUES 
(1, '初始数据库结构 - 支持DuckDB并发优化');

-- ========================================
-- 创建视图用于常用查询优化
-- ========================================

-- 当前下载进度视图（移除实时字段，这些数据从内存获取）
CREATE VIEW IF NOT EXISTS current_download_progress AS
SELECT 
    dt.id,
    dt.task_name,
    dt.total_size,
    dt.downloaded_size,
    ROUND(dt.downloaded_size * 100.0 / dt.total_size, 2) as progress_percentage,
    dt.status,
    dt.created_at,
    dt.updated_at,
    COUNT(dc.id) as total_chunks,
    COUNT(CASE WHEN dc.status = 'COMPLETED' THEN 1 END) as completed_chunks
FROM download_tasks dt
LEFT JOIN download_chunks dc ON dt.id = dc.task_id
WHERE dt.status IN ('DOWNLOADING', 'PAUSED')
GROUP BY dt.id, dt.task_name, dt.total_size, dt.downloaded_size, dt.status, dt.created_at, dt.updated_at;

-- 最新服务状态视图
CREATE VIEW IF NOT EXISTS latest_service_status AS
SELECT DISTINCT
    service_name,
    FIRST_VALUE(status) OVER (PARTITION BY service_name ORDER BY recorded_at DESC) as current_status,
    FIRST_VALUE(health_status) OVER (PARTITION BY service_name ORDER BY recorded_at DESC) as current_health,
    FIRST_VALUE(recorded_at) OVER (PARTITION BY service_name ORDER BY recorded_at DESC) as last_update
FROM service_status_history;

-- 配置管理视图（按分类组织，便于管理）
CREATE VIEW IF NOT EXISTS config_by_category AS
SELECT 
    category,
    config_key,
    config_value,
    config_type,
    description,
    is_system_config,
    is_user_editable,
    default_value,
    CASE 
        WHEN config_value = default_value THEN 'DEFAULT'
        ELSE 'MODIFIED'
    END as config_status,
    updated_at
FROM app_config
ORDER BY category, config_key;

-- 用户可编辑配置视图
CREATE VIEW IF NOT EXISTS user_editable_config AS
SELECT 
    config_key,
    config_value,
    config_type,
    category,
    description,
    validation_rule,
    default_value
FROM app_config
WHERE is_user_editable = TRUE
ORDER BY category, config_key; 