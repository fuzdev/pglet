-- The accept side: JSON's own whitespace -- space, tab, LF and CR -- is skipped at every
-- position the reader skips whitespace, on both engines. A gate that narrowed past these
-- four would redden here.
-- @test
SELECT (E' \t\n\r[1, \t2] \r\n')::jsonb::text AS around_an_array,
       (E'{\n  "k" :\t1,\r\n  "kk" : [ ]\n}')::jsonb::text AS inside_an_object,
       (E'{ }')::jsonb::text AS empty_object,
       (E'[\t]')::jsonb::text AS empty_array;
