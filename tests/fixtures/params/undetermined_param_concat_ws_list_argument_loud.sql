-- `concat_ws`'s separator is `text` and types a parameter; the list after it is
-- `"any"` and does not.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT concat_ws(',', 'a', $1) AS c;
