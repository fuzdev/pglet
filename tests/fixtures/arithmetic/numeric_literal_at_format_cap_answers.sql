-- The control beside the over-cap refusal: 131072 integer digits is the most the
-- numeric format's `int16` weight can address, and both engines answer there. One
-- digit more is `arithmetic/numeric_literal_over_format_cap_loud.sql`.
-- @test
SELECT length((1e131071)::text) AS digits;
