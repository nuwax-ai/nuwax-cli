# MySQL integration test container

This directory provides the MySQL service used by `client-core/tests/mysql_integration_test.rs`.

Default behavior:

- `cargo test --workspace --locked` starts this MySQL container automatically when `TEST_MYSQL_URL` is not set.
- The compose project name is `nuwax_mysql_integration`.
- The host port defaults to `13306`.
- The container is kept after the test so repeated local test runs are faster.

Useful environment variables:

- `TEST_MYSQL_URL=mysql://user:password@host:port/database`: use an existing MySQL service instead of Docker Compose.
- `TEST_MYSQL_REQUIRED=1`: fail the test if Docker Compose or MySQL startup is unavailable.
- `TEST_MYSQL_CLEANUP=1`: run `docker compose down -v` after the integration test.
