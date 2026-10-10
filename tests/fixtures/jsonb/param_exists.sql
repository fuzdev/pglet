-- `jsonb ? $1` with a bound text key parameter over the extended protocol. The
-- right operand of `?` is text, so `param_types` force-types `$1` as text (left
-- untyped it would report OID 0 and trip a binary-protocol client's introspection).
-- @setup
CREATE TABLE jp (id int, data jsonb);
INSERT INTO jp VALUES (1, '{"a":1,"b":2}'), (2, '{"x":9}'), (3, '{"a":7}');
-- @param $1 text a
-- @test
SELECT id FROM jp WHERE data ? $1 ORDER BY id;
