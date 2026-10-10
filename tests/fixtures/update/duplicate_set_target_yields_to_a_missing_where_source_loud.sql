-- The skip is **wider** than the identity guard's, and this is why. A `WHERE` subquery
-- naming a relation the catalog lacks is `42P01` on 18.6 (its range table is resolved as
-- the clause is transformed) and reaches pglet's own `42P01` per row. Taking only the
-- `FROM` half of the skip — which is what the identity guard below this gate takes —
-- would answer `42601` here instead.
--
-- What makes the wide skip payable here, where it was not for the identity guard, is
-- that it takes no refusal away: over a row the `WHERE` raises `42P01` before anything
-- is written, and over no row the statement writes nothing either way. The `RETURNING`
-- spelling (`… RETURNING (SELECT 1 FROM nosuchrel)`) takes the same half, and its row is
-- written before the clause is evaluated — so what unwinds it is the autocommit
-- statement rollback, which `--storage memory` does not have.
-- @setup
CREATE TABLE dsmw (a INT, b INT);
INSERT INTO dsmw VALUES (0, 0);

-- @expect-error: relation "nosuchrel" does not exist (42P01)
-- @no-profile-lock: the refusal is raised at execution, and the statement plans no index scan
-- @test
UPDATE dsmw SET a = 1, a = 2 WHERE EXISTS (SELECT 1 FROM nosuchrel);
