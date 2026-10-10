-- The reverse arrangement of
-- `select/set_op_order_by_bogus_key_before_a_bad_position_loud.sql`: the out-of-range
-- ordinal comes first in source order, so `42P10` is what 18.6 reports and what the
-- interleaved per-key check reports here.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: ORDER BY position 99 is not in select list (42P10)
-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY 99, bogus;
