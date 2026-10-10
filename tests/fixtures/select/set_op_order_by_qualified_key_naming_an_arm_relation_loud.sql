-- The control for the qualified member: the refusal is a property of the **namespace**,
-- not of the name being made up. `sopa` is a relation this very statement reads, and the
-- trailing `ORDER BY` still cannot name it — 18.6's set-op sort clause sees the combined
-- result's output columns and no range table at all, so this is `42P01` there as much as
-- `bogusrel.a` is.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);
INSERT INTO sopa VALUES (3);
INSERT INTO sopb VALUES (2);

-- @expect-error: missing FROM-clause entry for table "sopa" (42P01)
-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY sopa.a;
