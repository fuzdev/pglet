-- A computed `jsonb` side — `to_jsonb(…)`, a scalar subquery's `jsonb` column — reads the
-- untyped side as `jsonb` too.
-- @setup
CREATE TABLE jcc (id INT, j JSONB);
INSERT INTO jcc VALUES (1, '{"k": 1}');
-- @test
SELECT to_jsonb(t) || '{"extra": 1}' AS row_merge, to_jsonb(t.id) || '[3]' AS scalar_append,
       (SELECT j FROM jcc) || '{"b": 2}' AS sub_merge FROM jcc t;
