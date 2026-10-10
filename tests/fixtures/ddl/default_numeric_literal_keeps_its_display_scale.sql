-- Postgres's constant typing: a decimal literal is a `numeric` constant, so
-- `DEFAULT 1.50` keeps its scale in `column_default`. Storing it as a float lost the
-- scale here — the value agreed, the reported default did not.
-- @no-declared-types: `information_schema.columns.column_default` is Postgres's
-- `character varying` (OID 1043) and pglet's synthesized view declares `text` (25) —
-- the same catalog-view type gap the other `information_schema` fixtures waive, and
-- outside this fixture's subject, which is the rendered default text.
-- @setup
CREATE TABLE def_scale (a NUMERIC DEFAULT 1.50);
-- @test
SELECT column_default FROM information_schema.columns
WHERE table_name = 'def_scale' AND column_name = 'a';
