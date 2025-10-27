use client_core::sql_diff::generate_schema_diff;
use std::fs;

/// This test reproduces the SQL syntax issues found in the upgrade diff generation
#[test]
fn test_sql_diff_syntax_issues() {
    println!("🔍 Testing SQL diff generation with realistic scenarios");

    // Simulate problematic inputs that might cause shell variable interpolation
    let old_sql = r#"
create table tenant (
    id bigint auto_increment primary key,
    name varchar(255) not null comment '商户名称',
    domain varchar(255) not null comment '域名'
) engine=InnoDB;

create table eco_market_client_config (
    id bigint auto_increment primary key,
    name varchar(255) not null
) engine=InnoDB;

create table eco_market_client_publish_config (
    id bigint auto_increment primary key,
    name varchar(255) not null
) engine=InnoDB;
"#;

    // New SQL with additions that might cause issues
    let new_sql = r#"
create table tenant (
    id bigint auto_increment primary key,
    name varchar(255) not null comment '商户名称',
    domain varchar(255) not null comment '域名'
) engine=InnoDB;

create table eco_market_client_config (
    id bigint auto_increment primary key,
    name varchar(255) not null,
    tenant_enabled tinyint(1) default 0 comment '是否租户自动启用插件,1:租户自动启用;0:非租户自动启用;默认:0',
    approve_message varchar(256) comment '审批原因'
) engine=InnoDB;

create table eco_market_client_publish_config (
    id bigint auto_increment primary key,
    name varchar(255) not null,
    approve_message varchar(256) comment '审批原因',
    tenant_enabled tinyint(1) default 0 comment '是否租户自动启用插件,1:租户自动启用;0:非租户自动启用;默认:0'
) engine=InnoDB;

-- Add unique constraints
ALTER TABLE `tenant` ADD UNIQUE KEY `uk_domain` (`domain`);
"#;

    println!("📋 Input files generated");

    // Generate the diff
    let result = generate_schema_diff(Some(old_sql), new_sql, Some("1.0.0"), "1.1.0");

    let (diff_sql, description) = result.expect("Failed to generate SQL diff");

    println!("✅ Diff generated successfully");
    println!("Description: {description}");

    // Analyze the generated SQL for potential issues
    let lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with("--"))
        .collect();

    println!("\n📊 Analyzing {} lines of SQL:", lines.len());

    // Check for specific issues
    let mut issues = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Check for TinyInt(None) - the issue seen in your file
        if trimmed.contains("TinyInt(None)") {
            issues.push(format!("Line {}: Invalid TinyInt(None) syntax", i + 1));
        }

        // Check for missing column types
        if trimmed.contains("ADD UNIQUE KEY  ()") {
            issues.push(format!("Line {}: Empty unique key constraint", i + 1));
        }

        // Check for incomplete ALTER statements
        if trimmed.contains("ALTER TABLE") && !trimmed.ends_with(';') {
            issues.push(format!(
                "Line {}: ALTER statement not properly terminated",
                i + 1
            ));
        }

        // Check for potential shell variables
        if trimmed.starts_with('$') || trimmed.contains("tenant:") || trimmed.contains("domain:") {
            issues.push(format!(
                "Line {}: Potential shell variable injection: '{}'",
                i + 1,
                trimmed
            ));
        }
    }

    println!("\n🔍 Issued detected:");
    if issues.is_empty() {
        println!("✅ No issues found");
    } else {
        for issue in &issues {
            println!("❌ {issue}");
        }

        println!("\n📄 Generated SQL for inspection:");
        println!("{diff_sql}\n");
    }

    // Test SQL syntax validation
    println!("🧪 Testing SQL syntax validation...\n");

    // Check if we can execute a sample of the diff as MySQL
    let valid_sql_lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--")
        })
        .filter(|line| {
            // Remove low confidence ones
            !line.contains("TinyInt(None)")
                && !line.contains("ADD UNIQUE KEY  ()")
                && !line.starts_with('$')
        })
        .collect();

    println!("📋 Valid SQL lines:");
    for line in valid_sql_lines {
        println!("  {line}");
    }

    // This shouldn't fail with the current issue
    // Test is now expected to pass after fixes
    assert!(issues.is_empty(), "No issues should be found after fixes");
}

#[test]
fn test_sql_diff_parser_with_real_fixtures() {
    // Test with actual fixture files
    let project_root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixtures_path = std::path::Path::new(&project_root).join("fixtures");

    let old_sql_path = fixtures_path.join("old_init_mysql.sql");
    let new_sql_path = fixtures_path.join("new_init_mysql.sql");

    if old_sql_path.exists() && new_sql_path.exists() {
        let old_sql = fs::read_to_string(&old_sql_path).expect("Failed to read old SQL file");
        let new_sql = fs::read_to_string(&new_sql_path).expect("Failed to read new SQL file");

        let result = generate_schema_diff(Some(&old_sql), &new_sql, Some("production"), "dev");

        let (diff_sql, _) = result.expect("Failed to generate diff from fixtures");

        // Validate the generated SQL doesn't contain problematic patterns
        let invalid_patterns = vec![
            "TinyInt(None)",
            "BigInt(None)",
            "VARCHAR(None)",
            "ADD UNIQUE KEY  ()",
            "$tenant",
            "$domain",
            "tenant: command not found",
        ];

        let mut found_issues = Vec::new();

        for pattern in invalid_patterns {
            if diff_sql.contains(pattern) {
                found_issues.push(pattern.to_string());
            }
        }

        if !found_issues.is_empty() {
            println!("❌ Found problematic patterns in SQL diff:");
            for issue in &found_issues {
                println!("  - {issue}");
            }
            println!("\nGenerated diff:\n{diff_sql}");

            panic!("SQL generation produced invalid syntax");
        }

        println!("✅ SQL diff generation passed validation");
    } else {
        println!("⚠️  Fixture files not found, skipping full fixture test");
    }
}

#[test]
fn test_mysql_type_generation_fixes() {
    // Test specific type generation issues

    let old_sql = "create table test_table (id bigint primary key);";
    let new_sql = "create table test_table (
        id bigint primary key,
        enabled tinyint(1) default 0,
        count smallint(5) unsigned default 0
    );";

    let (diff_sql, _) = generate_schema_diff(Some(old_sql), new_sql, Some("1.0"), "1.1")
        .expect("Should generate diff");

    println!("Generated diff for type fixes:");
    println!("{diff_sql}");

    // Assert no TinyInt(None) or similar issues
    assert!(!diff_sql.contains("TinyInt(None)"));
    assert!(
        !diff_sql.contains("SmallInt(Some(5))"),
        "Should not contain SmallInt(Some(5)) format"
    );
    assert!(diff_sql.contains("TINYINT"), "Should contain TINYINT");
    // Adjust for actual output format
    assert!(
        diff_sql.contains("SmallInt") || diff_sql.contains("SMALLINT"),
        "Should contain SMALLINT variant"
    );
}
