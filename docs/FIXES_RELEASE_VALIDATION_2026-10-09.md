# Native validation and release fixes — 2026-10-09

The CLI fixes were validated on native Ubuntu 26.04 x86_64 and Windows 11 x64/MSVC.
The tested source was `fa33644636cff4b8c6321902e40f2ef1aa7ce73d` plus the 18-file
repair overlay with SHA256 `8ee7c95fed910c06ae963875d3b501d047e23736637010f05a084a1adb5c003d`.
Version 1.0.137 includes those repairs; native binary/package release validation
is a separate step and must be verified after npm publication.

## Repairs

- Preserve valid escaped single quotes, unquoted apostrophes, hyphenated/colon
  assignments and managed-looking lines inside non-managed multiline values.
- Parse Compose environment files with source-order references, native defaults,
  escapes and unmanaged host precedence; managed device values remain file-owned.
- Interpolate YAML mapping values without changing keys; retain native nested
  defaults and literal-brace behavior.
- Retain native path roots, drive/UNC prefixes and relative parents; keep parent
  traversal within a foreign absolute drive, and parse short mount drive colons.
- Fix Windows-only lint and closed-file atomic replacement fixtures.
- Remove test assumptions about ARM-only URLs and filesystem directory order.
- Keep supplied root credentials, decode URL credentials once, and use explicit
  SQLx options instead of reconstructing credential URLs.
- Make the legacy connection test explicitly ignored without an opt-in disposable
  TEST_MYSQL_URL; connection failures fail instead of silently passing. The test
  creates and removes a unique table in the configured database.

## Actual native results

| Gate | Linux | Windows/MSVC |
|---|---|---|
| Formatting, strict Clippy, check, release build | Passed | Passed |
| Full locked nextest workspace run | 345 passed, 2 ignored | 337 passed, 2 ignored |
| Ignored partial-DDL recovery | Separately executed and passed | Separately executed and passed |
| Ignored opt-in connection/DDL probe | Separately executed and passed | Separately executed and passed |
| Final native CLI device/env assertions | 22 passed | 18 passed |
| Same-account permission drift | ID/time retained | ID/time retained |

Both database tests used newly created disposable MySQL instances and
TEST_MYSQL_REQUIRED=1. Ignored, filtered or silently skipped tests are not counted
as successful execution. Actual native CLI testing covered JSON, frozen v1
compatibility, repeated processes, apply idempotence, BOM/CRLF/multiline source
preservation, duplicate/stale managed keys, symlinks, overwrite failure protection
and temporary-file cleanup. The Linux final library/container probe validated
all 15 device variables, literal credentials, custom paths, a warm cache and
managed/unmanaged process-environment precedence.

Temporary containers, networks, volumes, namespaces, topics and groups were
removed. Existing deployment files and data were preserved.

## Evidence limits and release changes

Windows Docker and real WMIC fallback were unavailable. Native ARM64, physical
SATA, whole-machine reboot and MQ HA/controller failover were not validated;
QEMU, synthetic sysfs and process restart do not replace those cases.

The release workflow adds the already-tested path and mount groups to the
cross-platform gate, marks beta GitHub Releases as prereleases, and retains
workflow assets when OSS/npm publication fails. A successful workflow launch
alone does not establish npm publication or installation acceptance.
