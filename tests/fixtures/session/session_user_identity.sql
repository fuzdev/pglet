-- The four role spellings, all parenless RESERVED_KEYWORDs on 18.6.
--
-- Byte-comparable because the harness connects to **both** engines as the same
-- role (`pglet_tests`' `POSTGRES_CONFIG` / `pglet_config`, `user=postgres`), and
-- pglet echoes the startup packet's `user` the way a `trust`-authenticated
-- Postgres does. They cannot differ from each other here: pglet models no roles,
-- so the `SET ROLE` that splits `current_user` from `session_user` on 18.6 is an
-- unmodelled no-op.
-- @test
SELECT current_user, session_user, user, current_role;
