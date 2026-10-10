-- Both ends of the refused range, on the accept side. 18.6's lexer refuses a byte at or
-- below 31, so `0x20` (a space) is the first character it takes and `0x7F` is above the
-- bound entirely -- a gate written `<= 0x20` would refuse a space inside a string, and one
-- written over `char::is_control` would refuse `0x7F`; each reddens one column here.
-- The `0x7F` columns are written as an equality against the same literal so the recorded
-- expectation holds no raw control byte: a mangled or dropped byte makes the comparison
-- false where 18.6's is true, and a refused input errors the fixture outright.
-- @test
SELECT (E'{"k": "a b"}')::jsonb::text AS space_inside_a_value,
       (E'{"a b": 1}')::jsonb::text AS space_inside_a_key,
       (E'{"k": "a\x7fb"}')::jsonb::text = E'{"k": "a\x7fb"}' AS del_char_survives,
       (E'{"a\x7fb": 1}')::jsonb::text = E'{"a\x7fb": 1}' AS del_char_key_survives,
       length((E'{"k": "a\x7fb"}')::jsonb ->> 'k') AS del_char_length;
