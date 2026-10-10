-- A window `PARTITION BY` key equal to the constant resolves it to `text` before the set
-- operation sees it, as a `GROUP BY` naming it does.
-- @setup
CREATE TABLE wpk (a INT);
INSERT INTO wpk VALUES (7);
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1, 1 UNION ALL SELECT '2', row_number() OVER (PARTITION BY '2') FROM wpk;
