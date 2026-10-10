-- The other arrangement: both copies precede the unresolvable key, so the repeat test
-- fires at the second `a` and the third key is never resolved. Read against
-- `create_table_key_column_resolution_beats_the_repeat.sql` — the pair is what pins the
-- interleave, and the SQLSTATEs differ (`42701` here, `42703` there), so each fixture
-- discriminates on its own.
-- @expect-error: the repeat is reached before the third key is resolved
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctkb (a INT, PRIMARY KEY (a, a, nosuch));
