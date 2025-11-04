/// 测试 CONSTRAINT 语法的解析
use client_core::sql_diff::parse_sql_tables;

#[test]
fn test_constraint_unique_syntax() {
    let sql = "CREATE TABLE space_user (
        id bigint auto_increment primary key,
        space_id bigint not null,
        user_id bigint not null,
        constraint uk_space_user unique (space_id, user_id)
    );";
    
    let tables = parse_sql_tables(sql).expect("解析失败");
    let table = tables.get("space_user").expect("应该有 space_user 表");
    
    println!("解析出的索引:");
    for idx in &table.indexes {
        println!("  - name: '{}', is_unique: {}, is_primary: {}, columns: {:?}", 
            idx.name, idx.is_unique, idx.is_primary, idx.columns);
    }
    
    // 查找唯一索引
    let unique_idx = table.indexes.iter()
        .find(|i| i.is_unique && !i.is_primary)
        .expect("应该有唯一索引");
    
    // 索引名应该是 uk_space_user
    assert_eq!(unique_idx.name, "uk_space_user", "索引名应该是 uk_space_user");
    assert_eq!(unique_idx.columns.len(), 2);
    assert_eq!(unique_idx.columns[0], "space_id");
    assert_eq!(unique_idx.columns[1], "user_id");
}

#[test]
fn test_unique_key_syntax() {
    let sql = "CREATE TABLE space_user (
        id bigint auto_increment primary key,
        space_id bigint not null,
        user_id bigint not null,
        unique key uk_space_user (space_id, user_id)
    );";
    
    let tables = parse_sql_tables(sql).expect("解析失败");
    let table = tables.get("space_user").expect("应该有 space_user 表");
    
    println!("解析出的索引:");
    for idx in &table.indexes {
        println!("  - name: '{}', is_unique: {}, is_primary: {}, columns: {:?}", 
            idx.name, idx.is_unique, idx.is_primary, idx.columns);
    }
    
    // 查找唯一索引
    let unique_idx = table.indexes.iter()
        .find(|i| i.is_unique && !i.is_primary)
        .expect("应该有唯一索引");
    
    println!("唯一索引名: {}", unique_idx.name);
}
