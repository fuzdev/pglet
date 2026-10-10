-- Two arguments of one call: the bare one is analysed before the cast beside it types
-- the parameter, so its reference stays untyped.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT jsonb_build_object('k', $1, 'j', $1::text) AS j;
