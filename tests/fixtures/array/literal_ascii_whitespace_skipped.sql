-- The array reader's whitespace is the space, tab, newline, carriage return, vertical tab
-- and form feed: each is skipped around an element and around the braces.
-- @test
SELECT E' \t\n{\va\f,\r"b"\v} \f'::text[] AS a, E'{\v1\f}'::int4[] AS b;
