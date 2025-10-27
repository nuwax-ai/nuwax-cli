# Architecture Overview

<cite>
**Referenced Files in This Document**   
- [mod.rs](file://client-core/src/patch_executor/mod.rs)
- [file_operations.rs](file://client-core/src/patch_executor/file_operations.rs)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs)
- [api_types.rs](file://client-core/src/api_types.rs)
</cite>

## Table of Contents
1. [Architecture Overview](#architecture-overview)
2. [Core Components](#core-components)
3. [Upgrade Strategy Pattern](#upgrade-strategy-pattern)
4. [Observer Pattern for Progress Monitoring](#observer-pattern-for-progress-monitoring)
5. [System Boundaries and Integration Points](#system-boundaries-and-integration-points)
6. [Data Flow Analysis](#data-flow-analysis)
7. [Cross-Cutting Concerns](#cross-cutting-concerns)

## Architecture Overview

The duck_client system is a monorepo-based application structured around three primary components: a Tauri-based GUI (cli-ui), a standalone CLI tool (nuwax-cli), and a shared core library (client-core) that encapsulates business logic. This architecture enables code reuse while supporting multiple user interfaces.

The system follows an MVC-like separation pattern:
- **View**: React components in the cli-ui frontend
- **Controller**: Tauri commands that bridge frontend and backend
- **Model**: client-core library handling business logic, state management, and external interactions

Both the GUI and CLI clients consume the client-core library, ensuring consistent behavior across interfaces. The monorepo structure with workspace-managed dependencies allows for coordinated development and versioning of these components.

```mermaid
graph TB
subgraph "User Interfaces"
GUI["Tauri GUI (cli-ui)"]
CLI["Standalone CLI (nuwax-cli)"]
end
subgraph "Core Logic"
CORE["Shared Core (client-core)"]
end
subgraph "External Systems"
DOCKER["Docker Engine"]
REMOTE["Remote Server"]
STORAGE["Local Storage"]
end
GUI --> CORE
CLI --> CORE
CORE --> DOCKER
CORE --> REMOTE
CORE --> STORAGE
style GUI fill:#4CAF50,stroke:#388E3C
style CLI fill:#2196F3,stroke:#1976D2
style CORE fill:#FF9800,stroke:#F57C00
style DOCKER fill:#9C27B0,stroke:#7B1FA2
style REMOTE fill:#607D8B,stroke:#455A64
style STORAGE fill:#795548,stroke:#5D4037
```

**Diagram sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs)

## Core Components

The client-core library contains the essential business logic for the duck_client system, with key components organized into modular crates. The architecture separates concerns into distinct modules:

- **Container Management**: Handles Docker container operations, configuration, and service management
- **Database Operations**: Manages database connections, migrations, and SQL operations
- **Patch Execution**: Implements the core patch application logic with safety features
- **Upgrade Strategy**: Determines optimal upgrade paths based on version compatibility and system constraints
- **Configuration Management**: Handles application settings and persistent state

The system leverages Rust's strong type system and error handling to ensure reliability and safety in critical operations like file manipulation and system updates.

**Section sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs)

## Upgrade Strategy Pattern

The system implements a sophisticated strategy pattern for upgrade management, allowing flexible selection between full and incremental update approaches based on runtime conditions.

```mermaid
classDiagram
class UpgradeStrategy {
<<enumeration>>
+FullUpgrade
+PatchUpgrade
+NoUpgrade
}
class UpgradeStrategyManager {
-manifest : EnhancedServiceManifest
-current_version : String
-force_full : bool
-architecture : Architecture
+determine_strategy() Result~UpgradeStrategy~
+select_full_upgrade_strategy() Result~UpgradeStrategy~
+select_patch_upgrade_strategy() Result~UpgradeStrategy~
}
class DownloadType {
<<enumeration>>
+Full
+Patch
}
class DecisionFactors {
+version_compatibility : f64
+network_condition : f64
+disk_space : f64
+risk_assessment : f64
+time_efficiency : f64
}
UpgradeStrategyManager --> UpgradeStrategy : creates
UpgradeStrategyManager --> DecisionFactors : uses
UpgradeStrategy --> DownloadType : contains
```

**Diagram sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs)

The `UpgradeStrategyManager` acts as a factory that evaluates multiple factors to determine the appropriate upgrade strategy:

1. **Version Compatibility**: Compares current and target versions to determine if a patch upgrade is possible
2. **Architecture Detection**: Identifies the system architecture (x86_64 or aarch64) to select appropriate packages
3. **System State**: Checks for the presence of required directories and files
4. **User Preferences**: Respects forced full upgrade settings

The strategy selection process follows this decision tree:
- If current version equals or exceeds target version → NoUpgrade
- If forced full upgrade is enabled → FullUpgrade
- If working directory or compose file is missing → FullUpgrade
- If versions have different base numbers → FullUpgrade
- If same base version with available patch → PatchUpgrade

This pattern allows the system to adapt to different deployment scenarios while maintaining a consistent interface for upgrade operations.

**Section sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)

## Observer Pattern for Progress Monitoring

The patch execution system implements the observer pattern through callback functions to provide real-time progress updates during long-running operations.

```mermaid
sequenceDiagram
participant UI as "User Interface"
participant Controller as "Tauri Command"
participant Executor as "PatchExecutor"
participant Processor as "PatchProcessor"
UI->>Controller : initiatePatchApplication()
Controller->>Executor : apply_patch() with progress_callback
Executor->>UI : progress_callback(0.0)
Executor->>Processor : download_patch()
Processor->>UI : progress_callback(incremental updates)
Processor-->>Executor : patch_path
Executor->>Processor : verify_patch_integrity()
Executor->>Processor : extract_patch()
Executor->>Executor : apply_patch_operations()
Executor->>UI : progress_callback(final updates)
Executor-->>Controller : success
Controller-->>UI : updateProgress(100%)
```

**Diagram sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L455)

The `PatchExecutor` struct accepts a progress callback function as a parameter in its `apply_patch` method:

```rust
pub async fn apply_patch<F>(
    &mut self,
    patch_info: &PatchPackageInfo,
    operations: &PatchOperations,
    progress_callback: F,
) -> Result<(), PatchExecutorError>
where
    F: Fn(f64) + Send + Sync,
```

This callback is invoked at key stages of the patch application process:
1. Initialization (0%)
2. After download completion (25%)
3. After integrity verification (35%)
4. After extraction (45%)
5. During operation application (50-100% based on completion)

The file operations component also implements progress tracking through individual operation callbacks:

```mermaid
flowchart TD
Start["Patch Application Start"] --> Validate["Validate Preconditions"]
Validate --> Download["Download Patch Package"]
Download --> Verify["Verify Patch Integrity"]
Verify --> Extract["Extract Patch Package"]
Extract --> Structure["Validate File Structure"]
Structure --> Apply["Apply Patch Operations"]
subgraph "Operation Progress"
Apply --> ReplaceFiles["Replace Files"]
ReplaceFiles --> ReplaceDirs["Replace Directories"]
ReplaceDirs --> DeleteFiles["Delete Files"]
DeleteFiles --> DeleteDirs["Delete Directories"]
end
Apply --> Complete["Patch Application Complete"]
style Start fill:#2196F3,stroke:#1976D2
style Complete fill:#4CAF50,stroke:#388E3C
style Validate,Verify,Structure fill:#FFC107,stroke:#FFA000
style Download,Extract fill:#03A9F4,stroke:#0288D1
style Apply fill:#9C27B0,stroke:#7B1FA2
```

**Diagram sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)

For rollback operations, the system maintains a backup directory using `TempDir` when backup mode is enabled, allowing for safe recovery if an upgrade fails.

**Section sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L455)

## System Boundaries and Integration Points

The duck_client system integrates with several external systems through well-defined boundaries:

```mermaid
graph TD
A[duck_client] --> B[Docker Engine]
A --> C[Remote Update Server]
A --> D[Local File System]
A --> E[Network Services]
subgraph "Internal Components"
A
F[CLI Interface]
G[GUI Interface]
H[Core Logic]
end
subgraph "External Systems"
B
C
D
E
end
style F,G,H fill:#FF9800,stroke:#F57C00
style B,C,D,E fill:#607D8B,stroke:#455A64
```

**Diagram sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs)

Key integration points include:
- **Docker Engine**: Managed through Docker Compose files and direct API calls for service lifecycle management
- **Remote Servers**: Used for downloading patch packages and retrieving version manifests via HTTPS
- **Local Storage**: Stores configuration files, backup data, and temporary files during operations
- **System Architecture**: Supports both x86_64 and aarch64 platforms with architecture-specific package selection

The system handles platform differences through the `Architecture` enum which automatically detects the current platform and selects appropriate upgrade packages.

## Data Flow Analysis

The data flow for a typical patch application follows a structured pipeline:

```mermaid
flowchart LR
UserInput --> CommandParsing --> StrategySelection --> PatchDownload --> IntegrityVerification --> Extraction --> FileOperations --> Completion
subgraph "Input Phase"
UserInput["User Input (CLI/GUI)"]
CommandParsing["Command Parsing and Validation"]
end
subgraph "Processing Phase"
StrategySelection["Upgrade Strategy Selection"]
PatchDownload["Patch Package Download"]
IntegrityVerification["Hash and Signature Verification"]
Extraction["Tar.gz Extraction"]
FileOperations["File Replacement and Deletion"]
end
subgraph "Output Phase"
Completion["Success/Failure Notification"]
Rollback["Automatic Rollback on Failure"]
end
style UserInput,CommandParsing fill:#2196F3,stroke:#1976D2
style StrategySelection,PatchDownload,IntegrityVerification,Extraction,FileOperations fill:#FF9800,stroke:#F57C00
style Completion,Rollback fill:#4CAF50,stroke:#388E3C
```

**Diagram sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)

The process begins with user input from either the CLI or GUI interface, which is parsed and validated before being passed to the core logic. The `UpgradeStrategyManager` determines the appropriate upgrade approach, then the `PatchExecutor` coordinates the download, verification, and application of patches.

During patch application, the system maintains data integrity through:
- Pre-operation validation
- Atomic file operations using temporary files
- Optional backup creation before modifications
- Automatic rollback on failure detection

## Cross-Cutting Concerns

### Security

The system implements multiple security measures:
- **Signature Verification**: Digital signatures are validated for all downloaded patches
- **Hash Checking**: SHA-256 hashes ensure package integrity
- **Path Safety**: Protection against directory traversal attacks during extraction
- **Atomic Operations**: File replacements use temporary files to prevent corruption

```mermaid
flowchart TD
Start --> Download
Download --> HashCheck
HashCheck --> SignatureCheck
SignatureCheck --> Extract
Extract --> PathValidation
PathValidation --> Apply
Apply --> Complete
HashCheck --> |Fail| Abort
SignatureCheck --> |Fail| Abort
PathValidation --> |Invalid| Abort
style HashCheck,SignatureCheck,PathValidation fill:#F44336,stroke:#D32F2F
style Abort fill:#F44336,stroke:#D32F2F,color:#FFFFFF
```

**Diagram sources**
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L455)

### Monitoring and Health Checks

The system includes comprehensive logging and monitoring capabilities:
- Structured logging with `tracing` crate
- Progress callbacks for UI updates
- Detailed error reporting with context
- Health check integration for Docker services

### Disaster Recovery

Robust disaster recovery features include:
- **Backup/Restore**: Optional backup creation before patch application
- **Automatic Rollback**: Failed operations automatically revert to previous state
- **Idempotent Operations**: Safe to retry failed operations
- **Error Resilience**: Comprehensive error handling with recovery options

The backup system creates a temporary directory that stores copies of files before they are modified, enabling complete restoration if an upgrade fails.

**Section sources**
- [mod.rs](file://client-core/src/patch_executor/mod.rs#L1-L432)
- [file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L1-L524)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L455)