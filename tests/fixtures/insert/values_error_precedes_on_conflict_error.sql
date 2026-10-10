-- Clause order inside an INSERT, the level above expression source order — the
-- companion to `update/where_error_precedes_set_error.sql`. The rule this locks is
-- Postgres's: `analyze.c`'s `transformInsertStmt` transforms the row source,
-- **then** `transformOnConflictClause`, then the `RETURNING` clause, so when two
-- clauses are ill-typed Postgres reports the earlier one — here the `VALUES` cell's
-- `42804`, not the `DO UPDATE SET`'s `42883`.
--
-- This is an **order lock, not a regression lock**: it pins where the rule is
-- currently satisfied, so a future refactor that resolves `ON CONFLICT` earlier
-- (where it is parsed, say) turns it red. It records no past pglet bug — no
-- committed revision reported the `SET`'s code here.
--
-- The two clauses raise **different** SQLSTATEs on purpose: with the same code on
-- both, the fixture would score green whichever clause won and would lock nothing.
-- @setup
CREATE TABLE vpo (id INT PRIMARY KEY, i4 INT4, tx TEXT);
INSERT INTO vpo VALUES (1, 10, 'a');
-- @test
-- @expect-error: the VALUES cell resolves first, so its no-common-type error wins (42804) over the DO UPDATE SET's 42883
-- @no-profile-lock: the error is raised while the INSERT is lowered, before any index selection runs, so both profiles reach it identically
INSERT INTO vpo VALUES (1, COALESCE(1, 'x'::TEXT), 'b')
  ON CONFLICT (id) DO UPDATE SET tx = (vpo.i4 LIKE 'x%')::TEXT;
