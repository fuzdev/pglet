-- `version()`'s string is build-specific on both engines — 18.6 names its Debian
-- packaging and its gcc, pglet names `pglet` and the Rust target triple, which
-- differs between the native and wasm legs — so **the string itself is what this
-- fixture never looks at**. What is portable, and what every driver's version
-- sniff actually reads, is the `PostgreSQL <release> (` prefix.
-- @test
SELECT version() LIKE 'PostgreSQL 18.6 (%' AS sniffable;
