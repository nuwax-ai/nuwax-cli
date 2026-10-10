//! User-facing errors retain their causes without exposing deployment secrets.
//!
//! This boundary uses Display, rather than Debug, and does not print configuration
//! documents. Known operator/runtime secrets are removed even when a dependency
//! includes their values in otherwise unlabelled SQL or command diagnostics.

use anyhow::Error;
use regex::{Captures, Regex};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

const REDACTED: &str = "[REDACTED]";
const MAX_ENV_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CAUSE_CHARACTERS: usize = 2048;
const MAX_DISPLAYED_CAUSES: usize = 16;

/// Format a causal chain using credential patterns and sensitive runtime values.
pub fn format_error(error: &Error) -> String {
    format_error_with_secrets(error, runtime_secrets())
}

/// Also redact values from the selected operator environment file. Reading the
/// file is best effort, bounded, and never changes the process environment.
pub fn format_error_with_env(error: &Error, env_path: &Path) -> String {
    let mut secrets = runtime_secrets();
    if let Some(source) = read_env_source(env_path) {
        // Use the same Compose interpolation/quoting rules as runtime preflight.
        // A malformed record must not replace the original useful error, and
        // parser errors (which could contain values) are never logged here.
        if let Ok(values) = client_core::container::preflight::compose_env_values_from_text(&source)
        {
            for (key, value) in values {
                if sensitive_key(&key) {
                    add_secret(&mut secrets, &value);
                }
            }
        }
        // Raw assignments also cover literal values when Compose interpolation
        // cannot resolve a later malformed record.
        if let Ok(patterns) = RedactionPatterns::new() {
            patterns.collect_secrets(&source, &mut secrets);
        }
    }
    format_error_with_secrets(error, secrets)
}

fn read_env_source(path: &Path) -> Option<String> {
    // Reject directories/devices/FIFOs before opening: an error-report boundary
    // must not block on an invalid operator path.
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ENV_BYTES {
        return None;
    }
    let mut source = String::new();
    file.take(MAX_ENV_BYTES + 1)
        .read_to_string(&mut source)
        .ok()?;
    (source.len() as u64 <= MAX_ENV_BYTES).then_some(source)
}

fn runtime_secrets() -> HashSet<String> {
    let mut secrets = HashSet::new();
    for (key, value) in std::env::vars_os() {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            continue;
        };
        // PWD/OLDPWD are shell directories, not password environment keys.
        if runtime_sensitive_key(key) {
            add_secret(&mut secrets, value);
        }
    }
    secrets
}

fn runtime_sensitive_key(key: &str) -> bool {
    !matches!(key, "PWD" | "OLDPWD") && sensitive_key(key)
}

fn sensitive_key(key: &str) -> bool {
    let key = percent_decode(key)
        .to_ascii_lowercase()
        .replace(['_', '-', '.'], "");
    key == "auth"
        || key.ends_with("pass")
        || [
            "password",
            "passphrase",
            "passwd",
            "pwd",
            "secret",
            "token",
            "apikey",
            "accesskey",
            "privatekey",
            "credential",
            "authorization",
            "signature",
            "aeskey",
            "encryptionkey",
            "cryptokey",
            "signingkey",
            "jwtkey",
            "rsakey",
        ]
        .iter()
        .any(|part| key.contains(part))
}

struct RedactionPatterns {
    assignment: Regex,
    flag: Regex,
    userinfo: Regex,
    sql_password: Regex,
    authorization: Regex,
    private_key: Regex,
    short_password: Regex,
    environment_dump: Regex,
    query_parameter: Regex,
}

impl RedactionPatterns {
    fn new() -> Result<Self, regex::Error> {
        Ok(Self {
            assignment: Regex::new(
                r#"(?is)(?P<prefix>["']?(?P<key>[a-z_%][a-z0-9_.%\-]*)["']?[ \t]*(?:=|:)[ \t]*)(?P<value>"(?:\\.|[^"\\])*"|'(?:''|\\.|[^'\\])*'|[^\s,;}\]\)&]+)"#,
            )?,
            flag: Regex::new(
                r#"(?is)(?P<prefix>--(?P<key>[a-z][a-z0-9_\-]*)[ \t]+)(?P<value>"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;]+)"#,
            )?,
            userinfo: Regex::new(r"(?i)(?P<prefix>\b[a-z][a-z0-9+.-]*://)(?P<value>[^\s/?#]+)@")?,
            sql_password: Regex::new(
                r#"(?is)(?P<prefix>\bIDENTIFIED[ \t]+(?:WITH[ \t]+[a-z0-9_]+[ \t]+)?BY[ \t]+)(?P<value>'(?:''|\\.|[^'\\])*'|"(?:""|\\.|[^"\\])*"|[^\s,;]+)"#,
            )?,
            authorization: Regex::new(
                r#"(?im)(?P<prefix>\b(?:proxy-)?authorization[ \t]*:[ \t]*)(?P<value>[^\r\n]+)"#,
            )?,
            private_key: Regex::new(
                r"(?s)-----BEGIN (?:[A-Z ]+ )?PRIVATE KEY-----.*?-----END (?:[A-Z ]+ )?PRIVATE KEY-----",
            )?,
            short_password: Regex::new(
                r#"(?s)(?P<prefix>(?:^|[ \t])-p[ \t]*)(?P<value>"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;]+)"#,
            )?,
            environment_dump: Regex::new(
                r"(?is)(?P<prefix>\b(?:environment|env)(?:[ \t]+(?:variables|values))?[ \t]*(?:=|:)[ \t]*)(?:\{.*\}|\[.*\])",
            )?,
            // This runs separately from assignments: a preceding `https:`
            // assignment-like match must not consume its first query parameter.
            query_parameter: Regex::new(
                r#"(?i)(?P<prefix>[?&](?P<key>[a-z_%][a-z0-9_.%\-]*)=)(?P<value>[^&#\s"'<>}\]\)]+)"#,
            )?,
        })
    }

    fn collect_secrets(&self, source: &str, secrets: &mut HashSet<String>) {
        for pattern in [&self.assignment, &self.flag, &self.query_parameter] {
            for captures in pattern.captures_iter(source) {
                if captures
                    .name("key")
                    .is_some_and(|key| sensitive_key(key.as_str()))
                    && let Some(value) = captures.name("value")
                {
                    add_secret(secrets, value.as_str());
                }
            }
        }
        for pattern in [&self.sql_password, &self.authorization] {
            for captures in pattern.captures_iter(source) {
                if let Some(value) = captures.name("value") {
                    add_secret(secrets, value.as_str());
                    if let Some((_, credential)) = value.as_str().split_once(' ') {
                        add_secret(secrets, credential.trim());
                    }
                }
            }
        }
        for captures in self.short_password.captures_iter(source) {
            if let Some(value) = captures.name("value")
                && !docker_port_mapping(value.as_str())
            {
                add_secret(secrets, value.as_str());
            }
        }
        for captures in self.userinfo.captures_iter(source) {
            if let Some(value) = captures.name("value") {
                add_secret(secrets, value.as_str());
                if let Some((_, password)) = value.as_str().split_once(':') {
                    add_secret(secrets, password);
                }
            }
        }
        for block in self.private_key.find_iter(source) {
            add_secret(secrets, block.as_str());
        }
    }

    fn redact(&self, source: &str) -> String {
        // Hide entire structured environment dumps, including values whose
        // operator-defined key names do not identify them as credentials.
        let source = self
            .environment_dump
            .replace_all(source, |captures: &Captures<'_>| {
                format!("{}{REDACTED}", &captures["prefix"])
            });
        let source = self.private_key.replace_all(&source, REDACTED);
        let source = self
            .authorization
            .replace_all(&source, |captures: &Captures<'_>| {
                format!("{}{REDACTED}", &captures["prefix"])
            });
        let source = self
            .userinfo
            .replace_all(&source, |captures: &Captures<'_>| {
                format!("{}{REDACTED}@", &captures["prefix"])
            });
        let source = self
            .sql_password
            .replace_all(&source, |captures: &Captures<'_>| {
                format!("{}{REDACTED}", &captures["prefix"])
            });
        let source = self.query_parameter.replace_all(&source, redact_assignment);
        let source = self.assignment.replace_all(&source, redact_assignment);
        let source = self
            .flag
            .replace_all(&source, redact_assignment)
            .into_owned();
        self.short_password
            .replace_all(&source, |captures: &Captures<'_>| {
                if docker_port_mapping(&captures["value"]) {
                    captures[0].to_string()
                } else {
                    format!("{}{REDACTED}", &captures["prefix"])
                }
            })
            .into_owned()
    }
}

fn docker_port_mapping(value: &str) -> bool {
    let value = value
        .strip_suffix("/tcp")
        .or_else(|| value.strip_suffix("/udp"))
        .unwrap_or(value);
    value.contains(':')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b':' | b'.' | b'-'))
}

fn redact_assignment(captures: &Captures<'_>) -> String {
    if captures
        .name("key")
        .is_some_and(|key| sensitive_key(key.as_str()))
    {
        format!("{}{REDACTED}", &captures["prefix"])
    } else {
        captures[0].to_string()
    }
}

fn add_secret(secrets: &mut HashSet<String>, raw: &str) {
    if raw.is_empty() || raw == REDACTED {
        return;
    }
    let unquoted = raw
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .or_else(|| {
            raw.strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or(raw);
    let mut variants = vec![
        raw.to_string(),
        unquoted.to_string(),
        percent_decode(unquoted),
    ];
    if let Ok(decoded) = serde_json::from_str::<String>(raw) {
        variants.push(decoded);
    }
    for value in variants {
        if value.is_empty() || value == REDACTED {
            continue;
        }
        secrets.insert(value.clone());
        secrets.insert(percent_encode(&value, true));
        secrets.insert(percent_encode(&value, false));
        secrets.insert(value.replace(' ', "+"));
        secrets.insert(value.replace("''", "'").replace("\\'", "'"));
        secrets.insert(value.replace('\'', "''"));
        secrets.insert(value.replace('\\', "\\\\").replace('\'', "\\'"));
        if let Ok(escaped) = serde_json::to_string(&value)
            && let Some(inner) = escaped
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
        {
            secrets.insert(inner.to_string());
        }
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'%'
            && let (Some(high), Some(low)) = (bytes.get(offset + 1), bytes.get(offset + 2))
            && let (Some(high), Some(low)) =
                ((*high as char).to_digit(16), (*low as char).to_digit(16))
        {
            decoded.push((high * 16 + low) as u8);
            offset += 3;
            continue;
        }
        decoded.push(bytes[offset]);
        offset += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn percent_encode(value: &str, uppercase: bool) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else if uppercase {
            encoded.push_str(&format!("%{byte:02X}"));
        } else {
            encoded.push_str(&format!("%{byte:02x}"));
        }
    }
    encoded
}

fn format_error_with_secrets(error: &Error, mut secrets: HashSet<String>) -> String {
    let Ok(patterns) = RedactionPatterns::new() else {
        // A redactor initialization bug must fail closed rather than leak an
        // entire configuration/command through the fallback formatter.
        return "Error details unavailable: credential redaction could not initialize".to_string();
    };
    let causes: Vec<_> = error.chain().map(ToString::to_string).collect();
    for cause in &causes {
        patterns.collect_secrets(cause, &mut secrets);
    }
    // Remove longer variants first, so a shorter secret cannot expose the
    // remainder of a quoted, encoded, or overlapping secret value.
    let mut secrets: Vec<_> = secrets
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));

    let mut rendered = Vec::new();
    for (index, cause) in causes.iter().enumerate() {
        if index >= MAX_DISPLAYED_CAUSES - 1 && index + 1 < causes.len() {
            if index == MAX_DISPLAYED_CAUSES - 1 {
                rendered.push("[intermediate causes omitted]".to_string());
            }
            continue;
        }
        let mut cause = patterns.redact(cause);
        for secret in &secrets {
            cause = redact_known_secret(&cause, secret);
        }
        // Keep each cause on one log record and strip terminal/control escapes.
        let cause: String = cause
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        // OS/Docker causes are frequently at the end of a long dependency
        // message. Keep its tail as well as the stage/path prefix.
        let characters: Vec<_> = cause.chars().collect();
        let bounded = if characters.len() > MAX_CAUSE_CHARACTERS {
            let head: String = characters[..MAX_CAUSE_CHARACTERS - 512].iter().collect();
            let tail: String = characters[characters.len() - 512..].iter().collect();
            format!("{head} [truncated] {tail}")
        } else {
            cause
        };
        rendered.push(bounded);
    }
    rendered.join("; caused by: ")
}

fn redact_known_secret(source: &str, secret: &str) -> String {
    if secret.chars().count() >= 4 {
        return source.replace(secret, REDACTED);
    }
    // Tiny configured values such as TOKEN=1 must not destroy errno 13, host
    // 192.168.32.131 or unrelated filesystem names. Labelled values were already
    // redacted above; unlabelled short values are hidden as standalone tokens.
    let mut output = String::with_capacity(source.len());
    let mut previous = 0;
    for (start, _) in source.match_indices(secret) {
        let end = start + secret.len();
        let adjacent = |character: char| {
            character.is_alphanumeric() || matches!(character, '_' | '/' | '\\' | '.' | ':' | '-')
        };
        let embedded = source[..start].chars().next_back().is_some_and(adjacent)
            || source[end..].chars().next().is_some_and(adjacent);
        if embedded {
            continue;
        }
        output.push_str(&source[previous..start]);
        output.push_str(REDACTED);
        previous = end;
    }
    output.push_str(&source[previous..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    #[test]
    fn nested_context_preserves_stage_path_and_errno() {
        let errno = if cfg!(windows) { 5 } else { 13 };
        let source = std::io::Error::from_raw_os_error(errno);
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        let expected_cause = source.to_string();
        let error = Error::from(source)
            .context("Failed to replace /home/test/docker/config/nginx.conf")
            .context("Offline candidate extraction failed before MySQL migration");
        let output = format_error(&error);
        assert!(output.contains("Offline candidate extraction"));
        assert!(output.contains("/home/test/docker/config/nginx.conf"));
        assert!(output.contains(&expected_cause));
        assert!(output.contains(&format!("os error {errno}")));
    }

    #[test]
    fn missing_configuration_key_names_stay_visible() {
        let error = anyhow!("Required keys missing or empty: MYSQL_PASSWORD, IM_RPC_SECRET_PLATFORM_IM, INTERNAL_API_KEY")
            .context("Candidate package preflight failed before stopping services");
        let output = format_error(&error);
        assert!(output.contains("MYSQL_PASSWORD"));
        assert!(output.contains("IM_RPC_SECRET_PLATFORM_IM"));
        assert!(output.contains("INTERNAL_API_KEY"));
    }

    #[test]
    fn address_conflict_preserves_user_actionable_port() {
        let error = anyhow!("Bind for 0.0.0.0:2379 failed: address already in use")
            .context("Failed to start milvus; resolve the host port conflict and retry");
        let output = format_error(&error);
        assert!(output.contains("0.0.0.0:2379"));
        assert!(output.contains("address already in use"));
        assert!(output.contains("resolve the host port conflict"));
    }

    #[test]
    fn assignment_json_and_command_flag_values_are_redacted() {
        let error = anyhow!(
            "{}",
            r#"MY_PASSWORD='env-secret' {"api_key":"json-secret"} docker exec --token flag-secret --port 8091; MYSQL_PWD=shell-secret"#
        );
        let output = format_error(&error);
        for secret in ["env-secret", "json-secret", "flag-secret", "shell-secret"] {
            assert!(!output.contains(secret), "secret leaked: {output}");
        }
        assert!(output.contains("MY_PASSWORD="));
        assert!(output.contains("--port 8091"));
    }

    #[test]
    fn mysql_password_flags_are_hidden_and_docker_port_mapping_is_preserved() {
        let error = anyhow!(
            "mysql -u root -pshort-secret -P3306; mysql -p 'spaced-secret'; docker run -p 127.0.0.1:8091:80/tcp; request ?pass=query-pass-secret"
        );
        let output = format_error(&error);
        for secret in ["short-secret", "spaced-secret", "query-pass-secret"] {
            assert!(!output.contains(secret), "secret leaked: {output}");
        }
        assert!(output.contains("-P3306"));
        assert!(output.contains("127.0.0.1:8091:80/tcp"));
    }

    #[test]
    fn whole_environment_dump_is_suppressed_even_with_unknown_key_names() {
        let error = anyhow!(
            "{}",
            r#"Environment: {"ARBITRARY":"unlabelled-secret","PORT":"8091"}; Permission denied (os error 13) at /home/test/docker/config"#
        );
        let output = format_error(&error);
        assert!(!output.contains("unlabelled-secret"));
        assert!(!output.contains("ARBITRARY"));
        assert!(output.contains("Permission denied (os error 13)"));
        assert!(output.contains("/home/test/docker/config"));
    }

    #[test]
    fn dsn_and_url_queries_keep_nonsecret_source_paths() {
        let error = anyhow!("mysql://user:dsn%2Fsecret%40one@127.0.0.1:3306/nuwax_im; GET https://packages.example/docker/20261010/amd64.zip?token=query-secret&arch=amd64&%70assword=encoded-key-secret")
            .context("DSN rejected literal dsn/secret@one and query-secret");
        let output = format_error(&error);
        for secret in [
            "dsn%2Fsecret%40one",
            "dsn/secret@one",
            "query-secret",
            "encoded-key-secret",
        ] {
            assert!(!output.contains(secret), "secret leaked: {output}");
        }
        assert!(output.contains("127.0.0.1:3306/nuwax_im"));
        assert!(output.contains("/docker/20261010/amd64.zip"));
        assert!(output.contains("arch=amd64"));
    }

    #[test]
    fn credentials_inside_json_keep_urls_and_hide_decoded_repeated_values() {
        let error = anyhow!("{}", r#"{"download_url":"https://packages.example/docker/amd64.zip?token=url-secret"} {"api_token":"line\njson-secret"}"#)
            .context("Dependency rejected url-secret and line\njson-secret");
        let output = format_error(&error);
        assert!(!output.contains("url-secret"));
        assert!(!output.contains("json-secret"));
        assert!(output.contains("/docker/amd64.zip"));
    }

    #[test]
    fn sql_credentials_and_repeated_literal_are_redacted() {
        let error = anyhow!("SQL: CREATE USER 'app'@'%' IDENTIFIED BY 'sql-secret'; ALTER USER 'app' IDENTIFIED WITH mysql_native_password BY 'another-secret'; rejected sql-secret")
            .context("MySQL migration failed at statement 7");
        let output = format_error(&error);
        assert!(output.contains("statement 7"));
        assert!(!output.contains("sql-secret"));
        assert!(!output.contains("another-secret"));
    }

    #[test]
    fn authorization_header_hides_bearer_value_and_repetition() {
        let error = anyhow!("Authorization: Bearer header-secret\nrequest rejected header-secret")
            .context("Failed to fetch /docker/latest.json");
        let output = format_error(&error);
        assert!(!output.contains("header-secret"));
        assert!(output.contains("/docker/latest.json"));
    }

    #[test]
    fn multiline_quoted_secrets_and_private_keys_are_redacted() {
        let error = anyhow!("IM_INTERNAL_SECRET='line-one\nline-two'\n-----BEGIN PRIVATE KEY-----\nprivate-key-value\n-----END PRIVATE KEY-----")
            .context("Failed to load configuration");
        let output = format_error(&error);
        for secret in ["line-one", "line-two", "private-key-value"] {
            assert!(!output.contains(secret), "secret leaked: {output}");
        }
        assert!(!output.contains('\n'));
    }

    #[test]
    fn selected_env_masks_unlabelled_and_encoded_dependency_values() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env = directory.path().join("operator.env");
        std::fs::write(
            &env,
            "MYSQL_PASSWORD='configured $secret/@: value'\nIM_INTERNAL_SECRET='multi\nline-secret'\nPORT=8091\n",
        )?;
        let secret = "configured $secret/@: value";
        let error = anyhow!(
            "SQL parser failed near '{secret}'; encoded={} multiline=multi\\nline-secret",
            percent_encode(secret, true)
        )
        .context("Migration failed at statement 2; port 8091");
        let before = std::fs::read(&env)?;
        let output = format_error_with_env(&error, &env);
        assert!(!output.contains(secret));
        assert!(!output.contains(&percent_encode(secret, true)));
        assert!(!output.contains("multi\\nline-secret"));
        assert!(output.contains("statement 2"));
        assert!(output.contains("port 8091"));
        assert_eq!(before, std::fs::read(&env)?);
        Ok(())
    }

    #[test]
    fn configured_secret_sql_escaping_is_hidden_without_a_password_label() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env = directory.path().join("operator.env");
        std::fs::write(&env, "MYSQL_PASSWORD=quote'secret\\value\n")?;
        let error = anyhow!(
            "SQL error near 'quote''secret\\value', alternatively quote\\'secret\\\\value; statement 3"
        );
        let output = format_error_with_env(&error, &env);
        assert!(!output.contains("quote''secret"));
        assert!(!output.contains("quote\\'secret"));
        assert!(output.contains("statement 3"));
        Ok(())
    }

    #[test]
    fn crypto_key_values_in_unlabelled_causes_are_redacted() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env = directory.path().join("operator.env");
        std::fs::write(
            &env,
            "AES_KEY='aes-key-secret'\nENCRYPTION_KEY=crypto-key-secret\n",
        )?;
        let error = anyhow!(
            "Decryption rejected aes-key-secret / crypto-key-secret; /home/test/docker/config"
        );
        let output = format_error_with_env(&error, &env);
        assert!(!output.contains("aes-key-secret"));
        assert!(!output.contains("crypto-key-secret"));
        assert!(output.contains("/home/test/docker/config"));
        Ok(())
    }

    #[test]
    fn shell_directory_keys_do_not_hide_paths() {
        assert!(!runtime_sensitive_key("PWD"));
        assert!(!runtime_sensitive_key("OLDPWD"));
        assert!(runtime_sensitive_key("MYSQL_PWD"));
        let output = format_error(&anyhow!("Failed to access /home/test/docker/config"));
        assert!(output.contains("/home/test/docker/config"));
    }

    #[test]
    fn tiny_configured_secret_keeps_errno_host_and_ports_visible() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env = directory.path().join("operator.env");
        std::fs::write(&env, "API_TOKEN=1\n")?;
        let error = anyhow!(
            "Permission denied (os error 13), host 192.168.32.131:8091; API_TOKEN=1; rejected standalone '1'; /tmp/data/1/file"
        );
        let output = format_error_with_env(&error, &env);
        assert!(output.contains("os error 13"));
        assert!(output.contains("192.168.32.131:8091"));
        assert!(output.contains("/tmp/data/1/file"));
        assert!(!output.contains("API_TOKEN=1"));
        assert!(!output.contains("standalone '1'"));
        Ok(())
    }

    #[test]
    fn mismatched_quotes_are_not_stripped_as_matching_quoted_value() {
        let mut secrets = HashSet::new();
        add_secret(&mut secrets, "'quote-secret\"");
        assert!(secrets.contains("'quote-secret\""));
        assert!(!secrets.contains("quote-secret"));
    }

    #[test]
    fn long_dependency_display_retains_address_conflict_tail() {
        let error = anyhow!(
            "Failed to start milvus: {}; 0.0.0.0:2379 address already in use",
            "detail ".repeat(700)
        );
        let output = format_error(&error);
        assert!(output.contains("Failed to start milvus"));
        assert!(output.contains("0.0.0.0:2379 address already in use"));
    }

    #[test]
    fn malformed_env_does_not_hide_original_error_or_raw_known_secret() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env = directory.path().join("operator.env");
        std::fs::write(&env, "malformed!record\nIM_INTERNAL_SECRET='raw-secret'\n")?;
        let error = anyhow!("Permission denied (os error 13); request rejected raw-secret");
        let output = format_error_with_env(&error, &env);
        assert!(output.contains("Permission denied (os error 13)"));
        assert!(!output.contains("raw-secret"));
        Ok(())
    }

    #[test]
    fn unknown_env_path_does_not_replace_root_cause() {
        let error = anyhow!("Address already in use on 127.0.0.1:2379");
        let output = format_error_with_env(&error, Path::new("/nonexistent/operator.env"));
        assert!(output.contains("127.0.0.1:2379"));
    }

    #[test]
    fn long_chains_are_bounded_but_retain_root_cause() {
        let mut error = anyhow!("Permission denied (os error 13)");
        for index in 0..25 {
            error = error.context(format!("stage {index}: {}", "a".repeat(3000)));
        }
        let output = format_error(&error);
        assert!(output.contains("stage 24"));
        assert!(output.contains("intermediate causes omitted"));
        assert!(output.contains("Permission denied (os error 13)"));
        assert!(output.contains("[truncated]"));
        assert!(output.len() < 35_000);
    }
}
