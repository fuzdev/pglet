-- @setup

-- @test
-- no column-alias list: columns default to Postgres's `column1`/`column2`, and a
-- bare `*` expands to them
SELECT * FROM (VALUES (1, 'x'), (2, 'y')) AS t ORDER BY column1;
