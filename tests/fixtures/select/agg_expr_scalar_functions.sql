-- An aggregate under a scalar function call answers: the grouping computes the aggregate,
-- and the ordinary scalar walk evaluates the call over the value it reads for that node.
-- Declared types are compared on both legs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT round(avg(a), 2) AS r2, round(avg(a)) AS r0, abs(sum(a)) AS ab,
  sqrt(max(a)) AS sq, upper(max(y)) AS up, lower(min(y)) AS lo, length(max(y)) AS len,
  concat('n=', count(*)) AS cc, jsonb_build_object('n', count(*)) AS jb
FROM u;
