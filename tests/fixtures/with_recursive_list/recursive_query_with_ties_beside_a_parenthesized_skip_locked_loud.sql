-- A `SKIP LOCKED` inside the parentheses merges with a `WITH TIES` after them, which the
-- grammar refuses.
-- @expect-error: SKIP LOCKED and WITH TIES options cannot be used together
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS ((SELECT n FROM r ORDER BY n FOR UPDATE SKIP LOCKED) FETCH FIRST 1 ROWS WITH TIES) SELECT * FROM r;
