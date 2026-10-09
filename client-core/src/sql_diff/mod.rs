mod differ;
mod generator;
mod parser;
mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod backtick_normalization_tests;

#[cfg(test)]
mod schema_template_tests;

// 重新导出公共接口
pub use generator::{generate_live_schema_diff_multi, generate_schema_diff};
pub use parser::{parse_schema_template, parse_sql_tables, parse_sql_tables_strict};
pub use types::{
    DbSectionResult, DiffStats, MultiDbDiffResult, SchemaDiffResult, SchemaTemplate, TableColumn,
    TableDefinition, TableIndex,
};
