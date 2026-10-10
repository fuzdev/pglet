-- The correlated outer reference in the **subquery's own `HAVING`**: `ch_o.ouid`
-- resolves against the enclosing level, so the bare string literal beside it gets its
-- coercing cast to `uuid` and the comparison matches — instead of reaching the
-- eval-time gate as `uuid = text` and raising `42883`. That is the whole of what this
-- fixture observes.
--
-- What it does **not** cross, despite the shape suggesting it: `validate_grouped_expr`
-- (which checks a `HAVING` expression against the inner `GROUP BY` list) also has a
-- say, and `ch_o.ouid` clears it twice over — the qualifier names another level's
-- relation (`is_own_relation`), *and* `ouid` is not a base column of `ch_i` at all, so
-- the escape hatch for a name this table does not hold would wave it through even if
-- the qualifier were ignored. This fixture therefore cannot tell those two apart.
-- The shape that can is one where the two tables share the column name:
--
--   CREATE TABLE go (okey INT, tag UUID); CREATE TABLE gi (ikey INT, tag TEXT);
--   SELECT okey FROM go WHERE EXISTS (SELECT 1 FROM gi GROUP BY gi.ikey
--     HAVING go.tag = '11111111-1111-1111-1111-111111111111');
--
-- That one used to raise `42803` here and is measured returning the row on both
-- engines now that both grouping checks test the qualifier.
--
-- The escape hatch that clears it is decided in that one direction only:
-- `!is_own_relation(table)` is equally true of a qualifier naming **no** relation, so
-- the grouping check alone lets `HAVING bogusrel.tag = 'z'` through. What refuses it is
-- the plan-time binding gate above that check, and the shape has its own fixtures —
-- `select/having_bogus_qualifier_loud.sql` for the read and
-- `delete/subquery_aggregate_bogus_qualifier_loud.sql` for the write it used to make
-- (`EDGES.md` row 30). This fixture must not be read as covering either.
-- @setup
CREATE TABLE ch_o (okey INT, ouid UUID);
CREATE TABLE ch_i (ikey INT, itag TEXT);
INSERT INTO ch_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO ch_i VALUES (1, 'a'), (2, 'b');
-- @test
SELECT okey FROM ch_o WHERE EXISTS (SELECT 1 FROM ch_i WHERE ch_i.ikey = ch_o.okey GROUP BY ch_i.ikey HAVING ch_o.ouid = '11111111-1111-1111-1111-111111111111') ORDER BY okey;
