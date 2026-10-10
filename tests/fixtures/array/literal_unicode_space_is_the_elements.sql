-- A Unicode space is not the array reader's whitespace: a no-break space or an em space
-- beside an element is one of the element's characters.
-- @test
SELECT '{a }'::text[] = ARRAY['a '] AS trailing_kept,
       '{ a, b}'::text[] = ARRAY[' a', ' b'] AS leading_kept,
       '{a }'::text[] = ARRAY['a'] AS trimmed,
       '{a }'::bytea[] AS as_bytes,
       '{ NULL}'::text[] = ARRAY[' NULL'] AS not_a_null;
