# SQL Diff Generation

<cite>
**Referenced Files in This Document**   
- [differ.rs](file://client-core/src/sql_diff/differ.rs)
- [generator.rs](file://client-core/src/sql_diff/generator.rs)
- [types.rs](file://client-core/src/sql_diff/types.rs)
- [parser.rs](file://client-core/src/sql_diff/parser.rs)
- [mod.rs](file://client-core/src/sql_diff/mod.rs)
- [tests.rs](file://client-core/src/sql_diff/tests.rs)
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs)
- [old_init_mysql.sql](file://client-core/fixtures/old_init_mysql.sql)
- [new_init_mysql.sql](file://client-core/fixtures/new_init_mysql.sql)
- [diff_sql_tests.rs](file://client-core/tests/diff_sql_tests.rs)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Core Components Overview](#core-components-overview)
3. [Schema Parsing Mechanism](#schema-parsing-mechanism)
4. [Difference Detection Algorithm](#difference-detection-algorithm)
5. [SQL Generation Process](#sql-generation-process)
6. [Data Structure Representation](#data-structure-representation)
7. [Integration with Execution Pipeline](#integration-with-execution-pipeline)
8. [Edge Case Handling](#edge-case-handling)
9. [Validation and Testing](#validation-and-testing)
10. [Performance Characteristics](#performance-characteristics)

## Introduction
The SQL diff generation system is a core component of the database migration framework that detects and generates schema differences between two database states. This system analyzes SQL schema definitions and produces executable migration scripts that can be applied to evolve database structures safely and efficiently. The implementation focuses on MySQL compatibility and handles various structural changes including table additions, column modifications, index changes, and constraint updates. The system is designed to be robust, accurate, and safe for production use.

## Core Components Overview

The SQL diff generation system consists of several interconnected components that work together to analyze schema differences and generate migration scripts. The main components include the parser, differ, generator, and type definitions, each responsible for a specific aspect of the diff generation process.

```mermaid
graph TD
A["generate_schema_diff<br/>Entry Point"] --> B["parse_sql_tables<br/>Schema Parser"]
B --> C["TableDefinition<br/>Internal Representation"]
C --> D["generate_mysql_diff<br/>Difference Detector"]
D --> E["generate_create_table_sql<br/>SQL Generator"]
E --> F["Executable SQL Patch"]
G["MySQLExecutor<br/>Execution Engine"] --> F
style A fill:#f9f,stroke:#333
style F fill:#bbf,stroke:#333
```

**Diagram sources**
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)
- [parser.rs](file://client-core/src/sql_diff/parser.rs#L8-L381)
- [differ.rs](file://client-core/src/sql_diff/differ.rs#L8-L266)
- [types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)

**Section sources**
- [mod.rs](file://client-core/src/sql_diff/mod.rs#L1-L11)
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)

## Schema Parsing Mechanism

The schema parsing mechanism extracts table definitions from raw SQL content by identifying CREATE TABLE statements and converting them into structured data objects. The parser uses a combination of regular expressions and SQL parsing libraries to accurately extract schema information while ignoring irrelevant statements.

```mermaid
flowchart TD
Start["Input SQL Content"] --> Extract["Extract CREATE TABLE Statements"]
Extract --> Parse["Parse with SQLParser"]
Parse --> Process["Process Column Definitions"]
Process --> Constraints["Extract Constraints & Indexes"]
Constraints --> Map["Create TableDefinition Object"]
Map --> Output["HashMap<String, TableDefinition>"]
subgraph "Statement Extraction"
Extract --> Regex{"Regex Pattern Match?"}
Regex --> |Yes| Collect["Collect Statement Content"]
Regex --> |No| Skip["Skip Line"]
Collect --> Balance["Parentheses Balancing"]
Balance --> Complete{"Complete Statement?"}
Complete --> |Yes| Add["Add to Statements"]
Complete --> |No| Continue["Continue Reading"]
end
subgraph "Object Creation"
Process --> Column["Create TableColumn Objects"]
Constraints --> Index["Create TableIndex Objects"]
Column --> Combine["Combine into TableDefinition"]
Index --> Combine
end
```

**Diagram sources**
- [parser.rs](file://client-core/src/sql_diff/parser.rs#L8-L381)
- [types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)

**Section sources**
- [parser.rs](file://client-core/src/sql_diff/parser.rs#L8-L381)

## Difference Detection Algorithm

The difference detection algorithm compares two sets of table definitions and identifies structural changes between them. The algorithm operates at multiple levels: table-level, column-level, and index-level differences, ensuring comprehensive coverage of all possible schema modifications.

```mermaid
flowchart TD
Start["Compare from_tables and to_tables"] --> Tables["Table-Level Changes"]
Tables --> New["New Tables: Not in from_tables"]
Tables --> Deleted["Deleted Tables: Not in to_tables"]
Tables --> Modified["Modified Tables: In Both"]
Modified --> Columns["Column-Level Changes"]
Columns --> Added["New Columns"]
Columns --> Removed["Removed Columns"]
Columns --> Changed["Modified Columns"]
Modified --> Indexes["Index-Level Changes"]
Indexes --> NewIndex["New Indexes"]
Indexes --> RemovedIndex["Removed Indexes"]
Indexes --> TypeChange["Index Type Changes"]
New --> SQL["Generate CREATE TABLE"]
Deleted --> SQL["Generate DROP TABLE"]
Added --> SQL["Generate ADD COLUMN"]
Removed --> SQL["Generate DROP COLUMN"]
Changed --> SQL["Generate MODIFY COLUMN"]
NewIndex --> SQL["Generate ADD INDEX"]
RemovedIndex --> SQL["Generate DROP INDEX"]
SQL --> Result["Accumulate All SQL Statements"]
Result --> Output["Return Complete Diff SQL"]
```

**Diagram sources**
- [differ.rs](file://client-core/src/sql_diff/differ.rs#L8-L266)
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)

**Section sources**
- [differ.rs](file://client-core/src/sql_diff/differ.rs#L8-L266)

## SQL Generation Process

The SQL generation process transforms detected differences into executable MySQL statements that can be applied to migrate a database from one schema version to another. The process ensures that generated SQL is syntactically correct, properly ordered, and includes appropriate comments for traceability.

```mermaid
classDiagram
class SchemaDiffGenerator {
+generate_schema_diff(from_sql : Option<&str>, to_sql : &str, from_version : Option<&str>, to_version : &str) -> Result<(String, String), DuckError>
}
class MySqlDiffGenerator {
+generate_mysql_diff(from_tables : &HashMap<String, TableDefinition>, to_tables : &HashMap<String, TableDefinition>) -> Result<String, DuckError>
+generate_table_diff(old_table : &TableDefinition, new_table : &TableDefinition) -> Vec<String>
+generate_column_diffs(old_table : &TableDefinition, new_table : &TableDefinition) -> Vec<String>
+generate_index_diffs(old_table : &TableDefinition, new_table : &TableDefinition) -> Vec<String>
}
class SqlGenerator {
+generate_create_table_sql(table : &TableDefinition) -> String
+generate_column_sql(column : &TableColumn) -> String
+generate_index_sql(index : &TableIndex) -> String
}
SchemaDiffGenerator --> MySqlDiffGenerator : "delegates"
MySqlDiffGenerator --> SqlGenerator : "uses"
MySqlDiffGenerator --> TableDefinition : "compares"
SqlGenerator --> TableColumn : "formats"
SqlGenerator --> TableIndex : "formats"
```

**Diagram sources**
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)
- [differ.rs](file://client-core/src/sql_diff/differ.rs#L8-L266)
- [types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)

**Section sources**
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)

## Data Structure Representation

The system uses a set of Rust structs to represent database schema elements in memory. These data structures provide a type-safe and efficient way to work with schema information during the diff generation process.

```mermaid
classDiagram
class TableColumn {
+name : String
+data_type : String
+nullable : bool
+default_value : Option<String>
+auto_increment : bool
+comment : Option<String>
}
class TableIndex {
+name : String
+columns : Vec<String>
+is_primary : bool
+is_unique : bool
+index_type : Option<String>
}
class TableDefinition {
+name : String
+columns : Vec<TableColumn>
+indexes : Vec<TableIndex>
+engine : Option<String>
+charset : Option<String>
}
class SchemaParser {
+parse_sql_tables(sql_content : &str) -> Result<HashMap<String, TableDefinition>, DuckError>
+extract_create_table_statements_with_regex(sql_content : &str) -> Result<Vec<String>, DuckError>
+parse_column_definition(column : &ColumnDef) -> Result<TableColumn, DuckError>
+parse_table_constraint(constraint : &TableConstraint) -> Result<Option<TableIndex>, DuckError>
}
TableColumn <-- TableDefinition : "contained in"
TableIndex <-- TableDefinition : "contained in"
SchemaParser --> TableDefinition : "creates"
SchemaParser --> TableColumn : "creates"
SchemaParser --> TableIndex : "creates"
```

**Diagram sources**
- [types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)
- [parser.rs](file://client-core/src/sql_diff/parser.rs#L8-L381)

**Section sources**
- [types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)

## Integration with Execution Pipeline

The generated SQL differences are integrated with the MySQL execution pipeline through the MySqlExecutor component, which handles the application of migration scripts to target databases. This integration ensures that generated patches can be safely and reliably applied.

```mermaid
sequenceDiagram
participant UI as "CLI/UI"
participant Generator as "SchemaDiffGenerator"
participant Executor as "MySqlExecutor"
participant Database as "MySQL Database"
UI->>Generator : generate_schema_diff(old_sql, new_sql)
Generator-->>UI : (diff_sql, description)
UI->>Executor : execute_diff_sql(diff_sql)
Executor->>Executor : parse_sql_commands()
Executor->>Executor : start_transaction()
loop Each SQL Command
Executor->>Database : query_drop(sql)
Database-->>Executor : result
Executor->>Executor : accumulate_results()
end
Executor->>Executor : commit_transaction()
Executor-->>UI : results
```

**Diagram sources**
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L0-L379)

**Section sources**
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L0-L379)

## Edge Case Handling

The system includes comprehensive handling of edge cases and special scenarios that commonly occur during schema evolution. These include handling of identical schemas, initial version creation, and various types of structural modifications.

```mermaid
flowchart TD
Start["Input Schemas"] --> Check["Validation Checks"]
subgraph "Initial Checks"
Check --> Empty{"from_sql is None?"}
Empty --> |Yes| Initial["Return full to_sql as creation script"]
Empty --> |No| Identical{"Content identical?"}
Identical --> |Yes| NoChange["Return empty diff"]
Identical --> |No| Parse["Parse both schemas"]
end
Parse --> Analyze["Analyze Differences"]
subgraph "Column Change Types"
Analyze --> Length{"VARCHAR length change?"}
Length --> |Yes| Modify["Use MODIFY COLUMN"]
Analyze --> Nullability{"NULL to NOT NULL?"}
Nullability --> |Yes| Modify
Analyze --> DataType{"Data type change?"}
DataType --> |Yes| Modify
Analyze --> Default{"Default value change?"}
Default --> |Yes| Modify
Analyze --> Comment{"Comment change?"}
Comment --> |Yes| Modify
end
subgraph "Index Operations"
Analyze --> Unique{"Add UNIQUE constraint?"}
Unique --> |Yes| AddUnique["ADD UNIQUE KEY"]
Analyze --> Primary{"Add PRIMARY KEY?"}
Primary --> |Yes| AddPrimary["ADD PRIMARY KEY"]
Analyze --> Regular{"Add regular index?"}
Regular --> |Yes| AddIndex["ADD KEY"]
end
Modify --> Generate["Generate ALTER TABLE"]
AddUnique --> Generate
AddPrimary --> Generate
AddIndex --> Generate
Generate --> Result["Accumulate SQL statements"]
Result --> Output["Return complete diff"]
```

**Section sources**
- [generator.rs](file://client-core/src/sql_diff/generator.rs#L8-L194)
- [differ.rs](file://client-core/src/sql_diff/differ.rs#L8-L266)
- [tests.rs](file://client-core/src/sql_diff/tests.rs#L0-L872)

## Validation and Testing

The system includes extensive testing to validate its correctness and reliability. Tests cover various scenarios including simple column additions, complex structural changes, and edge cases like comment modifications and constraint changes.

```mermaid
graph TB
TestSuite["SQL Diff Test Suite"] --> Simple["Simple Diff Tests"]
TestSuite --> AddTable["Add Table Tests"]
TestSuite --> DropTable["Drop Table Tests"]
TestSuite --> ModifyColumn["Modify Column Tests"]
TestSuite --> AddIndex["Add Index Tests"]
TestSuite --> RemoveIndex["Remove Index Tests"]
TestSuite --> Complex["Complex Schema Tests"]
TestSuite --> RealWorld["Real-World Schema Tests"]
Simple --> |test_simple_diff| Verify["Verify ADD COLUMN detection"]
AddTable --> |test_add_table| Verify
DropTable --> |test_drop_table| Verify["Verify DROP TABLE generation"]
ModifyColumn --> |test_modify_column| Verify["Verify MODIFY COLUMN usage"]
AddIndex --> |test_add_index| Verify["Verify ADD INDEX/KEY"]
RemoveIndex --> |test_tenant_unique_index_removal| Verify["Verify DROP KEY"]
Complex --> |test_complex_column_modifications| Verify["Verify multiple changes"]
RealWorld --> |demo_real_world_diff_sql| Verify["Verify fixture-based diffs"]
style Simple fill:#f96,stroke:#333
style AddTable fill:#f96,stroke:#333
style DropTable fill:#f96,stroke:#333
style ModifyColumn fill:#f96,stroke:#333
style AddIndex fill:#f96,stroke:#333
style RemoveIndex fill:#f96,stroke:#333
style Complex fill:#f96,stroke:#333
style RealWorld fill:#f96,stroke:#333
```

**Diagram sources**
- [tests.rs](file://client-core/src/sql_diff/tests.rs#L0-L872)
- [diff_sql_tests.rs](file://client-core/tests/diff_sql_tests.rs#L0-L298)

**Section sources**
- [tests.rs](file://client-core/src/sql_diff/tests.rs#L0-L872)

## Performance Characteristics

The SQL diff generation system is designed with performance considerations for handling large schemas efficiently. The implementation uses HashMap lookups for O(1) average-case complexity when comparing table and column definitions, and processes schemas in a single pass where possible.

```mermaid
erDiagram
TABLE_DEFINITION ||--o{ COLUMN_DEFINITION : "contains"
TABLE_DEFINITION ||--o{ INDEX_DEFINITION : "contains"
SCHEMA_DIFF_RESULT ||--o{ SQL_STATEMENT : "comprises"
EXECUTION_PLAN ||--o{ DATABASE_OPERATION : "sequences"
TABLE_DEFINITION {
string name PK
string engine
string charset
}
COLUMN_DEFINITION {
string name PK
string data_type
boolean nullable
string default_value
boolean auto_increment
string comment
}
INDEX_DEFINITION {
string name PK
string columns
boolean is_primary
boolean is_unique
string index_type
}
SQL_STATEMENT {
int order PK
string type
string content
string table_name
}
DATABASE_OPERATION {
int sequence PK
string operation_type
string target_object
string sql_command
datetime estimated_time
}
```

**Section sources**
- [differ.rs](file://client-core/src/sql_diff/differ.rs#L8-L266)
- [parser.rs](file://client-core/src/sql_diff/parser.rs#L8-L381)