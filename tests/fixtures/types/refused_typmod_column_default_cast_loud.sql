-- A persisted expression is a fourth route to the resolver, through the cast inside it,
-- and it is gated at DDL time: a column `DEFAULT` carrying a refused length takes the
-- `CREATE TABLE` down rather than storing an expression that would re-raise on every
-- insert — or dump to SQL stock Postgres refuses.
-- @expect-error: a character length below 1 inside a column DEFAULT
-- @expect-message
-- @test
CREATE TABLE rtdf (a char(1) DEFAULT 'x'::char(0));
