/// 测试索引名不匹配的场景
use client_core::sql_diff::{parse_sql_tables, generate_schema_diff};

#[test]
fn test_index_name_difference() {
    // 模拟 MySQL SHOW CREATE TABLE 返回的 SQL（可能使用不同的索引名）
    let mysql_sql = "CREATE TABLE `space_user` (
  `id` bigint AUTO_INCREMENT PRIMARY KEY,
  `space_id` bigint NOT NULL,
  `user_id` bigint NOT NULL,
  UNIQUE KEY `unique_space_id_user_id` (`space_id`, `user_id`)
) ENGINE=InnoDB;";
    
    // 文件中的 SQL（使用 constraint 语法）
    let file_sql = "CREATE TABLE space_user (
    id bigint auto_increment primary key,
    space_id bigint not null,
    user_id bigint not null,
    constraint uk_space_user unique (space_id, user_id)
) ENGINE=InnoDB;";
    
    let mysql_tables = parse_sql_tables(mysql_sql).expect("解析 MySQL SQL 失败");
    let file_tables = parse_sql_tables(file_sql).expect("解析文件 SQL 失败");
    
    println!("\nMySQL 表索引:");
    for idx in &mysql_tables.get("space_user").unwrap().indexes {
        println!("  - name: '{}', is_unique: {}, columns: {:?}", 
            idx.name, idx.is_unique, idx.columns);
    }
    
    println!("\n文件表索引:");
    for idx in &file_tables.get("space_user").unwrap().indexes {
        println!("  - name: '{}', is_unique: {}, columns: {:?}", 
            idx.name, idx.is_unique, idx.columns);
    }
    
    // 生成差异
    let (diff_sql, description) = generate_schema_diff(
        Some(mysql_sql),
        file_sql,
        Some("MySQL"),
        "文件"
    ).expect("生成差异失败");
    
    println!("\n差异描述: {}", description);
    println!("\n差异 SQL:\n{}", diff_sql);
    
    // 检查是否有索引相关的操作
    let has_add_index = diff_sql.contains("ADD") && diff_sql.contains("KEY");
    let has_drop_index = diff_sql.contains("DROP KEY");
    
    if has_add_index || has_drop_index {
        println!("\n⚠️  检测到索引变更操作");
        
        // 检查是否是同一个索引（列相同但名字不同）
        if has_add_index && has_drop_index {
            println!("⚠️  可能是索引重命名（删除旧索引，添加新索引）");
            println!("这可能导致 'Duplicate key name' 错误！");
        }
    }
}
