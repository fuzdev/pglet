-- The suppression polarity of the same collision, and the half that shows the shadow was
-- feeding the gate a type rather than merely hiding a name: here the **base** table
-- carries the array and the derived table carries the int, so a scope that reads the FROM
-- name as the base table sees a legal `unnest(int[])` argument and lets a statement
-- through that 18.6 refuses. Measured pre-repair, this was pglet's own
-- `0A000 set-returning unnest() is only supported in a SELECT projection` — an error, so
-- an `@expect-error` fixture that ignored SQLSTATEs would have scored it green — against
-- 18.6's `42883 function unnest(integer) does not exist`. The harness compares the
-- SQLSTATE because Postgres errors here too, which is what makes this fixture a lock
-- rather than a formality.
--
-- The row is required: it is what carries the pre-repair engine past the plan-time gate
-- and into the `0A000`, so the two codes are distinguishable at all.
-- @setup
CREATE TABLE unnest_suppress (a INT[], c INT);
INSERT INTO unnest_suppress VALUES (ARRAY[7, 8], 1);

-- @test
-- @expect-error: the derived table's `a` is the int column, so unnest has no candidate (42883)
SELECT unnest(a) FROM (SELECT c AS a FROM unnest_suppress) AS unnest_suppress;
