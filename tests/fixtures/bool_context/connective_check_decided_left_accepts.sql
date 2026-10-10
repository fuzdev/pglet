-- A `CHECK` is evaluated by the same walk: the row `id = 1` satisfies the constraint by
-- its left operand, and the division is never evaluated.
-- @setup
CREATE TABLE sk (id INT, CHECK (id = 1 OR 1/(id-1) = 0));
INSERT INTO sk VALUES (1);
-- @test
SELECT id FROM sk;
