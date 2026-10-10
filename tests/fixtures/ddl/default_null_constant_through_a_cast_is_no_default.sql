-- 18.6 skips `StoreAttrDefault` when the **cooked** expression is a NULL `Const`, and
-- `coerce_type` folds an `unknown` literal into a typed `Const` in place — so one
-- level of cast over a bare `NULL` is still a NULL constant and stores no
-- `pg_attrdef` row. A *second* cast is a real conversion rather than a constant, so
-- `NULL::text::int` **is** stored: the `c` column is the control that identifies the
-- rule as "a constant" rather than "contains a NULL".
-- @no-declared-types: `information_schema.columns.column_name` is Postgres's `name`
-- type (OID 19); pglet's synthesized view still declares it `text` (25) — `name` exists as a type now but the catalog columns have not moved to it. The same
-- catalog-view type gap the other `information_schema` fixtures waive, and outside
-- this fixture's subject.
-- @setup
CREATE TABLE def_nullc (
  a INT DEFAULT NULL::int,
  b INT DEFAULT CAST(NULL AS int),
  c INT DEFAULT NULL::text::int,
  d INT DEFAULT NULL
);
-- @test
SELECT column_name, column_default IS NULL AS no_default
FROM information_schema.columns WHERE table_name = 'def_nullc' ORDER BY ordinal_position;
