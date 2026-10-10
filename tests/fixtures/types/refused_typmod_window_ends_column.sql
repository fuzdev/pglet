-- The scale-high end read straight out of the catalog: `information_schema` is where an
-- accepted modifier is readable with **no conversion at all**, so this guards the gate's
-- acceptance independently of whether any value survives the scale. (It does — a zero
-- does, which `types/refused_typmod_window_ends.sql` asserts on the value path.)
-- @setup
CREATE TABLE rtwe (a numeric(5,1000), b numeric(1000), c varchar(10485760));
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` where pglet sends
--   `text` (25); the modifier columns themselves are `information_schema.cardinal_number`
--   there against pglet's `int4`.
-- @test
SELECT column_name, numeric_precision, numeric_scale, character_maximum_length
FROM information_schema.columns WHERE table_name = 'rtwe' ORDER BY ordinal_position;
