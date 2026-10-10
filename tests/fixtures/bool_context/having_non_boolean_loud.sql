-- `HAVING` names itself and dropped every group silently.
-- @setup
CREATE TABLE bcg (a INT);
INSERT INTO bcg VALUES (9);
-- @expect-error: argument of HAVING must be type boolean
-- @test
SELECT count(*) FROM bcg GROUP BY a HAVING a;
