// 用于测试和演示SQL差异的专门测试文件
// 这个文件读取真实fixtures并展示具体的差异结果

use client_core::sql_diff::generate_schema_diff;
use std::fs;

use std::path::Path;

/// 读取fixtures文件夹中的SQL文件内容
fn read_fixture_file(filename: &str) -> String {
    let project_root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixture_path = Path::new(&project_root).join("fixtures").join(filename);
    fs::read_to_string(&fixture_path).unwrap_or_else(|_| panic!("无法读取文件: {fixture_path:?}"))
}

#[test]
fn demo_real_world_diff_sql() {
    println!("🚀 开始演示真实世界的SQL差异分析");
    println!("{}", "=".repeat(60));

    // 读取测试文件
    let old_sql = read_fixture_file("old_init_mysql.sql");
    let new_sql = read_fixture_file("new_init_mysql.sql");

    println!("📊 文件统计信息:");
    println!("旧文件行数: {}", old_sql.lines().count());
    println!("新文件行数: {}", new_sql.lines().count());
    println!();

    // 执行差异分析
    let start_time = std::time::Instant::now();
    let result = generate_schema_diff(
        Some(&old_sql),
        &new_sql,
        Some("2025.07.22-old"),
        "2025.07.22-new",
    );
    let duration = start_time.elapsed();

    let (diff_sql, description) = result.expect("差异分析失败");

    let changes_count = diff_sql
        .lines()
        .filter(|line| {
            !line.trim().is_empty()
                && !line.trim().starts_with("--")
                && !line.trim().starts_with("/*")
        })
        .count();

    println!("📈 分析结果:");
    println!("分析耗时: {duration:?}");
    println!("差异描述: {description}");
    println!("有效SQL行数: {changes_count}");
    println!();

    if diff_sql.is_empty() {
        println!("ℹ️  没有发现实际的数据库架构差异");
    } else {
        println!("🔍 完整的差异SQL:");
        println!("{}", "-".repeat(60));
        println!("{}", diff_sql.trim_end());
        println!("{}", "-".repeat(60));
    }
}

#[test]
fn analyze_structural_changes() {
    println!("🏗️  详细结构变化分析");
    println!("{}", "=".repeat(60));

    let old_sql = read_fixture_file("old_init_mysql.sql");
    let new_sql = read_fixture_file("new_init_mysql.sql");

    let (diff_sql, _) = generate_schema_diff(Some(&old_sql), &new_sql, Some("old"), "new").unwrap();

    // 提取关键的变化类型
    let mut changes = Vec::new();

    if diff_sql.contains("CREATE TABLE") {
        changes.push("新增表");
    }
    if diff_sql.contains("DROP TABLE") {
        changes.push("删除表");
    }
    if diff_sql.contains("ADD COLUMN") {
        changes.push("添加列");
    }
    if diff_sql.contains("DROP COLUMN") {
        changes.push("删除列");
    }
    if diff_sql.contains("MODIFY COLUMN") {
        changes.push("修改列");
    }
    if diff_sql.contains("ADD UNIQUE KEY") || diff_sql.contains("ADD UNIQUE") {
        changes.push("添加唯一索引");
    }
    if diff_sql.contains("ADD PRIMARY KEY") {
        changes.push("添加主键");
    }
    if diff_sql.contains("ADD KEY") && !diff_sql.contains("UNIQUE") {
        changes.push("添加普通索引");
    }
    if diff_sql.contains("DROP KEY") || diff_sql.contains("DROP INDEX") {
        changes.push("删除索引");
    }

    println!("📋 变化类型统计:");
    for change in &changes {
        println!("  - {change}");
    }

    if changes.is_empty() {
        println!("未检测到数据表变化");
    } else {
        println!("总共发现 {} 种变化类型", changes.len());
    }
}

#[test]
fn detect_specific_changes() {
    println!("🎯 特定变化检测");
    println!("{}", "=".repeat(60));

    let old_sql = read_fixture_file("old_init_mysql.sql");
    let new_sql = read_fixture_file("new_init_mysql.sql");

    let (diff_sql, _) =
        generate_schema_diff(Some(&old_sql), &new_sql, Some("baseline"), "target").unwrap();

    let mut specific_changes = Vec::new();

    // 检查特定的变化
    let lines: Vec<&str> = diff_sql.lines().collect();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.contains("MODIFY COLUMN") {
            specific_changes.push(red("修改列: ") + trimmed);
        } else if trimmed.contains("ADD COLUMN") {
            specific_changes.push(blue("添加列: ") + trimmed);
        } else if trimmed.contains("DROP COLUMN") {
            specific_changes.push(yellow("删除列: ") + trimmed);
        } else if trimmed.contains("ADD UNIQUE KEY") {
            specific_changes.push(green("添加唯一索引: ") + trimmed);
        } else if trimmed.contains("ADD KEY") && !trimmed.contains("UNIQUE") {
            specific_changes.push(purple("添加普通索引: ") + trimmed);
        }
    }

    println!("✨ 详细变化列表:");
    for change in &specific_changes {
        println!("{change}");
    }

    if specific_changes.is_empty() {
        println!("未检测到具体变化（可能只有注释或格式变化）");
    }
}

// 输出着色功能的简单实现
fn red(text: &str) -> String {
    format!("\x1b[31m{text}\x1b[0m")
}
fn green(text: &str) -> String {
    format!("\x1b[32m{text}\x1b[0m")
}
fn blue(text: &str) -> String {
    format!("\x1b[34m{text}\x1b[0m")
}
fn yellow(text: &str) -> String {
    format!("\x1b[33m{text}\x1b[0m")
}
fn purple(text: &str) -> String {
    format!("\x1b[35m{text}\x1b[0m")
}

/// 运行所有演示测试的简单入口
pub fn run_all_diff_tests() {
    println!("🎯 开始执行SQL差异演示测试套件");
    println!("正在分析新旧SQL文件的差异...\n");

    demo_real_world_diff_sql();
    println!("\n");

    analyze_structural_changes();
    println!("\n");

    detect_specific_changes();

    println!("\n🎉 所有测试完成！");
}

#[test]
fn test_fixtures_direct_output() {
    println!("🔧 直接测试-fixtures文件夹SQL差异");

    let project_root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixtures_path = Path::new(&project_root).join("fixtures");

    println!("🗂 Fixtures路径: {fixtures_path:?}");

    let old_sql_path = fixtures_path.join("old_init_mysql.sql");
    let new_sql_path = fixtures_path.join("new_init_mysql.sql");

    let old_sql = fs::read_to_string(&old_sql_path).unwrap_or_else(|_| {
        panic!(
            "无法读取旧文件: {:?}\n请确保文件存在于: {:?}",
            old_sql_path,
            old_sql_path.parent()
        )
    });

    let new_sql = fs::read_to_string(&new_sql_path).unwrap_or_else(|_| {
        panic!(
            "无法读取新文件: {:?}\n请确保文件存在于: {:?}",
            new_sql_path,
            new_sql_path.parent()
        )
    });

    println!("✅ 成功读取两个文件");
    println!("旧文件: {} 行", old_sql.lines().count());
    println!("新文件: {} 行", new_sql.lines().count());

    let (diff_sql, description) =
        generate_schema_diff(Some(&old_sql), &new_sql, Some("prod-old"), "dev-new").unwrap();

    println!("\n📊 分析完成:");
    println!("结果: {description}");

    // 展示详细的差异
    let mut details = Vec::new();
    for line in diff_sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ALTER TABLE") {
            details.push(format!("\n🔹 {trimmed}"));
        } else if trimmed.contains("ADD COLUMN") {
            details.push(format!("  ➕ {trimmed}"));
        } else if trimmed.contains("ADD UNIQUE KEY") || trimmed.contains("ADD KEY") {
            details.push(format!("  🔑 {trimmed}"));
        } else if !trimmed.starts_with("--") && !trimmed.is_empty() {
            details.push(format!("    {trimmed}"));
        }
    }

    println!("\n📋 详细变更:");
    for detail in &details {
        println!("{detail}");
    }

    if !diff_sql.is_empty() {
        println!("\n📄 完整差异SQL:");
        println!("{diff_sql}");
    }
}

// 用于命令行直接调用的函数
#[cfg(test)]
pub fn pretty_print_diff() {
    use std::io::Write;

    println!("\n🚀 real world diff sql analysis");
    println!("{}", "╔".repeat(80));

    let old_sql = read_fixture_file("old_init_mysql.sql");
    let new_sql = read_fixture_file("new_init_mysql.sql");

    let (diff_sql, description) = generate_schema_diff(
        Some(&old_sql),
        &new_sql,
        Some("production-baseline"),
        "development-target",
    )
    .unwrap();

    println!("\n📊 Summary");
    println!("├── From: production-baseline");
    println!("├── To: development-target");
    println!("├── Changes: {description}");
    println!(
        "└── SQL Lines: {}",
        diff_sql.lines().filter(|l| !l.trim().is_empty()).count()
    );

    if !diff_sql.is_empty() {
        println!("\n╼╼╼╼╼╼ Generated SQL Migration ╼╼╼╼╼╼");
        println!("{diff_sql}");
    } else {
        println!("\n✅ No structural changes detected");
    }

    println!("{}", "╚".repeat(80));

    // 清理和验证
    std::io::stdout().flush().unwrap();
}
