-- `jsonb #> $1` with a bound text[] path parameter over the extended protocol. The
-- right operand of `#>`/`#>>`/`?|`/`?&` is a text[], so `param_types` force-types
-- `$1` as text[] (left untyped it would report OID 0).
-- @setup
CREATE TABLE jpp (id int, data jsonb);
INSERT INTO jpp VALUES (1, '{"a":{"b":42}}');
-- @param $1 text[] {a,b}
-- @test
SELECT data #> $1 AS v FROM jpp;
