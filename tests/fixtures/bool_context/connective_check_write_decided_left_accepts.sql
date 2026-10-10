-- The write itself: a `CHECK` whose left operand decides it admits the row.
-- @setup
CREATE TABLE sk (id INT, CHECK (id = 1 OR 1/(id-1) = 0));
-- @test
INSERT INTO sk VALUES (1) RETURNING id;
