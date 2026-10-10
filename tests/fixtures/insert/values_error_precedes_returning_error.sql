-- The third clause of the same order (see
-- `insert/values_error_precedes_on_conflict_error.sql`): `transformReturningClause`
-- runs last of the three, so in Postgres a `RETURNING` error never outranks a
-- `VALUES` one — the cell's `42804` wins over the projection's `42883`.
--
-- An **order lock, not a regression lock**, exactly as the sibling: it fails if
-- `RETURNING` is ever lowered ahead of the cells, and records no past pglet bug.
--
-- The two clauses raise different SQLSTATEs on purpose — see the sibling.
-- @setup
CREATE TABLE vpr (id INT, i4 INT4, tx TEXT);
-- @test
-- @expect-error: the VALUES cell resolves first, so its no-common-type error wins (42804) over the RETURNING's 42883
INSERT INTO vpr VALUES (1, COALESCE(1, 'x'::TEXT), 'b') RETURNING i4 LIKE 'x%';
