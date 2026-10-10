-- The other spelling of the same rule, and the one a `default.is_some()` test misses:
-- a `SERIAL` type name *is* a default clause on 18.6 (it synthesizes the `nextval`),
-- so `a SERIAL DEFAULT 5` is the same `42601`. Accepting it dropped the implicit
-- sequence the type name asked for.
-- @expect-error: multiple default values specified for column "a"
-- @test
CREATE TABLE def_serial_multi (a SERIAL DEFAULT 5);
