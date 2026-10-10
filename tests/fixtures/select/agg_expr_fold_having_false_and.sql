-- `HAVING false AND <aggregate predicate>` keeps no group, and the dropped aggregate
-- never runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT max(a) FROM u HAVING false AND sum(1/b) > 0;
