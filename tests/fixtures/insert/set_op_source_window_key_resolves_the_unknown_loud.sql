-- An `INSERT … SELECT` over a set operation whose side's constant a window key resolves
-- to `text`: the set operation refuses the pair rather than writing both rows.
-- @setup
CREATE TABLE swk_src (a INT);
INSERT INTO swk_src VALUES (7);
CREATE TABLE swk_dst (i INT, b BIGINT);
-- @expect-error: UNION types integer and text cannot be matched
-- @test
INSERT INTO swk_dst (i, b) SELECT 5, 1 UNION SELECT '6', row_number() OVER (PARTITION BY '6') FROM swk_src;
