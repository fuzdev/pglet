-- Two signs are not an infinity spelling.
-- @expect-error: invalid input syntax for type date
-- @expect-message
-- @test
SELECT '--infinity'::date;
