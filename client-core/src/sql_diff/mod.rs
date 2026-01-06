mod differ;
mod generator;
mod parser;
mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod backtick_normalization_tests;

// 重新导出公共接口
pub use generator::{generate_live_schema_diff, generate_schema_diff};
pub use parser::parse_sql_tables;
pub use types::{DiffStats, SchemaDiffResult, TableColumn, TableDefinition, TableIndex};
