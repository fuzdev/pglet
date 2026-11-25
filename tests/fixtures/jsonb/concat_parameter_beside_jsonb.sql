-- A bare `$N` beside a `jsonb` operand is `jsonb`, in an update and in a select list.
-- @setup
CREATE TABLE jcp (id INT, data JSONB);
INSERT INTO jcp VALUES (1, '{"a": 1}');
-- @param $1 jsonb {"b": 2}
-- @test
UPDATE jcp SET data = data || $1 RETURNING data, $1 || data AS flipped;
