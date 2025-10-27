# Check Update Command

<cite>
**Referenced Files in This Document**   
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs)
- [architecture.rs](file://client-core/src/architecture.rs)
- [api_types.rs](file://client-core/src/api_types.rs)
- [version.rs](file://client-core/src/version.rs)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs)
- [authenticated_client.rs](file://client-core/src/authenticated_client.rs)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Update Checking Workflow](#update-checking-workflow)
3. [EnhancedServiceManifest Structure](#enhancedservicemanifest-structure)
4. [Version Comparison Logic](#version-comparison-logic)
5. [Architecture-Specific Package Selection](#architecture-specific-package-selection)
6. [Network Request Handling](#network-request-handling)
7. [Signature Verification Process](#signature-verification-process)
8. [Integration with Upgrade Strategy](#integration-with-upgrade-strategy)
9. [Edge Case Handling](#edge-case-handling)
10. [Troubleshooting Guide](#troubleshooting-guide)

## Introduction
The `check-update` command is responsible for querying remote servers to determine if service updates are available. It retrieves and parses the EnhancedServiceManifest, compares the current version using client-core::version, and determines update availability based on version numbers, architecture compatibility, and patch availability. The command integrates with the upgrade strategy decision engine and provides status reporting to the UI.

**Section sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L1-L200)

## Update Checking Workflow
The update checking process follows a structured sequence of operations to determine if an update is available:

```mermaid
sequenceDiagram
participant CLI as "check-update Command"
participant Client as "AuthenticatedClient"
participant Manifest as "EnhancedServiceManifest"
participant Version as "VersionComparator"
participant Strategy as "UpgradeStrategyEngine"
participant UI as "UI Status Reporter"
CLI->>Client : initiate_request("/api/v1/service-manifest")
Client->>Client : add_authentication_headers()
Client->>Manifest : send_HTTP_request()
Manifest-->>Client : return_manifest_response()
Client-->>CLI : parse_JSON_response()
CLI->>Version : compare_current_version()
Version-->>CLI : return_update_status()
CLI->>Manifest : check_architecture_compatibility()
Manifest-->>CLI : return_platform_support()
CLI->>Strategy : determine_upgrade_strategy()
Strategy-->>CLI : return_strategy_decision()
CLI->>UI : report_status()
UI-->>User : display_update_information()
```

**Diagram sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L15-L150)
- [authenticated_client.rs](file://client-core/src/authenticated_client.rs#L50-L120)

**Section sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L1-L150)

## EnhancedServiceManifest Structure
The EnhancedServiceManifest contains comprehensive information about available service updates, including version details, package URLs, and platform-specific information.

```mermaid
classDiagram
class EnhancedServiceManifest {
+version : String
+release_date : String
+release_notes : String
+packages : Option~ServicePackages~
+platforms : Option~PlatformPackages~
+patch : Option~PatchInfo~
+validate() Result~()~
+supports_architecture(arch) bool
+has_patch_for_architecture(arch) bool
}
class ServicePackages {
+full : PackageInfo
+patch : Option~PatchInfo~
+validate() Result~()~
}
class PlatformPackages {
+x86_64 : Option~PlatformPackageInfo~
+aarch64 : Option~PlatformPackageInfo~
+validate() Result~()~
}
class PatchInfo {
+version : String
+x86_64 : Option~PatchPackageInfo~
+aarch64 : Option~PatchPackageInfo~
+validate() Result~()~
}
class PackageInfo {
+url : String
+hash : Option~String~
+signature : Option~String~
+size : u64
+validate() Result~()~
}
class PlatformPackageInfo {
+url : String
+signature : String
+validate() Result~()~
}
class PatchPackageInfo {
+url : String
+hash : String
+signature : String
+operations : PatchOperations
+validate() Result~()~
}
class PatchOperations {
+replace : Option~ReplaceOperations~
+delete : Option~DeleteOperations~
+validate() Result~()~
+total_operations() usize
}
class ReplaceOperations {
+files : Vec~String~
+directories : Vec~String~
+validate() Result~()~
}
EnhancedServiceManifest --> ServicePackages : "has"
EnhancedServiceManifest --> PlatformPackages : "has"
EnhancedServiceManifest --> PatchInfo : "has"
ServicePackages --> PackageInfo : "contains"
ServicePackages --> PatchInfo : "contains"
PlatformPackages --> PlatformPackageInfo : "contains"
PatchInfo --> PatchPackageInfo : "contains"
PatchPackageInfo --> PatchOperations : "contains"
PatchOperations --> ReplaceOperations : "contains"
```

**Diagram sources**
- [api_types.rs](file://client-core/src/api_types.rs#L100-L300)

**Section sources**
- [api_types.rs](file://client-core/src/api_types.rs#L100-L300)

## Version Comparison Logic
The version comparison system uses semantic versioning principles to determine if a newer version is available. The comparison considers major, minor, patch, and build numbers.

```mermaid
flowchart TD
Start([Start Version Comparison]) --> ParseCurrent["Parse Current Version"]
ParseCurrent --> ParseLatest["Parse Latest Manifest Version"]
ParseLatest --> ExtractCurrent["Extract Version Components<br>current_major, current_minor,<br>current_patch, current_build"]
ExtractCurrent --> ExtractLatest["Extract Version Components<br>latest_major, latest_minor,<br>latest_patch, latest_build"]
ExtractLatest --> CompareMajor{"latest_major > current_major?"}
CompareMajor --> |Yes| UpdateAvailable["Return: Update Available"]
CompareMajor --> |No| CompareMinor{"latest_minor > current_minor?"}
CompareMinor --> |Yes| UpdateAvailable
CompareMinor --> |No| ComparePatch{"latest_patch > current_patch?"}
ComparePatch --> |Yes| UpdateAvailable
ComparePatch --> |No| CompareBuild{"latest_build > current_build?"}
CompareBuild --> |Yes| UpdateAvailable
CompareBuild --> |No| NoUpdate["Return: No Update Available"]
UpdateAvailable --> End([End])
NoUpdate --> End
```

**Diagram sources**
- [version.rs](file://client-core/src/version.rs#L20-L100)

**Section sources**
- [version.rs](file://client-core/src/version.rs#L1-L150)

## Architecture-Specific Package Selection
The system automatically detects the current hardware architecture and selects the appropriate package from the manifest based on platform compatibility.

```mermaid
flowchart TD
Start([Start Architecture Selection]) --> DetectArch["Detect Current Architecture<br>Architecture::detect()"]
DetectArch --> CheckSupported{"Architecture Supported?"}
CheckSupported --> |No| Unsupported["Return: Unsupported Architecture"]
CheckSupported --> |Yes| CheckManifest["Check Manifest for Platform Packages"]
CheckManifest --> HasPlatforms{"Manifest has platforms field?"}
HasPlatforms --> |No| UseLegacy["Use Legacy Package URL"]
HasPlatforms --> |Yes| CheckArchSpecific{"Manifest has architecture-specific package?"}
CheckArchSpecific --> |No| UseFallback["Use Fallback Package"]
CheckArchSpecific --> |Yes| SelectPackage["Select Architecture-Specific Package"]
SelectPackage --> ReturnURL["Return Package URL for Current Architecture"]
UseLegacy --> ReturnURL
UseFallback --> ReturnURL
Unsupported --> End([End])
ReturnURL --> End
```

**Diagram sources**
- [architecture.rs](file://client-core/src/architecture.rs#L50-L200)
- [api_types.rs](file://client-core/src/api_types.rs#L250-L300)

**Section sources**
- [architecture.rs](file://client-core/src/architecture.rs#L1-L250)
- [api_types.rs](file://client-core/src/api_types.rs#L200-L350)

## Network Request Handling
The network request system implements timeout policies and retry mechanisms to handle unreliable network conditions and server responsiveness.

```mermaid
sequenceDiagram
participant Command as "check-update"
participant Client as "AuthenticatedClient"
participant Server as "Remote Server"
participant Retry as "Retry Mechanism"
Command->>Client : execute_request()
Client->>Client : set_timeout(30s)
Client->>Server : send_request()
alt Server Responsive
Server-->>Client : return_200_OK()
Client-->>Command : process_response()
else Server Unresponsive
Server--x Client : timeout_after_30s
Client->>Retry : trigger_retry_mechanism()
Retry->>Client : wait_exponential_backoff()
Client->>Server : retry_request()
Server-->>Client : return_200_OK()
Client-->>Command : process_response()
end
alt Authentication Required
Server-->>Client : return_401_Unauthorized()
Client->>Client : auto_register_client()
Client->>Server : retry_with_new_credentials()
Server-->>Client : return_200_OK()
Client-->>Command : process_response()
end
```

**Diagram sources**
- [authenticated_client.rs](file://client-core/src/authenticated_client.rs#L100-L200)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L50-L100)

**Section sources**
- [authenticated_client.rs](file://client-core/src/authenticated_client.rs#L50-L250)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L150)

## Signature Verification Process
The signature verification process ensures the authenticity and integrity of downloaded packages by validating digital signatures and hash values.

```mermaid
flowchart TD
Start([Start Verification]) --> CheckSignature{"Signature Present?"}
CheckSignature --> |No| SkipSignature["Skip Signature Verification"]
CheckSignature --> |Yes| DecodeBase64["Decode Base64 Signature"]
DecodeBase64 --> ValidBase64{"Valid Base64 Format?"}
ValidBase64 --> |No| FailSignature["Fail: Invalid Signature Format"]
ValidBase64 --> |Yes| VerifySignature["Verify Digital Signature<br>(Placeholder Implementation)"]
VerifySignature --> SignatureOK["Signature Verified"]
SignatureOK --> CheckHash{"Hash Present?"}
SkipSignature --> CheckHash
CheckHash --> |No| SkipHash["Skip Hash Verification"]
CheckHash --> |Yes| CalculateSHA256["Calculate File SHA256 Hash"]
CalculateSHA256 --> CompareHash{"Hash Matches Expected?"}
CompareHash --> |No| FailHash["Fail: Hash Mismatch"]
CompareHash --> |Yes| Success["Verification Successful"]
SkipHash --> Success
Success --> End([End])
FailSignature --> End
FailHash --> End
```

**Diagram sources**
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L130-L180)

**Section sources**
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L100-L200)

## Integration with Upgrade Strategy
The check-update command integrates with the upgrade strategy decision engine to determine the most appropriate update method based on various factors.

```mermaid
flowchart TD
Start([check-update Command]) --> RetrieveManifest["Retrieve EnhancedServiceManifest"]
RetrieveManifest --> CompareVersion["Compare Current vs Latest Version"]
CompareVersion --> VersionAvailable{"Update Available?"}
VersionAvailable --> |No| NoAction["No Action Required"]
VersionAvailable --> |Yes| CheckArchitecture["Check Architecture Compatibility"]
CheckArchitecture --> ArchitectureOK{"Compatible?"}
ArchitectureOK --> |No| Incompatible["Architecture Incompatible"]
ArchitectureOK --> |Yes| CheckPatch["Check for Patch Availability"]
CheckPatch --> HasPatch{"Patch Available?"}
HasPatch --> |Yes| StrategyEngine["Upgrade Strategy Engine"]
HasPatch --> |No| FullUpgrade["Full Package Upgrade"]
StrategyEngine --> AnalyzeConditions["Analyze Network, Storage, and<br>System Conditions"]
AnalyzeConditions --> DetermineStrategy{"Determine Optimal Strategy"}
DetermineStrategy --> PatchUpgrade["Patch-Based Upgrade"]
DetermineStrategy --> FullUpgrade
DetermineStrategy --> Deferred["Defer Upgrade"]
PatchUpgrade --> Report["Report Strategy to UI"]
FullUpgrade --> Report
Deferred --> Report
NoAction --> Report
Incompatible --> Report
Report --> End([End])
```

**Diagram sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L80-L150)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L10-L50)

**Section sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L50-L200)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L100)

## Edge Case Handling
The system includes comprehensive error handling for various edge cases that may occur during the update checking process.

```mermaid
flowchart TD
Start([Update Check Initiated]) --> NetworkError{"Network Request Failed?"}
NetworkError --> |Yes| HandleNetwork["Handle Network Error"]
HandleNetwork --> Retry{"Within Retry Limit?"}
Retry --> |Yes| ExponentialBackoff["Wait with Exponential Backoff"]
ExponentialBackoff --> RetryRequest["Retry Request"]
RetryRequest --> Start
Retry --> |No| ReportUnreachable["Report Server Unreachable"]
ReportUnreachable --> End
NetworkError --> |No| ParseError{"Manifest Parse Failed?"}
ParseError --> |Yes| HandleParse["Handle Malformed Manifest"]
HandleParse --> ReportMalformed["Report Malformed Manifest"]
ReportMalformed --> End
ParseError --> |No| ValidateError{"Manifest Validation Failed?"}
ValidateError --> |Yes| HandleInvalid["Handle Invalid Manifest"]
HandleInvalid --> ReportInvalid["Report Invalid Manifest"]
ReportInvalid --> End
ValidateError --> |No| SignatureError{"Signature Verification Failed?"}
SignatureError --> |Yes| HandleSignature["Handle Signature Failure"]
HandleSignature --> ReportSignature["Report Signature Verification Failed"]
ReportSignature --> End
SignatureError --> |No| Success["Update Check Successful"]
Success --> End([End])
```

**Diagram sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L150-L200)
- [api_types.rs](file://client-core/src/api_types.rs#L300-L400)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L130-L180)

**Section sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L150-L200)
- [api_types.rs](file://client-core/src/api_types.rs#L300-L450)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L100-L200)

## Troubleshooting Guide
This section provides diagnostic guidance and mitigation strategies for common issues encountered during the update checking process.

### Unreachable Servers
**Symptoms**: Network timeout errors, connection refused messages
**Diagnosis**: 
- Check network connectivity
- Verify server URL and port
- Test firewall settings
- Check DNS resolution

**Mitigation Strategies**:
- Implement retry mechanism with exponential backoff
- Provide alternative mirror servers
- Cache previous manifest for offline use
- Display clear error messages to users

### Malformed Manifests
**Symptoms**: JSON parsing errors, invalid field formats
**Diagnosis**:
- Validate manifest structure against schema
- Check for missing required fields
- Verify date format compliance (RFC3339)
- Ensure URL format validity

**Mitigation Strategies**:
- Implement comprehensive validation with detailed error messages
- Provide fallback to legacy manifest format
- Log validation errors for debugging
- Notify server administrators of schema violations

### Signature Verification Failures
**Symptoms**: Signature format errors, hash mismatches
**Diagnosis**:
- Verify base64 encoding of signatures
- Check SHA256 hash calculation
- Validate certificate chain (future implementation)
- Confirm public key authenticity

**Mitigation Strategies**:
- Implement graceful degradation (warn instead of fail)
- Provide signature bypass option for development environments
- Cache verified packages to avoid re-downloading
- Display detailed verification failure reasons

### Architecture Compatibility Issues
**Symptoms**: No available packages for current architecture
**Diagnosis**:
- Verify architecture detection accuracy
- Check manifest platform availability
- Confirm binary compatibility

**Mitigation Strategies**:
- Implement emulation support (future)
- Provide cross-compilation options
- Offer architecture conversion tools
- Display clear compatibility requirements

**Section sources**
- [check_update.rs](file://nuwax-cli/src/commands/check_update.rs#L150-L200)
- [architecture.rs](file://client-core/src/architecture.rs#L200-L250)
- [api_types.rs](file://client-core/src/api_types.rs#L350-L450)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L150-L200)