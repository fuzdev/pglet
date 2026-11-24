-- A bare `*` names every column of the FROM clause's relations, and with no FROM there are
-- none: `42601`, with a `WHERE` or without.
-- @expect-error: SELECT * with no tables
-- @test
SELECT * WHERE true;
