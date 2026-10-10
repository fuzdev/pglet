-- The **accept** polarity that bounds the fold, and it is a value fixture rather than an
-- error one because both engines run this statement. `::` outranks unary minus
-- (`TYPECAST` above `UMINUS` in `gram.y`'s precedence list), so `-99::int` is
-- `-(99::int)` — a cast node, not an `A_Const` — and `doNegate` leaves it an operator
-- node. It is therefore an ordinary constant sort key: every row compares equal and the
-- input order survives, exactly as `ORDER BY +1` does.
--
-- The seed is deliberately not already in `x` order, so a fold widened to reach through
-- the cast would read `-99` as an out-of-range ordinal and fail loud instead.
-- @setup
CREATE TABLE obnc (x INT, y INT);
INSERT INTO obnc VALUES (3, 1), (1, 2), (2, 3);
-- @test
SELECT x, y FROM obnc ORDER BY -99::int;
