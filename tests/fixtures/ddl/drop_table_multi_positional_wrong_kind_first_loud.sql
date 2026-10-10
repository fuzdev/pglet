-- The mirror of `drop_table_multi_positional_missing_first_loud.sql`: the same two
-- names in the other written order answer the other SQLSTATE (`42809`).
-- @setup
CREATE SEQUENCE dpw_seq;

-- @expect-error: "dpw_seq" is not a table
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dpw_seq, dpw_nosuch;
