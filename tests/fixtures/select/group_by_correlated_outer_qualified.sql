-- The grouping rule must not reach a **correlated outer reference**. The inner query
-- groups `ib` by `ib.bar` and projects `oa.id` — a column of the *enclosing* level, not
-- of the relation being grouped, so Postgres accepts it.
--
-- pglet's grouping validator destructured `Expr::Column { name, .. }` and dropped the
-- qualifier, so `oa.id` was judged as a bare `id`. Because `ib` happens to carry an
-- `id` of its own, the "not a column of this table" exemption did not apply and the
-- query failed `42803 column "id" must appear in the GROUP BY clause`. The collision is
-- what makes it fire — an inner relation without an `id` was already exempt, which is
-- why the existing correlated-GROUP-BY coverage never caught it.
-- @setup
CREATE TABLE oa (id INT, val INT);
CREATE TABLE ib (id INT, foo INT, bar INT);
INSERT INTO oa VALUES (1, 7), (2, 8);
INSERT INTO ib VALUES (9, 7, 3), (8, 8, 4);
-- @test
SELECT oa.id AS oid, (SELECT oa.id FROM ib WHERE ib.foo = oa.val GROUP BY ib.bar LIMIT 1) AS sub FROM oa ORDER BY 1;
