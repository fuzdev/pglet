-- A nested level's `WHERE` is re-resolved against the chained scope, and only
-- that re-walk can see a correlated reference's type — so a gate at the lowering
-- sites alone leaves every subquery's `WHERE` ungated, which is the half that
-- silently writes.
-- @setup
CREATE TABLE bcc1 (a INT);
CREATE TABLE bcc2 (z INT);
INSERT INTO bcc1 VALUES (1);
-- @expect-error: argument of WHERE must be type boolean
-- @test
DELETE FROM bcc1 WHERE EXISTS (SELECT 1 FROM bcc2 WHERE bcc1.a);
