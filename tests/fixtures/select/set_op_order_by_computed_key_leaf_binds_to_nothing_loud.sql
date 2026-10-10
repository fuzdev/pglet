-- A **computed** key's leaves are judged too, not only a key that *is* a column. 18.6
-- resolves the names in a set-op sort key before it raises its own
-- `0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY clause` for the expression, so a leaf
-- binding to nothing is `42703` there rather than `0A000`. Judging only the whole key left
-- every computed spelling unbound at plan time, and over two **empty** arms the eval-time
-- resolver — the only judge left — never runs, so this answered `(0 rows)`.
--
-- Emptiness identifies the site rather than the rule for this bare spelling, exactly as it
-- does for the whole-key twin (`select/set_op_order_by_key_binds_to_nothing_loud.sql`):
-- seed a row into each arm and it was already `42703` here, from the eval-time resolver.
-- The qualified spelling is row-count independent and has its own fixture,
-- `select/set_op_order_by_computed_key_qualified_leaf_loud.sql`.
-- @setup
CREATE TABLE sopca (a INT);
CREATE TABLE sopcb (a INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT a FROM sopca UNION SELECT a FROM sopcb ORDER BY bogus + 1;
