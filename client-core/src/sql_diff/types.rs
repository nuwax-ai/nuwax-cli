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
            },
            // 特殊情况：对于 nullable 列，DEFAULT NULL 等同于没有 DEFAULT
            (Some(v), None) | (None, Some(v)) if nullable => {
                Self::normalize_default_value(v) == "NULL"
            },
            _ => false,
        }
    }

    /// 标准化默认值
    fn normalize_default_value(value: &str) -> String {
        let trimmed = value.trim();
        
        // 移除数字周围的引号
        if trimmed.starts_with('\'') && trimmed.ends_with('\'') {
            let inner = &trimmed[1..trimmed.len()-1];
            // 如果内容是纯数字，移除引号
            if inner.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.') {
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
