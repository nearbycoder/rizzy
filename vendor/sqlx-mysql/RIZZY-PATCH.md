This directory is SQLx MySQL 0.8.6 with its original MIT and Apache-2.0
licenses. Rizzy patches `src/any.rs`: SQLx's generic `Any` driver classifies
MariaDB 13's `utf8mb4_bin LONGTEXT` columns as binary `BLOB`, preventing
`String` decoding. The patch treats the matching 4 GiB column type as text.

Rizzy's MySQL schema currently has no binary data columns. Before updating
SQLx or adding binary columns, verify this mapping with both MySQL and
MariaDB and remove the local patch when upstream provides a suitable fix.
