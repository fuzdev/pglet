-- `text = jsonb` has no operator (`42883`), even though both sides render to
-- comparable text.
-- @setup
CREATE TABLE t (name TEXT, data JSONB);
INSERT INTO t VALUES ('{"a": 1}', '{"a": 1}');
-- @test
-- @expect-error: no text = jsonb operator
SELECT * FROM t WHERE name = data;
