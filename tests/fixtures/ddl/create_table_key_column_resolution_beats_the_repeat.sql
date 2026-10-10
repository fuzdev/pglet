-- The two verdicts inside one key list **interleave**, because the repeat test sits
-- below the key's own resolution in the same per-key loop body. So a key that resolves
-- to nothing, written between the two copies, wins — while
-- `create_table_key_repeat_beats_a_later_missing_key_column.sql` has the copies first
-- and answers the repeat. Neither arrangement follows from the other, and no whole-list
-- resolve-then-dedup (or its reverse) yields both.
-- @expect-error: the middle key resolves to no column
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctkr (a INT, PRIMARY KEY (a, nosuch, a));
