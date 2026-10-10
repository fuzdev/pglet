-- The accept side of the declaration-only operand gate at a **math function**'s
-- argument. `abs(oid)` and `abs("char")` are `42883` on 18.6; every argument here is an
-- ordinary integer, which it answers.
--
-- The nest is the second subject: `abs` reads the type its argument derived, so
-- `abs(abs(abs(qty)))` has to agree with `abs(qty)` — and `round(x, n)`'s second
-- argument is gated too, through the whole walk rather than the threaded first.
-- @setup
CREATE TABLE oid_math (qty INT, price NUMERIC(10,2));
INSERT INTO oid_math VALUES (7, 1.55);

-- @test
SELECT abs(qty) + 1 AS abs_shifted,
       abs(abs(abs(qty))) AS abs_thrice,
       abs(-qty) AS abs_of_negate,
       round(price, 1) AS rounded,
       abs(round(price, 1)) AS abs_of_round
FROM oid_math
