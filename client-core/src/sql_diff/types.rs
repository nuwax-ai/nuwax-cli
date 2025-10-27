/// 表列定义
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub auto_increment: bool,
    pub comment: Option<String>,
}

/// 表索引定义
#[derive(Debug, Clone, PartialEq)]
pub struct TableIndex {
    pub name: String,
    pub columns: Vec<String>,
    pub is_primary: bool,
    pub is_unique: bool,
    pub index_type: Option<String>,
}

/// 表定义
#[derive(Debug, Clone)]
pub struct TableDefinition {
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub indexes: Vec<TableIndex>,
    pub engine: Option<String>,
    pub charset: Option<String>,
}
