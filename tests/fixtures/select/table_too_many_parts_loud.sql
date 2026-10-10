-- A name with more than two dotted parts fails loud (42601), matching Postgres's
-- "improper qualified name (too many dotted names)" — pglet does not model the
-- `db.schema.table` form.
-- @expect-error: improper qualified name, too many dotted names (42601)
-- @test
SELECT * FROM a.b.c.d;
