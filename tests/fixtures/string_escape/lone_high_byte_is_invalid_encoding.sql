-- A byte escape above `\x7F` that starts no legal UTF-8 sequence makes the finished
-- literal invalid in the server encoding, which is `22021` and not a value.
-- @test
-- @expect-error: a lone high byte starts no UTF-8 sequence
-- @expect-message
SELECT E'\xE9';
