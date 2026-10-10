-- The window member of the sort-key binding exemption — the same junk column one node
-- over (`Plan::Window`), and the same `(0 rows)` against 18.6's `42703` over an empty
-- relation. See `select/order_by_key_binds_to_nothing_over_an_aggregate_loud.sql`.
-- @setup
CREATE TABLE sk_win (a INT, v INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT row_number() OVER () AS w FROM sk_win ORDER BY bogus;
