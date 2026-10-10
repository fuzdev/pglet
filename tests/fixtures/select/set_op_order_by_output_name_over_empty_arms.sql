-- The accept polarity at the row count that made the refusals silent. The gate runs at
-- plan time, so it judges this statement identically whether or not a row reaches the
-- Sort — `select/set_op_order_by_output_name_sorts.sql` is the same key over seeded arms.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY a;
