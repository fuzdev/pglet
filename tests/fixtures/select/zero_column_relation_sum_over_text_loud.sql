-- The silent-wrong the same `float8` guess created, and the reason its rung was a
-- behavior change rather than a refactor. `max('abc')` over a zero-column relation was
-- declared `float8` by the executor walk; that descriptor is what the outer `sum`'s
-- operand gate reads through `certain_expr_type_oid`, and `aggregate_legality(Sum,
-- float8)` accepts what `aggregate_legality(Sum, text)` refuses — so the statement
-- answered a single NULL row where 18.6 raises `42883`. `avg(m)` and the `WITH` spelling
-- of the same shape are the other two measured members; one holds the class.
--
-- The setup builds no index of any kind, so the `@expect-error`-over-an-index rule does
-- not arm here and neither profile plans an accelerated node for this statement.
--
-- The comparison is on the **SQLSTATE**, not the phase it arrives at: measured, pglet
-- raises this `42883` at `Execute` while 18.6 raises it at `Parse`, and the fixture is
-- green across that difference. That phase gap is pre-existing, reproduces over an
-- ordinary table, and is `EDGES.md` row 60.
-- @setup
CREATE TABLE zc2 (x INT);
INSERT INTO zc2 VALUES (1), (2);
ALTER TABLE zc2 DROP COLUMN x;

-- @test
-- @expect-error: no sum(text)
SELECT sum(m) AS s FROM (SELECT max('abc') AS m FROM zc2) q;
