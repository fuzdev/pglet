-- The `DISTINCT ON` interleave's second half — the ordinal first, `42P10` on both engines,
-- green before the interleave rule and after. Read beside
-- `select/distinct_on_bogus_key_before_a_bad_position_loud.sql`.
-- @setup
CREATE TABLE sk_don2 (a INT, b INT);
INSERT INTO sk_don2 VALUES (1, 10);

-- @expect-error: DISTINCT ON position 9 is not in select list (42P10)
-- @test
SELECT DISTINCT ON (9, bogusdon) a FROM sk_don2;
