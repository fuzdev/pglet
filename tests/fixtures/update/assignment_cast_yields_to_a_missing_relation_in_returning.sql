-- **An error-order control — do not trim it**, and the third clause of the same
-- contract. `transformUpdateStmt` runs FROM, then the `WHERE`, then `RETURNING`, and
-- only then the target list — so a relation any of the three names and the catalog
-- lacks outranks the `SET` list's `42804` on 18.6. Its two siblings pin the other two
-- clauses (`assignment_cast_set_yields_to_a_missing_source.sql`,
-- `assignment_cast_yields_to_a_missing_relation_in_the_where.sql`).
--
-- Enumerating the clauses is the part that goes stale: a gate written for the `FROM`
-- alone leaves the other two pre-empting 18.6's `42P01` with a `42804`, which is what
-- these three measure rather than argue.
--
-- The row is load-bearing for the same reason its `WHERE` sibling's is: a `RETURNING`
-- item is projected per updated row, so a statement matching none never reaches the
-- subquery at all.
-- @setup
CREATE TABLE acmr (id INT, b BOOL);
INSERT INTO acmr VALUES (1, FALSE);

-- @expect-error: relation "acmr_nosuch" does not exist (42P01)
-- @test
UPDATE acmr SET b = 1 RETURNING (SELECT 1 FROM acmr_nosuch);
