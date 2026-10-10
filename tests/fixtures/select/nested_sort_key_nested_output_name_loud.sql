-- @setup
CREATE TABLE nsk1 (q INT, a INT, b INT);
INSERT INTO nsk1 VALUES (1, 1, 10), (2, 2, 20), (3, 2, 30);

-- `findTargetlistEntrySQL92` is entered only for a key that **is** a one-field
-- `ColumnRef`, so a name *nested* inside a key is an ordinary column reference resolved
-- against the range table. The first lowering has always applied that shape test
-- (`key_may_name_an_output_column`); the correlated re-walk pushed the output names into
-- the scope for the whole key tree instead, so the identical key was judged correctly at
-- top level and wrongly one level down, answering `10`.
--
-- The top-level spelling is the discriminator and lives in
-- `order_by_nested_name_with_no_base_column_loud.sql`; the accept polarity — the same key *bare* —
-- is `nested_sort_key_bare_output_name_resolves.sql`.
-- @expect-error: a nested output name in a subquery's ORDER BY key is 42703
-- @test
SELECT (SELECT q * 10 AS z FROM nsk1 ORDER BY z + 0 LIMIT 1) AS r;
