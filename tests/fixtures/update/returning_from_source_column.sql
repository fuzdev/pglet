-- `UPDATE t SET … FROM s WHERE … RETURNING s.col` — a qualified reference to a `FROM`
-- source's column, over a statement that matches **no row**.
--
-- The zero rows are the point, twice over. They are what makes this a *declared-type*
-- fixture: with no values to compare, the only thing the harness can see is the
-- `RowDescription`, which it compares live against 18.6 on both protocol legs. And they
-- are the shape the whole clause used to get wrong in a row-count-dependent way — with a
-- matching row the executor raised `42P01`, and with none it reached no resolver at all
-- and succeeded, declaring the source column's `int8` on the extended leg and `int4` on
-- the simple one. Two walkers disagreeing, on a statement both engines accept, visible
-- only when nothing matched.
--
-- The source is **unaliased**, so its own table name is the qualifier — the polarity
-- that separates "the alias is in scope" from "the relation is". Its aliased twin, and
-- the `42P01` an alias imposes on the table name, are
-- `returning_from_source_qualified_column.sql` and
-- `returning_from_source_name_hidden_by_alias.sql`.
--
-- `rfs_s.big` is `bigint` where the target carries a `smallint` and a `bigint`, so a
-- build resolving the reference against the wrong relation declares a different width.
-- @setup
CREATE TABLE rfs_t (id INT, s SMALLINT, b BIGINT);
CREATE TABLE rfs_s (id INT, big BIGINT);
INSERT INTO rfs_t VALUES (1, 11, 111);
INSERT INTO rfs_s VALUES (1, 999);

-- @test
UPDATE rfs_t SET s = 12 FROM rfs_s WHERE rfs_t.id = 999 AND rfs_t.id = rfs_s.id RETURNING rfs_s.big + 0;
