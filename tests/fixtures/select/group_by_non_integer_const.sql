-- A bare **non-integer constant** as a `GROUP BY` key is `42601` "non-integer constant
-- in GROUP BY" — only an integer literal is a positional reference, and no other
-- constant is a meaningful grouping key. pglet accepted it silently, grouping every
-- row into one group and returning a wrong single-row answer.
-- @setup
CREATE TABLE gbnc (x INT, y INT);
INSERT INTO gbnc VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: non-integer constant in GROUP BY (42601)
SELECT count(*) AS c FROM gbnc GROUP BY 'str';
