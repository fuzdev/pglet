-- The window's other end, and a different sentence: `MaxAttrSize` is 10 485 760
-- characters (`htup_details.h`), and one above it is refused rather than silently
-- uncapped. The *inclusive* end is accepted — `types/refused_typmod_window_ends.sql`.
-- @expect-error: a character length above the 10 MB cap
-- @expect-message
-- @test
SELECT 'ab'::char(10485761) AS never_reached;
