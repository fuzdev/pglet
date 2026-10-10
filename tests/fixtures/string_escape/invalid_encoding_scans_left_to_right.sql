-- The finished literal is validated once, left to right, so the **first** bad byte is
-- the failure and the NUL behind it is merely part of the reported run. Raising the NUL
-- eagerly, where the escape is decoded, names `0x00` alone instead.
-- @test
-- @expect-error: the first bad byte wins, not the NUL behind it
-- @expect-message
SELECT E'\xE9\x00';
