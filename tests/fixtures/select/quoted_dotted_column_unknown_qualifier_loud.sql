-- The other half of the quoted-dotted-name collision: with **no** `a` in scope,
-- `a.b` names a table that isn't in the FROM clause, so Postgres raises `42P01`
-- (`missing FROM-clause entry for table "a"`). Consulting the synthesized-join-name
-- tier unconditionally made pglet answer with the `"a.b"` column instead — a silent
-- wrong answer to a query that has no right answer.
--
-- The tier is gated on the scope having no single effective table, which is exactly
-- the join case that synthesizes dotted names; a single-table scope like this one
-- takes its names straight from the table, so a qualifier that doesn't name it is
-- unknown and falls through to the `42P01`.
-- @setup
CREATE TABLE dq2 ("a.b" TEXT, z INT);
INSERT INTO dq2 VALUES ('X', 1);

-- @expect-error: the qualifier names no in-scope table
-- @test
SELECT a.b FROM dq2;
