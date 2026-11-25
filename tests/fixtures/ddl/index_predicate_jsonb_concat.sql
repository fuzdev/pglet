-- `jsonb || jsonb` in a stored index predicate, which holds the rows whose merge is
-- not empty.
-- @setup
CREATE TABLE jc2 (id INT, a JSONB, b JSONB);
CREATE INDEX jc2_i ON jc2 (id) WHERE (a || b) <> '{}'::jsonb;
INSERT INTO jc2 VALUES (1, '{}', '{}'), (2, '{"x": 1}', '{}'), (3, NULL, '{}');
-- @test
SELECT id FROM jc2 WHERE (a || b) <> '{}'::jsonb ORDER BY id;
