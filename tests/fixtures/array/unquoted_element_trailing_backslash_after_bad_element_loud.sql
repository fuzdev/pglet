-- Each element is converted as it is read, so an earlier element's input error comes
-- before the refusal of the backslash that ends the body.
-- @test
-- @expect-error: the first element's odd hex digit count is 22023, ahead of the body's end
SELECT '{\\x616,a\}'::bytea[];
