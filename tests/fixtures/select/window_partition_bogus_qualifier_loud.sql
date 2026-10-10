-- The window-clause spelling of the same class: a `PARTITION BY` key qualified by a
-- relation the statement does not name **answered**, partitioning by whatever bare column
-- the loose tier found. `42P01` on 18.6.
--
-- Window `PARTITION BY` / `ORDER BY` keys are SQL99 and may not name an output alias on
-- either engine, so they gate with the projection and need none of the
-- `findTargetlistEntrySQL92` allowance the statement's own `ORDER BY` does — which is why
-- this fixture and `select/order_by_output_alias_precedence.sql` can both be green.
-- @setup
CREATE TABLE wpb (a INT, b TEXT);
INSERT INTO wpb VALUES (1, 'z');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
SELECT a, row_number() OVER (PARTITION BY zzz.v) FROM wpb;
