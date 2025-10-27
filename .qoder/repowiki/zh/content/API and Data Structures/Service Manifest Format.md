# Service Manifest Format

<cite>
**Referenced Files in This Document**   
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L0-L431)
- [patch_executor/patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L0-L454)
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L0-L523)
- [api.rs](file://client-core/src/api.rs#L600-L799)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [EnhancedServiceManifest Structure](#enhancedservicemanifest-structure)
3. [Version Metadata and Release Information](#version-metadata-and-release-information)
4. [Package URLs and Architecture Detection](#package-urls-and-architecture-detection)
5. [Cryptographic Signatures and Security](#cryptographic-signatures-and-security)
6. [Patch Deltas and Incremental Updates](#patch-deltas-and-incremental-updates)
7. [Dependency Specifications and Validation](#dependency-specifications-and-validation)
8. [Upgrade Decision Logic](#upgrade-decision-logic)
9. [Schema Versioning and Backward Compatibility](#schema-versioning-and-backward-compatibility)
10. [Manifest Integration with Components](#manifest-integration-with-components)
11. [JSON Schema Definition](#json-schema-definition)
12. [Security Mechanisms](#security-mechanisms)

## Introduction
The EnhancedServiceManifest format is a critical component of the system's update mechanism, enabling intelligent upgrade decisions through support for both full and incremental updates. This document provides a comprehensive analysis of the manifest structure, its role in the upgrade process, and its integration with various system components. The manifest serves as the central configuration that defines available updates, their metadata, cryptographic signatures, and patch operations, while supporting multiple architectures and ensuring backward compatibility with older clients.

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)

## EnhancedServiceManifest Structure
The EnhancedServiceManifest is a JSON-based structure that extends the traditional service manifest format with support for architecture-specific packages and incremental updates. It contains version metadata, release information, package URLs, cryptographic signatures, and patch operations.

```mermaid
classDiagram
class EnhancedServiceManifest {
+Version version
+String release_date
+String release_notes
+Option~ServicePackages~ packages
+Option~PlatformPackages~ platforms
+Option~PatchInfo~ patch
+validate() Result
+supports_architecture(arch) bool
+has_patch_for_architecture(arch) bool
}
class ServicePackages {
+PackageInfo full
+Option~PackageInfo~ patch
+validate() Result
}
class PlatformPackages {
+Option~PlatformPackageInfo~ x86_64
+Option~PlatformPackageInfo~ aarch64
+validate() Result
}
class PatchInfo {
+Option~PatchPackageInfo~ x86_64
+Option~PatchPackageInfo~ aarch64
+validate() Result
}
class PatchPackageInfo {
+String url
+Option~String~ hash
+Option~String~ signature
+PatchOperations operations
+Option~String~ notes
+validate() Result
+get_changed_files() Vec~String~
}
class PatchOperations {
+Option~ReplaceOperations~ replace
+Option~ReplaceOperations~ delete
+validate() Result
+total_operations() usize
}
class ReplaceOperations {
+Vec~String~ files
+Vec~String~ directories
+validate() Result
}
EnhancedServiceManifest --> ServicePackages : "has optional"
EnhancedServiceManifest --> PlatformPackages : "has optional"
EnhancedServiceManifest --> PatchInfo : "has optional"
PatchInfo --> PatchPackageInfo : "contains"
PatchPackageInfo --> PatchOperations : "has"
PatchOperations --> ReplaceOperations : "references"
```

**Diagram sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

## Version Metadata and Release Information
The EnhancedServiceManifest contains comprehensive version metadata that enables intelligent upgrade decisions. The version field uses a semantic versioning scheme with an optional fourth component for incremental updates. The release_date field follows RFC 3339 format to ensure consistent timestamp parsing across clients.

The version field is deserialized using a custom function `version_from_str` that handles both standard semantic versions (e.g., "1.0.2") and extended versions with an incremental component (e.g., "1.0.2.4"). This allows the system to distinguish between major releases and incremental patches within the same base version.

```mermaid
sequenceDiagram
participant Client as "Client"
participant API as "API Client"
participant Manifest as "EnhancedServiceManifest"
Client->>API : check_update()
API->>API : fetch manifest from server
API->>Manifest : parse JSON response
Manifest->>Manifest : deserialize version field
Manifest->>Manifest : validate release_date format
Manifest->>API : return validated manifest
API->>Client : return update information
```

**Diagram sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)
- [api.rs](file://client-core/src/api.rs#L600-L799)

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

## Package URLs and Architecture Detection
The EnhancedServiceManifest supports both universal and architecture-specific package URLs through two distinct fields: `packages` and `platforms`. The `packages` field maintains backward compatibility with older clients by providing a single URL for all architectures, while the `platforms` field enables optimized downloads for specific architectures.

Architecture detection is performed automatically by the `Architecture::detect()` method, which identifies the client's CPU architecture (x86_64 or aarch64). The manifest uses this information to select the appropriate package URL, ensuring optimal performance and compatibility.

```mermaid
graph TD
A[Client Architecture Detection] --> B{Architecture}
B --> |x86_64| C[Select x86_64 package URL]
B --> |aarch64| D[Select aarch64 package URL]
B --> |Unsupported| E[Use universal package URL]
C --> F[Download architecture-specific package]
D --> F
E --> F
F --> G[Verify package integrity]
```

**Diagram sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

**Section sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)

## Cryptographic Signatures and Security
The EnhancedServiceManifest includes cryptographic signatures to ensure package integrity and authenticity. Each package and patch contains a signature field that can be verified against a trusted public key. The system supports both SHA-256 hash verification and digital signature validation.

For full packages, the signature is mandatory and stored in the `signature` field of `PackageInfo` or `PlatformPackageInfo`. For patch packages, the signature is optional but recommended, stored in the `signature` field of `PatchPackageInfo`. The validation process checks both the hash and signature when available, providing defense in depth against tampering.

```mermaid
sequenceDiagram
participant Client as "Client"
participant Downloader as "Downloader"
participant Verifier as "Integrity Verifier"
Client->>Downloader : download package
Downloader->>Downloader : save package and hash
Downloader->>Verifier : verify hash
Verifier->>Verifier : calculate actual hash
Verifier->>Verifier : compare with expected hash
Verifier-->>Downloader : hash verification result
Downloader->>Verifier : verify signature
Verifier->>Verifier : decode signature
Verifier->>Verifier : validate with public key
Verifier-->>Downloader : signature verification result
Downloader-->>Client : download result
```

**Diagram sources**
- [patch_executor/patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L0-L454)
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

**Section sources**
- [patch_executor/patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L0-L454)

## Patch Deltas and Incremental Updates
The EnhancedServiceManifest supports incremental updates through the `patch` field, which contains architecture-specific patch packages. Each patch package includes a URL to download the patch, cryptographic verification data, and a detailed specification of the file operations required to apply the patch.

The patch operations are defined in the `PatchOperations` structure, which specifies files and directories to be replaced or deleted. This enables efficient updates by only downloading and applying the changes between versions rather than the entire package.

```mermaid
classDiagram
class PatchPackageInfo {
+String url
+Option~String~ hash
+Option~String~ signature
+PatchOperations operations
+Option~String~ notes
}
class PatchOperations {
+Option~ReplaceOperations~ replace
+Option~ReplaceOperations~ delete
}
class ReplaceOperations {
+Vec~String~ files
+Vec~String~ directories
}
PatchPackageInfo --> PatchOperations : "has"
PatchOperations --> ReplaceOperations : "contains"
```

**Diagram sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

## Dependency Specifications and Validation
The EnhancedServiceManifest includes comprehensive validation rules to ensure data integrity and prevent malformed manifests from causing system issues. The validation process checks the format of all fields, verifies URL syntax, and ensures that required fields are present.

The manifest validation is performed recursively, with each component validating its own fields before the overall manifest is considered valid. This layered approach ensures that errors are caught early and provides detailed error messages for troubleshooting.

```mermaid
flowchart TD
Start([Validate Manifest]) --> ValidateDate["Validate release_date format"]
ValidateDate --> ValidatePackages["Validate packages field"]
ValidatePackages --> ValidatePlatforms["Validate platforms field"]
ValidatePlatforms --> ValidatePatch["Validate patch field"]
ValidatePatch --> CheckArchitecture["Check architecture support"]
CheckArchitecture --> CheckPatchAvailability["Check patch availability"]
CheckArchitecture --> End([Manifest Valid])
CheckPatchAvailability --> End
ValidateDate --> |Invalid| Error["Return validation error"]
ValidatePackages --> |Invalid| Error
ValidatePlatforms --> |Invalid| Error
ValidatePatch --> |Invalid| Error
```

**Diagram sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

## Upgrade Decision Logic
The upgrade decision logic is implemented in the `UpgradeStrategyManager` class, which analyzes the EnhancedServiceManifest and client state to determine the optimal upgrade strategy. The decision process considers version compatibility, architecture, and system state to choose between full upgrades, incremental updates, or no upgrade.

The decision algorithm first compares the current version with the target version to determine if an upgrade is needed. If the versions have the same base version, an incremental update is preferred. Otherwise, a full upgrade is required. The system also checks for the presence of necessary files and directories before proceeding with any upgrade.

```mermaid
graph TD
A[Start Upgrade Decision] --> B{Force Full Upgrade?}
B --> |Yes| C[Select Full Upgrade]
B --> |No| D{Docker Directory Exists?}
D --> |No| C
D --> |Yes| E{Versions Compatible?}
E --> |Equal or Newer| F[No Upgrade Needed]
E --> |Patch Upgradeable| G{Patch Available?}
G --> |Yes| H[Select Patch Upgrade]
G --> |No| C
E --> |Full Upgrade Required| C
C --> I[Execute Full Upgrade]
H --> J[Execute Patch Upgrade]
F --> K[Exit]
```

**Diagram sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)

**Section sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)

## Schema Versioning and Backward Compatibility
The EnhancedServiceManifest maintains backward compatibility through a dual-format approach. When a manifest contains the `platforms` field, it is parsed as an enhanced format. Otherwise, it is treated as a legacy format and automatically converted to the enhanced format.

This approach allows older servers to continue using the traditional format while newer clients can take advantage of the enhanced features. The conversion process preserves all relevant information and sets appropriate default values for new fields.

```mermaid
sequenceDiagram
participant Client as "Client"
participant API as "API Client"
participant Parser as "Manifest Parser"
Client->>API : request manifest
API->>Parser : receive JSON response
Parser->>Parser : check for platforms field
alt Has platforms field
Parser->>Parser : parse as EnhancedServiceManifest
else No platforms field
Parser->>Parser : parse as ServiceManifest
Parser->>Parser : convert to EnhancedServiceManifest
end
Parser-->>API : return EnhancedServiceManifest
API-->>Client : return manifest
```

**Diagram sources**
- [api.rs](file://client-core/src/api.rs#L600-L799)
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

**Section sources**
- [api.rs](file://client-core/src/api.rs#L600-L799)

## Manifest Integration with Components
The EnhancedServiceManifest integrates with multiple system components, including the download manager, patch executor, and API client. The download manager uses the manifest to determine the appropriate package URL and verification data, while the patch executor uses the patch operations to apply incremental updates.

The integration is designed to be modular, with each component responsible for a specific aspect of the update process. This separation of concerns ensures that changes to one component do not affect the others, making the system more maintainable and easier to test.

```mermaid
graph TB
subgraph "Update System"
Manifest[EnhancedServiceManifest]
Strategy[UpgradeStrategyManager]
Downloader[FileDownloader]
Executor[PatchExecutor]
end
Manifest --> Strategy : "Provides update data"
Strategy --> Downloader : "Provides download URL"
Strategy --> Executor : "Provides patch operations"
Downloader --> Executor : "Downloads patch package"
Executor --> Executor : "Applies patch operations"
```

**Diagram sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L0-L431)

**Section sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L0-L462)
- [patch_executor/mod.rs](file://client-core/src/patch_executor/mod.rs#L0-L431)

## JSON Schema Definition
The EnhancedServiceManifest follows a well-defined JSON schema that ensures consistency and validity across all instances. The schema includes validation rules for all fields and supports both required and optional properties.

```json
{
  "type": "object",
  "properties": {
    "version": {
      "type": "string",
      "pattern": "^(\\d+)\\.(\\d+)\\.(\\d+)(\\.\\d+)?$"
    },
    "release_date": {
      "type": "string",
      "format": "date-time"
    },
    "release_notes": {
      "type": "string"
    },
    "packages": {
      "type": "object",
      "properties": {
        "full": {
          "type": "object",
          "properties": {
            "url": {
              "type": "string",
              "format": "uri"
            },
            "hash": {
              "type": "string"
            },
            "signature": {
              "type": "string"
            },
            "size": {
              "type": "integer"
            }
          },
          "required": ["url", "hash", "signature", "size"]
        },
        "patch": {
          "type": "object",
          "properties": {
            "url": {
              "type": "string",
              "format": "uri"
            },
            "hash": {
              "type": "string"
            },
            "signature": {
              "type": "string"
            },
            "size": {
              "type": "integer"
            }
          }
        }
      },
      "required": ["full"]
    },
    "platforms": {
      "type": "object",
      "properties": {
        "x86_64": {
          "type": "object",
          "properties": {
            "url": {
              "type": "string",
              "format": "uri"
            },
            "signature": {
              "type": "string"
            }
          }
        },
        "aarch64": {
          "type": "object",
          "properties": {
            "url": {
              "type": "string",
              "format": "uri"
            },
            "signature": {
              "type": "string"
            }
          }
        }
      }
    },
    "patch": {
      "type": "object",
      "properties": {
        "x86_64": {
          "type": "object",
          "properties": {
            "url": {
              "type": "string",
              "format": "uri"
            },
            "hash": {
              "type": "string"
            },
            "signature": {
              "type": "string"
            },
            "operations": {
              "type": "object",
              "properties": {
                "replace": {
                  "type": "object",
                  "properties": {
                    "files": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    },
                    "directories": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    }
                  }
                },
                "delete": {
                  "type": "object",
                  "properties": {
                    "files": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    },
                    "directories": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    }
                  }
                }
              }
            },
            "notes": {
              "type": "string"
            }
          }
        },
        "aarch64": {
          "type": "object",
          "properties": {
            "url": {
              "type": "string",
              "format": "uri"
            },
            "hash": {
              "type": "string"
            },
            "signature": {
              "type": "string"
            },
            "operations": {
              "type": "object",
              "properties": {
                "replace": {
                  "type": "object",
                  "properties": {
                    "files": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    },
                    "directories": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    }
                  }
                },
                "delete": {
                  "type": "object",
                  "properties": {
                    "files": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    },
                    "directories": {
                      "type": "array",
                      "items": {
                        "type": "string"
                      }
                    }
                  }
                }
              }
            },
            "notes": {
              "type": "string"
            }
          }
        }
      }
    }
  },
  "required": ["version", "release_date", "release_notes"]
}
```

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L300-L600)

## Security Mechanisms
The EnhancedServiceManifest incorporates multiple security mechanisms to prevent tampering and ensure the integrity of updates. These include cryptographic signatures, hash verification, and path validation to prevent directory traversal attacks.

The system verifies both the hash and signature of downloaded packages, providing defense in depth against various attack vectors. The patch processor also validates the structure of extracted files and checks for dangerous paths that could lead to security vulnerabilities.

```mermaid
sequenceDiagram
participant Client as "Client"
participant Downloader as "Downloader"
participant Verifier as "Integrity Verifier"
participant Executor as "Patch Executor"
Client->>Downloader : initiate download
Downloader->>Verifier : verify hash
Verifier-->>Downloader : hash verification result
Downloader->>Verifier : verify signature
Verifier-->>Downloader : signature verification result
Downloader->>Executor : extract patch
Executor->>Executor : validate file paths
Executor->>Executor : check for directory traversal
Executor-->>Client : ready to apply patch
```

**Diagram sources**
- [patch_executor/patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L0-L454)
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L0-L523)

**Section sources**
- [patch_executor/patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L0-L454)
- [patch_executor/file_operations.rs](file://client-core/src/patch_executor/file_operations.rs#L0-L523)