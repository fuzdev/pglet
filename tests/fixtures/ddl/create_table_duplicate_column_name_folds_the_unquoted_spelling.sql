-- The other half of that pair: an unquoted `A` folds onto the quoted `"a"` beside it,
-- so the two are one name and the gate refuses — naming the **folded** spelling, which
-- is what the message has to carry for `@expect-message` to hold.
-- @expect-error: an unquoted A folds onto a quoted "a" in the same column list
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctdfd (A INT, "a" TEXT);
