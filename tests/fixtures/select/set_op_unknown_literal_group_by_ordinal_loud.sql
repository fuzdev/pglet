-- A `GROUP BY` ordinal naming the constant resolves it to `text`.
-- @setup
CREATE TABLE sgo (a INT);
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1 UNION ALL SELECT '2' FROM sgo GROUP BY 1;
