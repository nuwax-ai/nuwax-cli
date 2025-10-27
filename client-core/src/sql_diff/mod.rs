mod differ;
mod generator;
mod parser;
mod types;

#[cfg(test)]
mod tests;

// 重新导出公共接口
pub use generator::generate_schema_diff;
