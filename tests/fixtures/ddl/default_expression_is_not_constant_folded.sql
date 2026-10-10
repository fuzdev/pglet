-- No constant folding: `column_default` reports the expression, not its value.
-- `cookDefault` does not run `eval_const_expressions`, so 18.6 reads back `(1 + 1)`
-- rather than `2` — and an engine that folded at DDL time would report `2` here while
-- still answering `2` to every read, which is the silent half.
-- @no-declared-types: `information_schema.columns.column_default` is Postgres's
-- `character varying` (OID 1043) and pglet's synthesized view declares `text` (25) —
-- the same catalog-view type gap the other `information_schema` fixtures waive, and
-- outside this fixture's subject, which is the rendered default text.
-- @setup
CREATE TABLE def_fold (a INT DEFAULT 1 + 1);
-- @test
SELECT column_default FROM information_schema.columns
WHERE table_name = 'def_fold' AND column_name = 'a';
