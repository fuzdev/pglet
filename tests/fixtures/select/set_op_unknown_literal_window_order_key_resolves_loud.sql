-- A window `ORDER BY` key equal to the constant resolves it to `text` before the set
-- operation sees it, as an `ORDER BY` naming it does.
-- @setup
CREATE TABLE wok (a INT);
INSERT INTO wok VALUES (7);
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1, 1 UNION ALL SELECT '2', row_number() OVER (ORDER BY '2') FROM wok;
