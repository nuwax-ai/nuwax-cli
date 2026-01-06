/// 表列定义
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub auto_increment: bool,
    pub comment: Option<String>,
}

impl PartialEq for TableColumn {
    fn eq(&self, other: &Self) -> bool {
        // 名称必须完全匹配
        if self.name != other.name {
            return false;
        }

        // 数据类型比较（忽略大小写）
        if self.data_type.to_uppercase() != other.data_type.to_uppercase() {
            return false;
        }

        // nullable 必须匹配
        if self.nullable != other.nullable {
            return false;
        }

        // auto_increment 必须匹配
        if self.auto_increment != other.auto_increment {
            return false;
        }

        // 默认值比较（标准化后比较，考虑 nullable 情况）
        if !Self::default_values_equal(&self.default_value, &other.default_value, self.nullable) {
            return false;
        }

        // comment 可以忽略（通常不影响功能）
        // 如果需要严格比较 comment，取消下面的注释
        // if self.comment != other.comment {
        //     return false;
        // }

        true
    }
}

impl TableColumn {
    /// 比较两个默认值是否相等（标准化后）
    fn default_values_equal(val1: &Option<String>, val2: &Option<String>, nullable: bool) -> bool {
        match (val1, val2) {
            (None, None) => true,
            (Some(v1), Some(v2)) => {
                let norm1 = Self::normalize_default_value(v1);
                let norm2 = Self::normalize_default_value(v2);
                norm1 == norm2
            }
            // 特殊情况：对于 nullable 列，DEFAULT NULL 等同于没有 DEFAULT
            (Some(v), None) | (None, Some(v)) if nullable => {
                Self::normalize_default_value(v) == "NULL"
            }
            _ => false,
        }
    }

    /// 标准化默认值
    fn normalize_default_value(value: &str) -> String {
        let trimmed = value.trim();

        // 移除数字周围的引号
        if trimmed.starts_with('\'') && trimmed.ends_with('\'') {
            let inner = &trimmed[1..trimmed.len() - 1];
            // 如果内容是纯数字，移除引号
            if inner
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
            {
                return inner.to_string();
            }
        }

        // 统一大小写（对于关键字）
        trimmed.to_uppercase()
    }
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

/// SQL差异结果
#[derive(Debug, Clone)]
pub struct SchemaDiffResult {
    /// 差异SQL内容
    pub diff_sql: String,
    /// 差异描述
    pub description: String,
    /// 在线架构原始SQL（仅Live Diff时有值）
    pub live_sql: Option<String>,
    /// 是否有可执行的SQL语句
    pub has_executable_sql: bool,
    /// 是否包含警告（删除操作）
    pub has_warnings: bool,
}

/// MySQL差异统计信息
#[derive(Debug, Clone, Default)]
pub struct DiffStats {
    /// 新增的表数量
    pub tables_added: usize,
    /// 删除的表数量（警告）
    pub tables_dropped: usize,
    /// 修改的表数量
    pub tables_modified: usize,
    /// 新增的列数量
    pub columns_added: usize,
    /// 删除的列数量（警告）
    pub columns_dropped: usize,
    /// 修改的列数量
    pub columns_modified: usize,
    /// 新增的索引数量
    pub indexes_added: usize,
    /// 删除的索引数量（警告）
    pub indexes_dropped: usize,
    /// 修改的索引数量
    pub indexes_modified: usize,
}

impl DiffStats {
    /// 是否有任何变更
    pub fn has_changes(&self) -> bool {
        self.tables_added > 0
            || self.tables_dropped > 0
            || self.tables_modified > 0
            || self.columns_added > 0
            || self.columns_dropped > 0
            || self.columns_modified > 0
            || self.indexes_added > 0
            || self.indexes_dropped > 0
            || self.indexes_modified > 0
    }

    /// 是否有删除操作（危险操作）
    pub fn has_dangerous_operations(&self) -> bool {
        self.tables_dropped > 0 || self.columns_dropped > 0 || self.indexes_dropped > 0
    }

    /// 是否有可执行的操作（非删除操作）
    pub fn has_executable_operations(&self) -> bool {
        self.tables_added > 0
            || self.columns_added > 0
            || self.columns_modified > 0
            || self.indexes_added > 0
            || self.indexes_modified > 0
    }

    /// 生成变更摘要
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if self.tables_added > 0 {
            parts.push(format!("新增表({})", self.tables_added));
        }
        if self.tables_dropped > 0 {
            parts.push(format!("删除表({}·警告)", self.tables_dropped));
        }
        if self.columns_added > 0 {
            parts.push(format!("新增列({})", self.columns_added));
        }
        if self.columns_dropped > 0 {
            parts.push(format!("删除列({}·警告)", self.columns_dropped));
        }
        if self.columns_modified > 0 {
            parts.push(format!("修改列({})", self.columns_modified));
        }
        if self.indexes_added > 0 {
            parts.push(format!("新增索引({})", self.indexes_added));
        }
        if self.indexes_dropped > 0 {
            parts.push(format!("删除索引({}·警告)", self.indexes_dropped));
        }
        if self.indexes_modified > 0 {
            parts.push(format!("修改索引({})", self.indexes_modified));
        }

        if parts.is_empty() {
            "无变更".to_string()
        } else {
            parts.join("、")
        }
    }
}
