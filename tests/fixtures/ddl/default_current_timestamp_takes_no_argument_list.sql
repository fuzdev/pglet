-- `CURRENT_TIMESTAMP` is a SQL value function, not a `pg_proc` entry, so it takes no
-- argument list: `current_timestamp()` is `42601` on 18.6. The token-level classifier
-- used to swallow the empty parens and define the column, backfilling a real
-- timestamp where 18.6 defines no default at all — this clause's one silent member.
-- @expect-error: a SQL value function takes no argument list
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 at the closing paren, pglet at the opening one
-- @test
CREATE TABLE def_cts (a TIMESTAMPTZ DEFAULT current_timestamp());
