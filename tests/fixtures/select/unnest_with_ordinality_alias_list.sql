-- An alias list naming the element and the ordinality column, without `AS`, over an empty
-- array's no rows beside a NULL array's: neither yields a row to number.
-- @test
SELECT x, o FROM unnest(ARRAY[3, 1, 2]) WITH ORDINALITY h(x, o)
UNION ALL SELECT x, o FROM unnest(ARRAY[]::int[]) WITH ORDINALITY h(x, o)
UNION ALL SELECT x, o FROM unnest(NULL::int[]) WITH ORDINALITY h(x, o)
ORDER BY o;
