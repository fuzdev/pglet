-- The `ALL` polarity of the empty-list bound. It was already 42601 here, but by accident
-- — pglet did not know the keyword in this position at all — so the code that teaches it
-- `ALL` has to keep the refusal rather than start accepting it. 42601 on both.
-- @test
-- @expect-error: syntax error at or near ")"
SELECT upper(ALL);
