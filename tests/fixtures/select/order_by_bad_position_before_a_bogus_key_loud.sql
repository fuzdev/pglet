-- The interleave's second half: the same two faults with their positions swapped, where the
-- out-of-range ordinal is first and both engines say `42P10`. Green before the interleave rule as
-- well as after — it is the half that pins *which* of the two orders is Postgres's, and is
-- only worth anything read beside
-- `select/order_by_bogus_key_before_a_bad_position_loud.sql`.
-- @setup
CREATE TABLE sk_pos2 (a INT, b INT);
INSERT INTO sk_pos2 VALUES (1, 10);

-- @expect-error: ORDER BY position 9 is not in select list (42P10)
-- @test
SELECT a FROM sk_pos2 ORDER BY 9, bogus;
