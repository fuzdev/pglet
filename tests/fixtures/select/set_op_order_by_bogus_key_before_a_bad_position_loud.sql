-- Error order inside the clause. 18.6 resolves a set-op's sort keys in source order, so
-- the first key's binding failure is what it reports even though a later key's ordinal is
-- out of range. The binding check therefore lives in the same per-key closure as the
-- positional range check rather than in a pass of its own — with the two split into
-- whole-clause passes, whichever ran first would win for every arrangement.
--
-- `select/set_op_order_by_bad_position_before_a_bogus_key_loud.sql` is the other
-- arrangement, and it answers the other code.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: column "bogus" does not exist, ahead of the bad ordinal (42703)
-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY bogus, 99;
