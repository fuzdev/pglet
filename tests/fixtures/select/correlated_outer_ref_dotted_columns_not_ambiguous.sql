-- A false rejection once shipped knowingly, now closed. The inner relation's two
-- columns are quoted `"a.c"` and `"b.c"`, and a suffix match over the join's flat names
-- called a bare `c` ambiguous — a bogus `42702` at the inner level, which 5a's
-- "an ambiguity is final where it is claimed" rule then made fatal. The inner scope's
-- pairs carry `a.c` and `b.c` as ordinary *column names*, so nothing claims a bare `c`
-- and the reference resolves outward, as 18.6 does.
--
-- Its no-outer control is `select/dotted_columns_bare_is_missing.sql` (`42703` on both
-- engines), and its inner-claimant polarity is
-- `select/correlated_outer_ref_dotted_columns_inner_claimant.sql`.
-- @setup
CREATE TABLE dcna_o (c INT);
CREATE TABLE dcna_i ("a.c" INT, "b.c" INT);
CREATE TABLE dcna_z (z INT);
INSERT INTO dcna_o VALUES (1), (2);
INSERT INTO dcna_i VALUES (10, 20);
INSERT INTO dcna_z VALUES (99);

-- @test
SELECT c FROM dcna_o WHERE EXISTS (SELECT 1 FROM dcna_i, dcna_z WHERE c = 1);
