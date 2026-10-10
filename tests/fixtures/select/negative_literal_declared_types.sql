-- @setup
CREATE TABLE nldt (k INT);
INSERT INTO nldt VALUES (1);

-- Folding unary minus into the constant changes the constant's **declared type**, and in
-- one case it fixes a divergence: Postgres types an integer constant `int4` when it fits
-- and `int8` otherwise, and it decides that on the *signed* value. `-2147483648` fits
-- `int4` and 18.6 declares `integer`, where an unfolded `Negate(2147483648)` took its
-- type from the unsigned operand and declared `bigint` — the same eight bytes under an
-- `int4` client decode, invisible to a value comparison and caught only by the harness's
-- two declared-type legs.
--
-- The neighbours are the boundary's other side (`-2147483649` is `bigint` on both), the
-- decimal family (`numeric`), the doubly-folded spelling, and a `COALESCE` branch — where
-- the folded constant is *concrete* for target selection and the wrapper was not.
-- @test
SELECT -1 AS neg_one,
       -2147483648 AS int4_min,
       -2147483649 AS below_int4_min,
       -1.5 AS neg_decimal,
       - -1 AS double_negated,
       COALESCE(k, -1) AS coalesced
FROM nldt;
