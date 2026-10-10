-- The `UNIQUE` arm of the same pair. Both arms share `42701`, so the message is what
-- separates them — 18.6 words this one *unique constraint*.
-- @expect-error: the repeated unique key column outranks the column default cook
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ukrdc (a INT DEFAULT 'zz', UNIQUE (a, a));
