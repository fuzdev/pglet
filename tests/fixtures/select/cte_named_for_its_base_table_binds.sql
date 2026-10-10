-- The CTE spelling of the same shadow. `WITH t AS (SELECT … FROM t)` — a CTE named for
-- the table it derives from — is idiomatic SQL, and it reached the identical defect by
-- the identical route: a read-only CTE reference inlines as a `Plan::SubqueryScan`, so
-- its columns live in the same map a derived table's do, and `build_column_scope` asked
-- `schema.get_table` before that map. `42703 column "z" does not exist` here against
-- 18.6's row.
--
-- Kept separate from the derived-table fixture because the two are different *parse*
-- routes to one scope entry (`Parser::ctes` versus `parse_derived_table`), and a repair
-- that reached only one of them would still look green on the other.
-- @setup
CREATE TABLE cte_shadow (a INT, b INT[]);
INSERT INTO cte_shadow VALUES (1, ARRAY[7, 8]);

-- @test
WITH cte_shadow AS (SELECT 1 AS z) SELECT z FROM cte_shadow;
