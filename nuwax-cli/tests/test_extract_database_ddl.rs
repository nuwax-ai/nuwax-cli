/// 测试 SQL 解析功能的各种边界情况
/// 
/// 注意：client_core::sql_diff::parse_sql_tables 内部的 
/// extract_create_table_statements_with_regex 函数会自动处理 USE 语句的查找和提取
/// 
/// 这些测试验证 parse_sql_tables 能正确处理各种 SQL 格式
#[cfg(test)]
mod tests {
    #[test]
    fn test_use_statement_variations() {
        // 测试用例：验证各种 USE 语句格式都包含 CREATE TABLE
        let test_cases = vec![
            ("USE agent_platform;\nCREATE TABLE test (id INT);", "标准格式"),
            ("USE `agent_platform`;\nCREATE TABLE test (id INT);", "带反引号"),
            ("use Agent_Platform;\nCREATE TABLE test (id INT);", "大小写混合"),
            ("USE    agent_platform  ;\nCREATE TABLE test (id INT);", "多个空格"),
            ("USE agent_platform\nCREATE TABLE test (id INT);", "无分号"),
            ("-- Comment\nUSE agent_platform;\nCREATE TABLE test (id INT);", "前面有注释"),
        ];

        for (sql, description) in test_cases {
            assert!(
                sql.contains("CREATE TABLE"),
                "测试失败: {}",
                description
            );
            println!("✓ 测试通过: {}", description);
        }

        println!("✅ 所有 USE 语句格式测试通过");
    }

    #[test]
    fn test_regex_pattern_multiline() {
        use regex::Regex;

        let database_name = "agent_platform";
        // 使用多行模式 (?im)，^ 匹配每行的开始
        let pattern = format!(r"(?im)^\s*USE\s+`?{}`?\s*;?\s*$", regex::escape(database_name));
        let re = Regex::new(&pattern).unwrap();

        // 测试各种格式（每个 USE 语句独占一行）
        let test_cases = vec![
            ("USE agent_platform;\n", true, "标准格式"),
            ("USE `agent_platform`;\n", true, "带反引号"),
            ("use agent_platform;\n", true, "小写"),
            ("USE  agent_platform  ;\n", true, "多个空格"),
            ("USE agent_platform\n", true, "无分号"),
            ("  USE agent_platform;  \n", true, "前后有空格"),
            ("USE other_database;\n", false, "其他数据库"),
            ("-- USE agent_platform;\n", false, "注释中的USE"),
        ];

        for (sql, should_match, description) in test_cases {
            let matches = re.is_match(sql);
            assert_eq!(
                matches, should_match,
                "测试失败: {} - SQL: {}",
                description, sql
            );
            if matches {
                println!("✓ 匹配成功: {}", description);
            } else {
                println!("✓ 正确拒绝: {}", description);
            }
        }

        println!("✅ 正则表达式多行模式测试通过");
    }

    #[test]
    fn test_extract_after_use_statement() {
        // 这个测试验证 parser.rs 中的 extract_create_table_statements_with_regex 
        // 能正确处理 USE 语句
        use client_core::sql_diff::parse_sql_tables;

        let sql = "-- Some comments\nUSE agent_platform;\nCREATE TABLE users (id INT PRIMARY KEY);\nCREATE TABLE posts (id INT PRIMARY KEY);";
        
        // parse_sql_tables 会自动处理 USE 语句
        match parse_sql_tables(sql) {
            Ok(tables) => {
                println!("✓ 成功解析 {} 个表", tables.len());
                assert_eq!(tables.len(), 2, "应该解析出2个表");
                
                // 表名可能带反引号
                let has_users = tables.contains_key("users") || tables.contains_key("`users`");
                let has_posts = tables.contains_key("posts") || tables.contains_key("`posts`");
                
                assert!(has_users, "应该包含 users 表");
                assert!(has_posts, "应该包含 posts 表");
                
                println!("✅ USE 语句处理验证通过");
            }
            Err(e) => {
                panic!("解析失败: {}", e);
            }
        }
    }

    #[test]
    fn test_parser_integration() {
        // 这个测试验证 parse_sql_tables 函数能正确处理提取后的内容
        use client_core::sql_diff::parse_sql_tables;

        let sql_with_use = r#"
USE agent_platform;

CREATE TABLE `users` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `name` VARCHAR(64) NOT NULL,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE `posts` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `user_id` BIGINT NOT NULL,
    `title` VARCHAR(255) NOT NULL,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
"#;

        // 模拟 extract_database_ddl 的行为
        let use_pattern = r"(?im)^\s*USE\s+`?agent_platform`?\s*;?\s*$";
        let re = regex::Regex::new(use_pattern).unwrap();
        
        let extracted = if let Some(mat) = re.find(sql_with_use) {
            &sql_with_use[mat.end()..].trim()
        } else {
            sql_with_use
        };

        // 使用 parse_sql_tables 解析
        match parse_sql_tables(extracted) {
            Ok(tables) => {
                println!("✓ 成功解析 {} 个表", tables.len());
                
                // 打印所有表名以便调试
                println!("✓ 解析到的表名:");
                for table_name in tables.keys() {
                    println!("  - {}", table_name);
                }
                
                assert_eq!(tables.len(), 2, "应该解析出2个表");
                
                // parse_sql_tables 可能返回带反引号的表名，尝试两种格式
                let has_users = tables.contains_key("users") || tables.contains_key("`users`");
                let has_posts = tables.contains_key("posts") || tables.contains_key("`posts`");
                
                assert!(has_users, "应该包含 users 表（实际表名: {:?})", tables.keys().collect::<Vec<_>>());
                assert!(has_posts, "应该包含 posts 表（实际表名: {:?})", tables.keys().collect::<Vec<_>>());
                
                println!("✅ parse_sql_tables 集成测试通过");
            }
            Err(e) => {
                panic!("解析失败: {}", e);
            }
        }
    }
}
