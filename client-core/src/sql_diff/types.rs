/// 生成列定义（MySQL `GENERATED ALWAYS AS (expr) STORED/VIRTUAL`）
///
/// 注意：对存量表 ADD/MODIFY 为 STORED 生成列会触发全表重建，大表升级需评估耗时。
#[derive(Debug, Clone)]
pub struct GeneratedColumnDef {
    /// 生成表达式（解析后已剥掉外层括号的内层表达式）
    pub expr: String,
    /// true = STORED；false = VIRTUAL（MySQL 缺省 VIRTUAL）
    pub stored: bool,
}

impl PartialEq for GeneratedColumnDef {
    fn eq(&self, other: &Self) -> bool {
        self.stored == other.stored
            && normalize_generated_expr(&self.expr) == normalize_generated_expr(&other.expr)
    }
}

/// 表列定义
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    /// MySQL `ON UPDATE CURRENT_TIMESTAMP` 等自动更新表达式
    pub on_update: Option<String>,
    /// 生成列定义；普通列为 None
    pub generated: Option<GeneratedColumnDef>,
    pub auto_increment: bool,
    pub comment: Option<String>,
}

impl PartialEq for TableColumn {
    fn eq(&self, other: &Self) -> bool {
        // 名称必须完全匹配
        if self.name != other.name {
            return false;
        }

        // 数据类型比较（语义归一化：忽略已弃用的显示宽度，但 TINYINT(1) 例外）
        if normalize_type_for_compare(&self.data_type)
            != normalize_type_for_compare(&other.data_type)
        {
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

        // ON UPDATE 语义比较（NOW()/LOCALTIME() 等同义词归一为 CURRENT_TIMESTAMP）
        if normalize_on_update(&self.on_update) != normalize_on_update(&other.on_update) {
            return false;
        }

        // 生成列必须匹配（表达式经归一化比较）
        if self.generated != other.generated {
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

        // 移除数字周围的引号（数字无大小写语义）
        if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
            let inner = &trimmed[1..trimmed.len() - 1];
            // 如果内容是纯数字，移除引号
            if !inner.is_empty()
                && inner
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
            {
                return inner.to_string();
            }
            // 引号内是字符串字面量：保留原始大小写，否则 'Pending' vs 'pending'
            // 这类默认值大小写变化会被漏检（静默漂移）
            return trimmed.to_string();
        }

        // 无引号：可能是关键字（CURRENT_TIMESTAMP/NULL/...），统一大小写
        trimmed.to_uppercase()
    }
}

/// 类型语义归一化（仅用于差异比较，生成侧保留原始写法）：
/// - 整数族剥显示宽度：MySQL 8.0.17+ 已弃用且 SHOW CREATE 不再输出，
///   模板 `int(11)` 与线上 `int` 不应产生虚假 MODIFY；
/// - `TINYINT(1)` 例外：布尔语义，与 `TINYINT` 视为不同；
/// - 时间类型剥 `(0)` 精度（`DATETIME` ≡ `DATETIME(0)`）。
fn normalize_type_for_compare(data_type: &str) -> String {
    let upper = data_type.to_uppercase();

    const INT_TYPES: [&str; 6] = [
        "TINYINT",
        "SMALLINT",
        "MEDIUMINT",
        "INT",
        "INTEGER",
        "BIGINT",
    ];
    for int_type in INT_TYPES {
        let prefix = format!("{int_type}(");
        let Some(rest) = upper.strip_prefix(&prefix) else {
            continue;
        };
        let Some(close) = rest.find(')') else {
            continue;
        };
        let width = &rest[..close];
        let tail = rest[close + 1..].trim(); // 允许 " UNSIGNED" 等后缀
        if !width.is_empty() && width.chars().all(|c| c.is_ascii_digit()) {
            // TINYINT(1) 保留宽度参与比较（布尔语义），其余整数宽度剥离。
            // 后缀与类型名之间必须以单个空格连接，否则 `INT(11) UNSIGNED`
            // 会归一化成 `INTUNSIGNED`，与 `INT UNSIGNED` 不等（虚假 MODIFY）。
            if int_type == "TINYINT" && width == "1" {
                return upper;
            }
            if tail.is_empty() {
                return int_type.to_string();
            }
            return format!("{int_type} {tail}");
        }
    }

    const TIME_TYPES: [&str; 4] = ["DATETIME", "TIMESTAMP", "TIME", "DATE"];
    let without_precision = |name: &str| upper == format!("{name}(0)");
    if TIME_TYPES.iter().any(|t| without_precision(t)) {
        return upper.replace("(0)", "");
    }

    upper
}

/// ON UPDATE 表达式语义归一化（仅用于比较）：
/// SHOW CREATE TABLE 恒输出 CURRENT_TIMESTAMP 形态；模板写 NOW()/LOCALTIME()/
/// LOCALTIMESTAMP()（无精度参数）语义相同，不归一会导致线上部署每次都重复 MODIFY。
fn normalize_on_update(on_update: &Option<String>) -> String {
    let Some(value) = on_update else {
        return String::new();
    };
    let upper = value.trim().to_uppercase();
    match upper.as_str() {
        "NOW()" | "NOW" | "LOCALTIME()" | "LOCALTIME" | "LOCALTIMESTAMP()" | "LOCALTIMESTAMP" => {
            "CURRENT_TIMESTAMP".to_string()
        }
        _ => upper,
    }
}

/// 生成列表达式归一化（仅用于比较）：
/// SHOW CREATE TABLE 打印生成列时标识符带反引号、空格与关键字大小写由服务端控制、
/// 整个表达式会多包一层括号（`((a + b))`）、字符串字面量前会加字符集引导符
/// （`_utf8mb4'...'`）。与手写模板比较前依次归一：剥反引号、连续空白折叠为单空格、
/// 单引号字符串字面量之外大小写折叠、剥字符集引导符、剥整体包裹的平衡括号。
/// 残余差异方向是多报 MODIFY（安全侧），不会漏检。
fn normalize_generated_expr(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut in_string = false;
    let mut last_was_space = false;

    for ch in expr.chars() {
        if in_string {
            out.push(ch);
            if ch == '\'' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '\'' => {
                // 剥 SHOW CREATE 加在字符串字面量前的字符集引导符。
                // 注意 Display 会把 `_utf8mb4'x'` 渲染为 `_utf8mb4 'x'`（引导符与
                // 引号之间带空格），先 trim 尾部空白再匹配；截断后保留引导符前的
                // 参数分隔空格，与模板形态对齐。
                let trimmed_end = out.trim_end();
                for introducer in ["_UTF8MB4", "_UTF8MB3", "_UTF8", "_LATIN1", "_BINARY"] {
                    if trimmed_end.ends_with(introducer) {
                        out.truncate(trimmed_end.len() - introducer.len());
                        break;
                    }
                }
                in_string = true;
                last_was_space = false;
                out.push(ch);
            }
            // 剥反引号：`col` 与 col 视为同一标识符
            '`' => {}
            c if c.is_whitespace() => {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            c => {
                last_was_space = false;
                out.push(c.to_ascii_uppercase());
            }
        }
    }

    let mut result = out.trim().to_string();
    while let Some(inner) = strip_balanced_outer_parens(&result) {
        result = inner;
    }
    result
}

/// 整个表达式被一对平衡的括号包裹时剥掉这层（`((a))` → `a`）；
/// 若首个闭合括号不在末尾（如 `(a) + (b)`），说明不是整体包裹，原样返回。
fn strip_balanced_outer_parens(s: &str) -> Option<String> {
    if !(s.starts_with('(') && s.ends_with(')') && s.len() >= 2) {
        return None;
    }
    let mut depth: i32 = 0;
    let mut in_string = false;
    for (i, ch) in s.chars().enumerate() {
        if in_string {
            if ch == '\'' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '\'' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i != s.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth == 0 {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

/// 表索引定义
#[derive(Debug, Clone, PartialEq)]
pub struct TableIndex {
    pub name: String,
    pub columns: Vec<String>,
    pub is_primary: bool,
    pub is_unique: bool,
    /// MySQL: `CREATE FULLTEXT INDEX` / `CREATE SPATIAL INDEX` 标记
    pub is_fulltext: bool,
    /// MySQL: `CREATE FULLTEXT INDEX` / `CREATE SPATIAL INDEX` 标记
    pub is_spatial: bool,
    pub index_type: Option<String>,
    /// MySQL FULLTEXT 索引的 `WITH PARSER <name>` 子句，例如 `ngram`。
    /// standalone `CREATE FULLTEXT INDEX` 与内联 `CREATE TABLE ... FULLTEXT KEY`
    /// 两种形式都会提取该值（fork 版 sqlparser 0.63.2+ 支持内联形式）。
    pub parser: Option<String>,
}

/// 表定义
#[derive(Debug, Clone)]
pub struct TableDefinition {
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub indexes: Vec<TableIndex>,
    pub engine: Option<String>,
    pub charset: Option<String>,
    /// 表级 COLLATE（如 utf8mb4_0900_ai_ci）；缺省 None 表示未显式声明
    pub collation: Option<String>,
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
    /// 是否包含需要人工处理的警告
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
    /// 需要人工处理的表选项变更数量
    pub table_options_changed: usize,
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
            || self.table_options_changed > 0
    }

    /// 是否有需要人工处理的删除或同名索引修改
    pub fn has_dangerous_operations(&self) -> bool {
        self.tables_dropped > 0
            || self.columns_dropped > 0
            || self.indexes_dropped > 0
            || self.indexes_modified > 0
    }

    /// 是否有人工处理提示，包括不会自动执行的表选项变更。
    pub fn has_warnings(&self) -> bool {
        self.has_dangerous_operations() || self.table_options_changed > 0
    }

    /// 是否有可执行的操作（非删除操作）
    pub fn has_executable_operations(&self) -> bool {
        self.tables_added > 0
            || self.columns_added > 0
            || self.columns_modified > 0
            || self.indexes_added > 0
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
        if self.table_options_changed > 0 {
            parts.push(format!("表选项变更({}·警告)", self.table_options_changed));
        }

        if parts.is_empty() {
            "无变更".to_string()
        } else {
            parts.join("、")
        }
    }
}
