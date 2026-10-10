-- A **zero-column relation** is the shape where an executor walk that guesses `float8`
-- for an unresolved `sum`/`min`/`max` argument would fire. Two measured routes build one,
-- and this fixture takes the first: `ALTER TABLE … DROP COLUMN` down to nothing, which
-- both engines accept, and a `RETURNING`-less data-modifying CTE the outer query
-- references, which 18.6 refuses outright (`0A000`) — so "which both engines accept"
-- bounds this spelling and not the class, and the CTE route cannot be a fixture at all
-- (`EDGES.md` rows 58 and 61).
--
-- Both legs declare `25` for `m` and `n`, as 18.6 does. A walk that guesses `float8`
-- for the unresolved `text` argument declares `701` on the simple-query `RowDescription`
-- against `Describe`'s `25`; the values are `abc` on every leg, so only the OID moves,
-- which is why the declared-type comparison is this fixture's whole subject.
--
-- `count(*)` is the polarity pin: its result type is fixed, so it declares `20`
-- whichever way the argument is typed.
-- @setup
CREATE TABLE zc (x INT);
INSERT INTO zc VALUES (1), (2);
ALTER TABLE zc DROP COLUMN x;

-- @test
SELECT max('abc') AS m,  -- text (was float8 on the simple leg)
       min('abc') AS n,  -- text (was float8 on the simple leg)
       count(*) AS c     -- int8 — polarity pin
FROM zc;
