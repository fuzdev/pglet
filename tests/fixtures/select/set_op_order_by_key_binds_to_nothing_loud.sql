-- The **read** polarity of the set-op trailing-`ORDER BY` binding hole. Those keys were
-- resolved with no `ColumnScope` at all (`planner/query.rs`'s `plan_set_op_order_limit`
-- ran the positional range check and nothing else), so a name binding to nothing survived
-- plan time; the only judge left was the eval-time resolver, which a set op over two
-- **empty** relations never reaches. So this answered `(0 rows)` where 18.6 refuses in
-- parse analysis.
--
-- Emptiness is what identifies the site rather than the rule for this bare spelling: seed
-- a row into each arm and it was already `42703` here, from the eval-time resolver. The
-- qualified spelling has no such control — it wrote at either row count, and its fixture
-- is `select/set_op_order_by_qualified_key_binds_to_nothing_loud.sql`.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY bogus;
