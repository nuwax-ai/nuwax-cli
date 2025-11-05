/// 测试列定义比较的标准化
use client_core::sql_diff::parse_sql_tables;

#[test]
fn test_default_value_with_quotes() {
    // MySQL 返回的 DEFAULT 值带引号
    let mysql_sql = "CREATE TABLE test (
        id BIGINT NOT NULL DEFAULT '1',
        status TINYINT NOT NULL DEFAULT '0'
    );";
    
    // 文件中的 DEFAULT 值不带引号
    let file_sql = "CREATE TABLE test (
        id BIGINT NOT NULL DEFAULT 1,
        status TINYINT NOT NULL DEFAULT 0
    );";
    
    let mysql_tables = parse_sql_tables(mysql_sql).expect("解析 MySQL SQL 失败");
    let file_tables = parse_sql_tables(file_sql).expect("解析文件 SQL 失败");
    
    let mysql_table = mysql_tables.get("test").unwrap();
    let file_table = file_tables.get("test").unwrap();
    
    // 列应该被认为是相同的
    assert_eq!(mysql_table.columns.len(), file_table.columns.len());
    
    for (mysql_col, file_col) in mysql_table.columns.iter().zip(file_table.columns.iter()) {
        assert_eq!(mysql_col, file_col, 
            "列 {} 应该相等，但 MySQL 的默认值是 {:?}，文件的默认值是 {:?}",
            mysql_col.name, mysql_col.default_value, file_col.default_value);
    }
}

#[test]
fn test_data_type_case_insensitive() {
    // MySQL 返回大写
    let mysql_sql = "CREATE TABLE test (
        id BIGINT NOT NULL,
        name VARCHAR(64)
    );";
    
    // 文件中小写
    let file_sql = "CREATE TABLE test (
        id bigint not null,
        name varchar(64)
    );";
    
    let mysql_tables = parse_sql_tables(mysql_sql).expect("解析 MySQL SQL 失败");
    let file_tables = parse_sql_tables(file_sql).expect("解析文件 SQL 失败");
    
    let mysql_table = mysql_tables.get("test").unwrap();
    let file_table = file_tables.get("test").unwrap();
    
    // 列应该被认为是相同的
    for (mysql_col, file_col) in mysql_table.columns.iter().zip(file_table.columns.iter()) {
        assert_eq!(mysql_col, file_col, 
            "列 {} 的数据类型应该忽略大小写", mysql_col.name);
    }
}

#[test]
fn test_full_diff_with_normalized_columns() {
    use client_core::sql_diff::generate_schema_diff;
    
    // 模拟 MySQL 返回的 SQL（带引号的 DEFAULT，大写类型）
    let mysql_sql = "CREATE TABLE `agent_component_config` (
  `id` BIGINT NOT NULL AUTO_INCREMENT,
  `_tenant_id` BIGINT NOT NULL DEFAULT '1' COMMENT '商户ID',
  `name` VARCHAR(64) DEFAULT NULL COMMENT '节点名称',
  `exception_out` TINYINT NOT NULL DEFAULT '0' COMMENT '异常是否抛出',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB;";
    
    // 文件中的 SQL（不带引号的 DEFAULT，小写类型）
    let file_sql = "create table agent_component_config (
    id            bigint auto_increment primary key,
    _tenant_id    bigint   default 1                 not null comment '商户ID',
    name          varchar(64)                        null comment '节点名称',
    exception_out tinyint  default 0                 not null comment '异常是否抛出'
) ENGINE=InnoDB;";
    
    let (diff_sql, description) = generate_schema_diff(
        Some(mysql_sql),
        file_sql,
        Some("MySQL"),
        "文件"
    ).expect("生成差异失败");
    
    println!("\n差异描述: {}", description);
    println!("\n差异 SQL:\n{}", diff_sql);
    
    // 应该没有实际差异（忽略 COMMENT 和格式差异）
    let meaningful_lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--")
        })
        .collect();
    
    if !meaningful_lines.is_empty() {
        println!("\n⚠️  检测到差异:");
        for line in &meaningful_lines {
            println!("  {}", line);
        }
    }
    
    assert!(meaningful_lines.is_empty(), 
        "相同结构的表不应产生差异（忽略 DEFAULT 引号和大小写），但生成了 {} 行差异",
        meaningful_lines.len());
}
