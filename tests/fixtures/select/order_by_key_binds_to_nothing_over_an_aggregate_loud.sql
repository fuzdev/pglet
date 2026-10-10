-- The aggregate member of the sort-key binding exemption. An `ORDER BY` key a `GROUP BY`
-- query's output cannot answer rides the Aggregate as a Postgres **resjunk** column, and a
-- junk column is an expression evaluated only when a row is — so over an empty relation
-- the key was judged by nobody and this answered `(0 rows)` where 18.6 is `42703`.
--
-- Read it beside `select/order_by_key_binds_to_nothing_over_a_window_loud.sql`,
-- `select/order_by_key_binds_to_nothing_loud.sql` and
-- `select/order_by_key_binds_to_nothing_under_distinct_loud.sql`: four different plan
-- shapes reached the one blanket, so a fix aimed at any single node would have closed a
-- quarter of it.
-- @setup
CREATE TABLE sk_agg (a INT, v INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT count(*) FROM sk_agg GROUP BY a ORDER BY bogus;
