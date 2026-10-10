-- The second accept polarity: `-abs(99)` negates a **function call**, which is not one of
-- `doNegate`'s two constant kinds, so it stays an operator node on both engines and sorts
-- by a constant. Written beside the cast spelling because the two reach the non-fold by
-- different routes — precedence there, node kind here — and a fold that folded the whole
-- parsed operand rather than a bare literal would break this one and not that one.
-- @setup
CREATE TABLE obnfn (x INT, y INT);
INSERT INTO obnfn VALUES (3, 1), (1, 2), (2, 3);
-- @test
SELECT x, y FROM obnfn ORDER BY -abs(99);
