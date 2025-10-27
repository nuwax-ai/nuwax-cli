// DuckDB数据库模块
//
// 这个模块提供了一个线程安全的DuckDB数据库接口，通过Actor模式确保
// DuckDB的单线程访问要求，同时为客户端提供异步、类型安全的API。
//
// 主要组件：
// - DuckDbManager: 高级API接口，供应用程序使用
// - DuckDbActor: 内部Actor，处理实际的数据库操作
// - 数据模型和消息定义

mod actor;
mod manager;
mod messages;
mod models;

// 公开核心接口
pub use manager::DuckDbManager;
pub use models::{BackupRecord, ScheduledTask};

// 重新导出常用类型
pub type DbManager = DuckDbManager;
