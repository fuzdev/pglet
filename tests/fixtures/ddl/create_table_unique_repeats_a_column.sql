-- The `UNIQUE` spelling of the same rule, which 18.6 words differently (*unique
-- constraint*, not *primary key constraint*) on the same SQLSTATE — so the message is
-- what separates the two arms, and `@expect-message` is what asserts it.
-- @expect-error: a repeated column in a unique constraint
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctur (a INT, UNIQUE (a, a));
