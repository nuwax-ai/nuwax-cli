# Patch Upgrade Implementation

<cite>
**Referenced Files in This Document**   
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L463)
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)
- [sql_diff/differ.rs](file://client-core/src/sql_diff/differ.rs#L1-L266)
- [sql_diff/types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)
- [sql_diff/generator.rs](file://client-core/src/sql_diff/generator.rs#L1-L195)
- [sql_diff/parser.rs](file://client-core/src/sql_diff/parser.rs#L1-L381)
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L1-L379)
- [api_types.rs](file://client-core/src/api_types.rs#L1-L902)
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Patch Upgrade Strategy Decision](#patch-upgrade-strategy-decision)
3. [Patch Package Structure and Manifest](#patch-package-structure-and-manifest)
4. [Patch Application Process Flow](#patch-application-process-flow)
5. [File Operations Execution](#file-operations-execution)
6. [SQL Schema Migration and Synchronization](#sql-schema-migration-and-synchronization)
7. [Database Schema Synchronization with MySQL Executor](#database-schema-synchronization-with-mysql-executor)
8. [Error Handling, Rollback, and Recovery](#error-handling-rollback-and-recovery)
9. [Integration and Orchestration](#integration-and-orchestration)

## Introduction
The PatchUpgrade strategy is a core component of the Duck Client's update mechanism, designed to efficiently apply incremental updates when minor or patch versions change. This document details the implementation of the PatchUpgrade system, focusing on how it minimizes bandwidth usage by downloading only changed assets and applying atomic operations. The process involves parsing a patch manifest, applying SQL differences via the `sql_diff` module, executing file operations using the `patch_executor`, and synchronizing the database schema using the `mysql_executor`. The system ensures idempotency, supports partial failure recovery, and includes robust verification steps. This documentation provides a comprehensive overview of the architecture, data flow, and key components that enable secure and reliable incremental updates.

## Patch Upgrade Strategy Decision

The `UpgradeStrategyManager` is responsible for determining the appropriate upgrade strategy based on the current version, server manifest, and environmental factors. It evaluates whether a full upgrade, patch upgrade, or no upgrade is required.

```mermaid
flowchart TD
A[Start] --> B{Force Full Upgrade?}
B --> |Yes| C[Select FullUpgrade]
B --> |No| D{Docker Directory Exists?}
D --> |No| C
D --> |Yes| E{Version Comparison}
E --> |Equal or Newer| F[Select NoUpgrade]
E --> |PatchUpgradeable| G{Patch Available for Architecture?}
G --> |No| C
G --> |Yes| H[Select PatchUpgrade]
E --> |FullUpgradeRequired| C
```

**Diagram sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L463)

**Section sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L463)

## Patch Package Structure and Manifest

The `EnhancedServiceManifest` defines the structure of the server response, which includes information for both full and patch upgrades. The `PatchPackageInfo` contains the URL, hash, signature, and a detailed list of file operations to be performed.

```mermaid
classDiagram
class EnhancedServiceManifest {
+version : Version
+release_date : String
+release_notes : String
+packages : Option<ServicePackages>
+platforms : Option<PlatformPackages>
+patch : Option<PatchInfo>
}
class PatchInfo {
+x86_64 : Option<PatchPackageInfo>
+aarch64 : Option<PatchPackageInfo>
}
class PatchPackageInfo {
+url : String
+hash : Option<String>
+signature : Option<String>
+operations : PatchOperations
+notes : Option<String>
+get_changed_files() : Vec<String>
}
class PatchOperations {
+replace : Option<ReplaceOperations>
+delete : Option<ReplaceOperations>
+total_operations() : usize
}
class ReplaceOperations {
+files : Vec<String>
+directories : Vec<String>
+validate() : Result<(), DuckError>
}
EnhancedServiceManifest --> PatchInfo : "has"
PatchInfo --> PatchPackageInfo : "contains"
PatchPackageInfo --> PatchOperations : "defines"
PatchOperations --> ReplaceOperations : "uses"
```

**Diagram sources**
- [api_types.rs](file://client-core/src/api_types.rs#L1-L902)

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L1-L902)

## Patch Application Process Flow

The `PatchExecutor` orchestrates the entire patch application process, which includes downloading, verifying, extracting, and applying the patch. The process is designed to be transactional, with a rollback mechanism in place for failure recovery.

```mermaid
sequenceDiagram
participant Client as "Upgrade Manager"
participant Executor as "PatchExecutor"
participant Processor as "PatchProcessor"
participant FileOp as "FileOperationExecutor"
Client->>Executor : apply_patch(patch_info, operations)
Executor->>Executor : validate_preconditions()
Executor->>Processor : download_patch(patch_info)
Processor-->>Executor : patch_path
Executor->>Processor : verify_patch_integrity(patch_path, patch_info)
Processor-->>Executor : success
Executor->>Processor : extract_patch(patch_path)
Processor-->>Executor : extracted_path
Executor->>Executor : validate_patch_structure(extracted_path, operations)
Executor->>FileOp : set_patch_source(extracted_path)
Executor->>Executor : apply_patch_operations(extracted_path, operations)
loop For each operation
Executor->>FileOp : replace_files() or delete_items()
end
Executor-->>Client : Success
alt Failure
Executor->>Executor : rollback()
end
```

**Diagram sources**
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)

**Section sources**
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)

## File Operations Execution

The `FileOperationExecutor` handles the safe execution of file operations, including replacement and deletion. It supports a backup mode that enables rollback in case of failure. Operations are performed atomically to ensure data integrity.

```mermaid
flowchart TD
A[Start] --> B{Operation Type}
B --> |Replace File| C[Create Backup]
C --> D[Atomic File Replace]
D --> E[Log Success]
B --> |Replace Directory| F[Create Backup]
F --> G[Safe Remove Directory]
G --> H[Copy Directory]
H --> E
B --> |Delete Item| I{Exists?}
I --> |No| J[Skip]
I --> |Yes| K[Create Backup]
K --> L{Is Directory?}
L --> |Yes| M[Safe Remove Directory]
L --> |No| N[Remove File]
M --> E
N --> E
J --> E
```

**Diagram sources**
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)

**Section sources**
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)

## SQL Schema Migration and Synchronization

The `sql_diff` module is responsible for generating the SQL migration scripts that are used to synchronize the database schema. It parses the SQL files, compares the table definitions, and generates the necessary `ALTER TABLE` statements.

```mermaid
classDiagram
class SqlDiffGenerator {
+generate_schema_diff(from_sql, to_sql) : Result<(String, String)>
}
class SqlParser {
+parse_sql_tables(sql_content) : Result<HashMap<String, TableDefinition>>
}
class SqlDiffer {
+generate_mysql_diff(from_tables, to_tables) : Result<String>
+generate_table_diff(old_table, new_table) : Vec<String>
}
class SqlTypes {
+TableDefinition
+TableColumn
+TableIndex
}
SqlDiffGenerator --> SqlParser : "uses"
SqlDiffGenerator --> SqlDiffer : "uses"
SqlParser --> SqlTypes : "returns"
SqlDiffer --> SqlTypes : "uses"
```

**Diagram sources**
- [sql_diff/generator.rs](file://client-core/src/sql_diff/generator.rs#L1-L195)
- [sql_diff/parser.rs](file://client-core/src/sql_diff/parser.rs#L1-L381)
- [sql_diff/differ.rs](file://client-core/src/sql_diff/differ.rs#L1-L266)
- [sql_diff/types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)

**Section sources**
- [sql_diff/generator.rs](file://client-core/src/sql_diff/generator.rs#L1-L195)
- [sql_diff/parser.rs](file://client-core/src/sql_diff/parser.rs#L1-L381)
- [sql_diff/differ.rs](file://client-core/src/sql_diff/differ.rs#L1-L266)
- [sql_diff/types.rs](file://client-core/src/sql_diff/types.rs#L1-L31)

## Database Schema Synchronization with MySQL Executor

The `MySqlExecutor` is responsible for applying the generated SQL migration scripts to the database. It uses a transactional approach to ensure that all changes are applied atomically, with automatic rollback in case of failure.

```mermaid
sequenceDiagram
participant Executor as "MySqlExecutor"
participant DB as "MySQL Database"
participant Tx as "Transaction"
Executor->>Executor : execute_diff_sql(sql_content)
Executor->>Executor : parse_sql_commands(sql_content)
Executor->>DB : get_conn()
DB-->>Executor : conn
Executor->>Tx : start_transaction()
Tx-->>Executor : tx
loop For each SQL command
Executor->>Tx : query_drop(sql)
alt Success
Tx-->>Executor : success
else Failure
Executor->>Tx : rollback()
Tx-->>Executor : rolled back
Executor-->>Executor : retry or fail
end
end
Executor->>Tx : commit()
Tx-->>Executor : committed
Executor-->>Executor : return results
```

**Diagram sources**
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L1-L379)

**Section sources**
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L1-L379)

## Error Handling, Rollback, and Recovery

The system implements a comprehensive error handling and recovery strategy. The `PatchExecutor` can automatically initiate a rollback if an error occurs during the patch application. The `MySqlExecutor` uses database transactions to ensure atomicity, and the `FileOperationExecutor` uses backups to enable file-level rollback.

```mermaid
flowchart TD
A[Operation Starts] --> B{Success?}
B --> |Yes| C[Continue]
B --> |No| D{Requires Rollback?}
D --> |No| E[Report Error]
D --> |Yes| F{Backup Enabled?}
F --> |No| E
F --> |Yes| G[Initiate Rollback]
G --> H[Restore Files from Backup]
H --> I[Report Rollback Success]
I --> J[Report Original Error]
```

**Diagram sources**
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L1-L379)

**Section sources**
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L1-L379)

## Integration and Orchestration

The `UpgradeManager` in the `upgrade.rs` module serves as the main orchestrator, integrating the `UpgradeStrategyManager`, `PatchExecutor`, and `MySqlExecutor` to provide a seamless upgrade experience. It handles the high-level workflow, from checking for updates to applying the patch and verifying the result.

```mermaid
classDiagram
class UpgradeManager {
+check_for_updates() : Result<UpgradeStrategy>
}
class UpgradeStrategyManager {
+determine_strategy() : Result<UpgradeStrategy>
}
class PatchExecutor {
+apply_patch() : Result<(), PatchExecutorError>
}
class MySqlExecutor {
+execute_diff_sql() : Result<Vec<String>, anyhow : : Error>
}
UpgradeManager --> UpgradeStrategyManager : "uses"
UpgradeManager --> PatchExecutor : "uses"
UpgradeManager --> MySqlExecutor : "uses"
```

**Diagram sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L463)
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [mysql_executor.rs](file://client-core/src/mysql_executor.rs#L1-L379)

**Section sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)