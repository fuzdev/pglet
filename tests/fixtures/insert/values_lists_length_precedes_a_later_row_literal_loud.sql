-- The rows after the refused one are never read: the third row's `'x'` would be `22P02`,
-- and the second row's length is refused first.
-- @setup
CREATE TABLE vlll (a INT, b INT);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601), ahead of the third row's literal
-- @expect-message
INSERT INTO vlll VALUES (1, 2), (2), (3, 'x');
