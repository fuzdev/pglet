-- Postgres *does* define `bytea ~~ bytea` (byte-wise, `%` = 0x25, `_` = 0x5F) and
-- returns `f` here; pglet's `like_match` works over `&str`, so the operator is
-- unimplemented (`0A000`), not nonexistent (`42883`). Postgres accepts the query,
-- so any pglet error satisfies `@expect-error` — this locks that pglet stays loud
-- rather than silently matching. (There is no `bytea ~~* bytea` in Postgres, so
-- `ILIKE` over bytea stays `42883`.)
-- @test
-- @expect-error: bytea LIKE bytea unimplemented in pglet
SELECT '\xAA'::bytea LIKE '\xBB'::bytea;
