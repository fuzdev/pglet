-- The same refusal ahead of a write: nothing is inserted.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: recursive reference within EXCEPT (42P19)
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL (SELECT n + 1 FROM r WHERE n < 3 EXCEPT ALL SELECT 99))
INSERT INTO s SELECT n FROM r;
