-- A missing name anywhere in the list is `42P01`, and nothing is dropped. (The
-- "nothing is dropped" half is not visible here — the harness rolls the fixture back
-- either way — and is guarded by `crates/pglet/tests/drop_dependencies.rs`'s
-- `a_failed_multi_drop_leaves_every_table_in_place`.)
-- @setup
CREATE TABLE dmm (a INT);

-- @expect-error: table "dmm_nosuch" does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dmm, dmm_nosuch;
