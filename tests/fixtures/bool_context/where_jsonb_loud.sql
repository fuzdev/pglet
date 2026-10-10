-- `jsonb` is one of the two types with a `pg_cast` row into `boolean`, and it is
-- **explicit**-context — so a boolean position refuses it like any other type.
-- @setup
CREATE TABLE bcn4 (j JSONB);
INSERT INTO bcn4 VALUES ('{"k":1}');
-- @expect-error: argument of WHERE must be type boolean, not type jsonb
-- @test
SELECT count(*) FROM bcn4 WHERE j;
