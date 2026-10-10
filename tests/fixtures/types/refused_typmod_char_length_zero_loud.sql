-- **A type modifier is a value the *type* validates, and a modifier the type refuses is
-- a refusal — never a modifier dropped.** `char(0)` is below `anychar_typmodin`'s window
-- (`if (*tl < 1)`, `varchar.c`), so parse analysis raises `22023` where pglet used to
-- resolve an unconstrained type and answer `ab`.
-- @expect-error: a character length below 1
-- @expect-message
-- @test
SELECT 'ab'::char(0) AS never_reached;
